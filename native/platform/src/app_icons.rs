//! Read-only Windows Shell application icon rasters, without Tauri or a browser host.
//!
//! This is an internal platform API, not a C ABI: the caller must pass an [`Application`] obtained
//! from [`crate::apps::scan`] (acceptance fixtures construct the same shape explicitly). The core
//! stage that exposes icons must therefore enforce authoritative catalog membership itself; this
//! module only proves the identity is one of the two catalog shapes it can resolve.
//!
//! Stage contract:
//!
//! - [`read`] is synchronous and must run off the UI dispatcher. It never launches an application,
//!   reads the clipboard, enumerates catalog entries, loads thumbnails or documents, downloads
//!   assets, writes files, caches pixels or starts threads.
//! - Desktop rows are only read when the handler is an existing, drive-absolute local `.exe` that
//!   `GetBinaryTypeW` accepts as a native executable. Opaque desktop Shell IDs are never treated as
//!   launch paths, and UNC/relative/URI/script targets never reach a generic file-icon lookup.
//! - Packaged rows are only read through the per-user `shell:AppsFolder\<AUMID>` namespace.
//! - The requested edge is bounded to `1..=`[`MAX_EDGE`]; the returned raster is bounded to
//!   [`MAX_BYTES`] with checked width/height/byte arithmetic before any buffer is allocated. The
//!   actual Shell bitmap is measured and bounded before pixel extraction.
//! - Cancellation is cooperative between non-abortable Shell/COM/GDI phases. It is checked before
//!   and after every blocking phase, and an in-flight call cannot be interrupted; it completes or
//!   fails and its scoped resources are released either way.
//! - Caller-owned COM apartments are preserved: an existing STA/MTA (`RPC_E_CHANGED_MODE`) is
//!   reused without a matching `CoUninitialize`, while our own initialization is balanced.
//! - Every HBITMAP/DC and COM reference is scoped and released on success, error and cancellation.

use std::sync::atomic::{AtomicBool, Ordering};
use windows::core::{Interface, PCWSTR};
use windows::Win32::Foundation::{RPC_E_CHANGED_MODE, SIZE};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ,
};
use windows::Win32::Storage::FileSystem::GetBinaryTypeW;
use windows::Win32::System::Com::{
    CoInitializeEx, CoUninitialize, IBindCtx, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
};
use windows::Win32::System::WindowsProgramming::{SCS_32BIT_BINARY, SCS_64BIT_BINARY};
use windows::Win32::UI::Shell::{
    IShellItem, IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_ICONONLY,
};

use crate::apps::{Application, Kind, Unavailable};

/// Largest accepted request edge and largest returned raster edge, in physical pixels.
pub const MAX_EDGE: u32 = 128;
/// Largest returned raster in bytes; a 128 x 128 RGBA raster is exactly this bound.
pub const MAX_BYTES: usize = 64 * 1024;
/// Identity strings are bounded before any Shell call or buffer build.
const MAX_IDENTITY: usize = 4096;
/// The packaged identity limit mirrors the catalog's strict `Family!App` shape.
const MAX_PACKAGED: usize = 255;

/// A bounded, top-down RGBA8 application icon raster with straight (unpremultiplied) alpha.
#[derive(Clone, Eq, PartialEq)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    /// Exactly `width * height * 4` bytes in row-major RGBA order.
    pub rgba: Vec<u8>,
}

impl std::fmt::Debug for Raster {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Raster")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bytes", &self.rgba.len())
            .finish()
    }
}

/// Why an icon raster was not produced. No variant ever stands for a fabricated placeholder.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// The caller's cancellation flag was observed. A blocking call already in progress completes.
    Cancelled,
    /// The requested edge was zero or above [`MAX_EDGE`].
    InvalidRequest,
    /// The catalog marked this row unsupported; the recorded reason is preserved.
    Unavailable(Unavailable),
    /// The handler/source identity is not a validated local `.exe` path or a strict packaged ID.
    InvalidIdentity,
    /// The Shell returned a bitmap outside the request edge / [`MAX_BYTES`] contract.
    OutOfBounds,
    /// A Windows phase failed. `code` is the failing HRESULT, or 0 for zero/partial returns.
    Windows { phase: &'static str, code: i32 },
}

impl Error {
    fn windows(phase: &'static str, error: &windows::core::Error) -> Self {
        Self::Windows {
            phase,
            code: error.code().0,
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("application icon read cancelled"),
            Self::InvalidRequest => f.write_str("application icon edge must be 1..=MAX_EDGE"),
            Self::Unavailable(reason) => write!(f, "catalog entry is unavailable: {reason:?}"),
            Self::InvalidIdentity => f.write_str(
                "catalog identity is not a validated local executable or packaged application",
            ),
            Self::OutOfBounds => f.write_str("icon bitmap exceeds the bounded raster contract"),
            Self::Windows { phase, code } => {
                write!(f, "Windows phase {phase} failed (0x{code:08X})")
            }
        }
    }
}

impl std::error::Error for Error {}

/// Read one square icon raster for a trusted catalog application at the requested edge.
///
/// `application` must come from the shared catalog scan; `edge` is the requested square side in
/// physical pixels (`1..=`[`MAX_EDGE`]). Call this off the dispatcher: the Shell/COM/GDI work is
/// synchronous and only cooperatively cancellable.
pub fn read(application: &Application, edge: u32, cancel: &AtomicBool) -> Result<Raster, Error> {
    if !(1..=MAX_EDGE).contains(&edge) {
        return Err(Error::InvalidRequest);
    }
    check(cancel)?;
    if let Some(reason) = application.unavailable {
        return Err(Error::Unavailable(reason));
    }
    let identity = match (application.kind, application.handler.as_deref()) {
        (Kind::Unavailable, _) => return Err(Error::Unavailable(Unavailable::MissingTarget)),
        (Kind::Desktop, Some(path)) if local_executable(path) => Identity::File(path),
        (Kind::Packaged, Some(id)) if packaged(id) => Identity::AppsFolder(id),
        _ => return Err(Error::InvalidIdentity),
    };
    // The parse string is bounded with the validated identity before any COM call.
    let mut target: Vec<u16> = match identity {
        Identity::File(path) => path.encode_utf16().collect(),
        Identity::AppsFolder(id) => "shell:AppsFolder\\"
            .encode_utf16()
            .chain(id.encode_utf16())
            .collect(),
    };
    target.push(0);
    check(cancel)?;
    let _apartment = Apartment::enter()?;
    check(cancel)?;
    let item: IShellItem =
        unsafe { SHCreateItemFromParsingName(PCWSTR(target.as_ptr()), None::<&IBindCtx>) }
            .map_err(|error| Error::windows("icons_item", &error))?;
    check(cancel)?;
    let factory: IShellItemImageFactory = item
        .cast()
        .map_err(|error| Error::windows("icons_factory", &error))?;
    check(cancel)?;
    // ICONONLY keeps this an application icon lookup: no thumbnail, no document preview.
    let bitmap = unsafe {
        factory.GetImage(
            SIZE {
                cx: edge as i32,
                cy: edge as i32,
            },
            SIIGBF_ICONONLY,
        )
    }
    .map_err(|error| Error::windows("icons_image", &error))?;
    let bitmap = Bitmap(bitmap);
    check(cancel)?;
    let raster = read_pixels(bitmap.0, edge, cancel);
    check(cancel)?;
    raster
}

enum Identity<'a> {
    /// A validated drive-absolute local executable resumed as a file-system Shell item.
    File(&'a str),
    /// A strict packaged identity resumed through the per-user AppsFolder namespace.
    AppsFolder(&'a str),
}

fn check(cancel: &AtomicBool) -> Result<(), Error> {
    if cancel.load(Ordering::Acquire) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

/// Mirror of the catalog's strict `Family!App` identity check, repeated before a Shell call.
fn packaged(value: &str) -> bool {
    value.len() <= MAX_PACKAGED
        && !value.chars().any(char::is_control)
        && value.split_once('!').is_some_and(|(family, app)| {
            !family.is_empty()
                && !app.is_empty()
                && !app.contains('!')
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_.!-".contains(&byte))
        })
}

/// Desktop handlers must be an existing, drive-absolute local native `.exe`.
///
/// This rejects UNC, device, relative, URI and shell-namespace strings before any Shell lookup, so
/// an opaque desktop ID or a script can never be resolved into a generic file icon.
fn local_executable(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_IDENTITY || value.chars().any(char::is_control) {
        return false;
    }
    let bytes = value.as_bytes();
    if bytes.len() < 4 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' || bytes[2] != b'\\' {
        return false;
    }
    // Reject alternate data streams and device qualifiers after the drive prefix.
    if value[3..].contains(':') || !value.to_ascii_lowercase().ends_with(".exe") {
        return false;
    }
    if !std::path::Path::new(value).is_file() {
        return false;
    }
    let wide: Vec<u16> = value.encode_utf16().chain([0]).collect();
    let mut kind = 0;
    unsafe { GetBinaryTypeW(PCWSTR(wide.as_ptr()), &mut kind) }.is_ok()
        && matches!(kind, SCS_32BIT_BINARY | SCS_64BIT_BINARY)
}

/// Checked raster byte length; rejects empty, oversized and overflowing dimensions.
fn raster_len(width: u32, height: u32) -> Result<usize, Error> {
    if width == 0 || height == 0 || width > MAX_EDGE || height > MAX_EDGE {
        return Err(Error::OutOfBounds);
    }
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .ok_or(Error::OutOfBounds)?;
    let bytes = pixels.checked_mul(4).ok_or(Error::OutOfBounds)?;
    if bytes > MAX_BYTES {
        return Err(Error::OutOfBounds);
    }
    Ok(bytes)
}

/// Convert a bottom-up BGRA buffer with premultiplied alpha into top-down straight RGBA.
///
/// `GetDIBits` returns rows bottom-up for a positive `biHeight`; this flips them and removes the
/// premultiplied alpha the Shell image factory produces.
fn bgra_bottom_up_to_rgba(bgra: &[u8], width: u32, height: u32) -> Result<Vec<u8>, Error> {
    let bytes = raster_len(width, height)?;
    if bgra.len() != bytes {
        return Err(Error::OutOfBounds);
    }
    let row = width as usize * 4;
    let rows = height as usize;
    let mut rgba = vec![0u8; bytes];
    for y in 0..rows {
        let source = &bgra[(rows - 1 - y) * row..][..row];
        let destination = &mut rgba[y * row..][..row];
        for (source, destination) in source.chunks_exact(4).zip(destination.chunks_exact_mut(4)) {
            let alpha = source[3];
            destination[0] = unpremultiply(source[2], alpha);
            destination[1] = unpremultiply(source[1], alpha);
            destination[2] = unpremultiply(source[0], alpha);
            destination[3] = alpha;
        }
    }
    Ok(rgba)
}

fn unpremultiply(channel: u8, alpha: u8) -> u8 {
    if alpha == 0 {
        return 0;
    }
    ((channel as u32 * 255 + alpha as u32 / 2) / alpha as u32).min(255) as u8
}

/// Measure the Shell bitmap, bound it, then copy its pixels into a checked RGBA raster.
fn read_pixels(bitmap: HBITMAP, edge: u32, cancel: &AtomicBool) -> Result<Raster, Error> {
    let mut measured = BITMAP::default();
    let read = unsafe {
        GetObjectW(
            HGDIOBJ(bitmap.0),
            std::mem::size_of::<BITMAP>() as i32,
            Some((&mut measured as *mut BITMAP).cast()),
        )
    };
    if read == 0 {
        return Err(Error::Windows {
            phase: "icons_bitmap",
            code: 0,
        });
    }
    if measured.bmWidth <= 0 || measured.bmHeight <= 0 {
        return Err(Error::OutOfBounds);
    }
    let (width, height) = (measured.bmWidth as u32, measured.bmHeight as u32);
    // Bound the actual bitmap, not only the request, before allocating anything for its pixels.
    if width > edge || height > edge {
        return Err(Error::OutOfBounds);
    }
    let bytes = raster_len(width, height)?;
    check(cancel)?;
    let dc = Dc::create()?;
    let mut info = BITMAPINFO::default();
    info.bmiHeader = BITMAPINFOHEADER {
        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: width as i32,
        // Positive height: rows arrive bottom-up and the pure conversion flips them.
        biHeight: height as i32,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        ..Default::default()
    };
    let mut bgra = vec![0u8; bytes];
    // The bitmap is intentionally not selected into the DC: GetDIBits requires that.
    let copied = unsafe {
        GetDIBits(
            dc.0,
            bitmap,
            0,
            height,
            Some(bgra.as_mut_ptr().cast()),
            &mut info,
            DIB_RGB_COLORS,
        )
    };
    if copied <= 0 {
        return Err(Error::Windows {
            phase: "icons_pixels",
            code: 0,
        });
    }
    if copied as u32 != height {
        return Err(Error::Windows {
            phase: "icons_pixels_partial",
            code: copied,
        });
    }
    check(cancel)?;
    let rgba = bgra_bottom_up_to_rgba(&bgra, width, height)?;
    Ok(Raster {
        width,
        height,
        rgba,
    })
}

/// Scoped COM apartment: reuses a caller-owned STA/MTA without unbalancing it.
struct Apartment(bool);

impl Apartment {
    fn enter() -> Result<Self, Error> {
        let result =
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        if result.is_ok() {
            Ok(Self(true))
        } else if result == RPC_E_CHANGED_MODE {
            Ok(Self(false))
        } else {
            Err(Error::windows(
                "icons_com",
                &windows::core::Error::from_hresult(result),
            ))
        }
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

/// Scoped memory device context used only to identify the destination pixel format.
struct Dc(HDC);

impl Dc {
    fn create() -> Result<Self, Error> {
        let dc = unsafe { CreateCompatibleDC(None) };
        if dc.0.is_null() {
            Err(Error::Windows {
                phase: "icons_dc",
                code: 0,
            })
        } else {
            Ok(Self(dc))
        }
    }
}

impl Drop for Dc {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteDC(self.0);
        }
    }
}

/// Scoped ownership of the Shell-returned bitmap.
struct Bitmap(HBITMAP);

impl Drop for Bitmap {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.0 .0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desktop(handler: Option<&str>) -> Application {
        Application {
            name: "Editor".into(),
            source_id: "Opaque.Editor".into(),
            handler: handler.map(Into::into),
            kind: Kind::Desktop,
            unavailable: None,
        }
    }

    fn packaged_app(handler: &str) -> Application {
        Application {
            name: "Package".into(),
            source_id: handler.into(),
            handler: Some(handler.into()),
            kind: Kind::Packaged,
            unavailable: None,
        }
    }

    fn unsupported(reason: Unavailable) -> Application {
        Application {
            name: "Missing".into(),
            source_id: "Opaque.Missing".into(),
            handler: None,
            kind: Kind::Unavailable,
            unavailable: Some(reason),
        }
    }

    #[test]
    fn requested_edge_is_validated_before_any_windows_work() {
        let cancel = AtomicBool::new(false);
        let app = desktop(Some(r"C:\Windows\System32\notepad.exe"));
        for edge in [0, MAX_EDGE + 1, u32::MAX] {
            assert_eq!(
                read(&app, edge, &cancel).unwrap_err(),
                Error::InvalidRequest
            );
        }
    }

    #[test]
    fn unavailable_and_untrusted_identities_never_guess_a_generic_icon() {
        let cancel = AtomicBool::new(false);
        for reason in [
            Unavailable::MissingTarget,
            Unavailable::InvalidIdentity,
            Unavailable::NotNativeExecutable,
        ] {
            assert_eq!(
                read(&unsupported(reason), 32, &cancel).unwrap_err(),
                Error::Unavailable(reason)
            );
        }
        // Opaque desktop Shell IDs, UNC, relative, script, stream and namespace strings are refused.
        for handler in [
            Some("Opaque.Editor"),
            Some(r"\\server\share\app.exe"),
            Some(r"C:\app.cmd"),
            Some("app.exe"),
            Some(r"C:\definitely missing editor.exe"),
            Some(r"C:\app.exe:stream"),
            Some("C:\\app.exe\n"),
            Some(r"shell:AppsFolder\Family!App"),
            None,
        ] {
            assert_eq!(
                read(&desktop(handler), 32, &cancel).unwrap_err(),
                Error::InvalidIdentity,
                "{handler:?}"
            );
        }
        let mut marked = desktop(None);
        marked.kind = Kind::Unavailable;
        assert_eq!(
            read(&marked, 32, &cancel).unwrap_err(),
            Error::Unavailable(Unavailable::MissingTarget)
        );
        // A packaged identity is never accepted as a desktop path and vice versa.
        assert_eq!(
            read(
                &packaged_app(r"C:\Windows\System32\notepad.exe"),
                32,
                &cancel
            )
            .unwrap_err(),
            Error::InvalidIdentity
        );
        for id in ["Family!", "!App", "A!B!C", "A!B /c", "A!B\n", "中文!App"] {
            assert_eq!(
                read(&packaged_app(id), 32, &cancel).unwrap_err(),
                Error::InvalidIdentity,
                "{id}"
            );
        }
        let oversized = format!("{}!App", "x".repeat(MAX_PACKAGED));
        assert_eq!(
            read(&packaged_app(&oversized), 32, &cancel).unwrap_err(),
            Error::InvalidIdentity
        );
    }

    #[test]
    fn pre_cancelled_reads_never_touch_the_shell() {
        let cancel = AtomicBool::new(true);
        let exe = std::env::current_exe().unwrap();
        assert_eq!(
            read(&desktop(Some(&*exe.to_string_lossy())), 32, &cancel).unwrap_err(),
            Error::Cancelled
        );
        assert_eq!(
            read(&packaged_app("Family_1!App"), 32, &cancel).unwrap_err(),
            Error::Cancelled
        );
    }

    #[test]
    fn raster_byte_math_is_checked_and_bounded() {
        assert_eq!(raster_len(128, 128), Ok(MAX_BYTES));
        assert_eq!(raster_len(129, 129), Err(Error::OutOfBounds));
        assert_eq!(raster_len(1, 1), Ok(4));
        for (width, height) in [
            (0, 1),
            (1, 0),
            (129, 1),
            (1, 129),
            (65536, 4),
            (u32::MAX, u32::MAX),
        ] {
            assert_eq!(
                raster_len(width, height),
                Err(Error::OutOfBounds),
                "{width}x{height}"
            );
        }
        assert_eq!(bgra_bottom_up_to_rgba(&[], 1, 1), Err(Error::OutOfBounds));
        assert_eq!(
            bgra_bottom_up_to_rgba(&[0; 8], 3, 1),
            Err(Error::OutOfBounds)
        );
        assert_eq!(
            bgra_bottom_up_to_rgba(&[0; 6], 1, 1),
            Err(Error::OutOfBounds)
        );
    }

    #[test]
    fn bottom_up_bgra_rows_are_flipped_and_premultiplied_alpha_is_removed() {
        let bgra = [
            // Bottom row: opaque then fully transparent.
            1, 2, 3, 255, 9, 9, 9, 0, // Top row: half-alpha premultiplied, then opaque white.
            0, 128, 64, 128, 255, 255, 255, 255,
        ];
        let rgba = bgra_bottom_up_to_rgba(&bgra, 2, 2).unwrap();
        assert_eq!(rgba.len(), 16);
        assert_eq!(&rgba[0..8], &[128, 255, 0, 128, 255, 255, 255, 255]);
        assert_eq!(&rgba[8..12], &[3, 2, 1, 255]);
        assert_eq!(&rgba[12..16], &[0, 0, 0, 0]);
    }

    #[test]
    fn premultiplied_channels_clamp_after_division() {
        assert_eq!(
            bgra_bottom_up_to_rgba(&[255, 255, 255, 1], 1, 1).unwrap(),
            vec![255, 255, 255, 1]
        );
        assert_eq!(
            bgra_bottom_up_to_rgba(&[17, 17, 17, 0], 1, 1).unwrap(),
            vec![0, 0, 0, 0]
        );
    }

    #[test]
    fn local_executable_requires_an_existing_drive_absolute_native_executable() {
        let exe = std::env::current_exe().unwrap();
        assert!(local_executable(&exe.to_string_lossy()));
        for value in [
            "",
            "notepad.exe",
            r"C:\definitely missing editor.exe",
            r"C:\script.cmd",
            r"C:\file.txt",
            r"\\server\share\app.exe",
            r"\\?\C:\app.exe",
            "https://example.invalid/app.exe",
            r"shell:AppsFolder\Family!App",
            r"C:\app.exe:stream",
            "C:\\app.exe\n",
        ] {
            assert!(!local_executable(value), "{value}");
        }
        let long = format!(r"C:\{}", "a".repeat(MAX_IDENTITY));
        assert!(!local_executable(&long));
    }

    #[test]
    fn a_non_pe_file_that_merely_ends_in_exe_is_refused() {
        let fixture = std::env::temp_dir().join(format!(
            "tiez-icon-identity-fixture-{}.exe",
            std::process::id()
        ));
        std::fs::write(&fixture, b"not a portable executable").unwrap();
        let result = local_executable(&fixture.to_string_lossy());
        let _ = std::fs::remove_file(&fixture);
        assert!(
            !result,
            "A script disguised with an .exe suffix was accepted"
        );
    }

    #[test]
    fn packaged_identity_mirrors_catalog_strictness() {
        for id in [
            "Family_123!App",
            "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App",
            "A.1-b_c!Microsoft.Editor-1",
        ] {
            assert!(packaged(id), "{id}");
        }
        for id in [
            "",
            "!",
            "Family!",
            "!App",
            "A!B!C",
            "A!B /c",
            "A!B\n",
            "中文!App",
        ] {
            assert!(!packaged(id), "{id}");
        }
        assert!(!packaged(&format!("{}!App", "x".repeat(MAX_PACKAGED))));
    }
}
