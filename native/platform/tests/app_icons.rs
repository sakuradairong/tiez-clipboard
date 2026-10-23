//! Explicit Windows acceptance for the native application icon backend.
//!
//! These checks read real Shell icons only. They never launch an application, touch the clipboard,
//! write production files or load a browser host. The non-ignored test uses an installed Windows
//! executable, not user data; the ignored test reads the current user's AppsFolder catalog because
//! packaged identities are per-user.
#![cfg(windows)]
use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use tiez_native_platform::app_icons::{self, Error, Raster};
use tiez_native_platform::apps::{self, Application, Kind, Unavailable};
use windows::core::w;
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::System::Com::{
    CoInitializeEx, CoUninitialize, COINIT, COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetGuiResources, GetProcessHandleCount, GR_GDIOBJECTS,
};

fn handles() -> u32 {
    let mut count = 0;
    unsafe { GetProcessHandleCount(GetCurrentProcess(), &mut count) }.unwrap();
    count
}

fn gdi_objects() -> u32 {
    unsafe { GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS) }
}

fn desktop(handler: &str) -> Application {
    Application {
        name: "Installed editor".into(),
        // A catalog desktop row keeps an opaque Shell ID; only the handler is a real path.
        source_id: "Opaque.Desktop.Editor".into(),
        handler: Some(handler.into()),
        kind: Kind::Desktop,
        unavailable: None,
    }
}

fn packaged(identity: &str) -> Application {
    Application {
        name: "Packaged application".into(),
        source_id: identity.into(),
        handler: Some(identity.into()),
        kind: Kind::Packaged,
        unavailable: None,
    }
}

/// A real installed desktop executable, without consulting the user's application catalog.
fn installed_executable() -> String {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    for relative in [
        "System32\\notepad.exe",
        "explorer.exe",
        "System32\\cmd.exe",
        "System32\\mspaint.exe",
    ] {
        let candidate = format!("{root}\\{relative}");
        if Path::new(&candidate).is_file() {
            return candidate;
        }
    }
    panic!("this host has no local Windows executable for real icon acceptance");
}

fn validate(raster: &Raster, edge: u32) {
    assert!(raster.width > 0 && raster.height > 0, "empty raster");
    assert!(
        raster.width <= edge && raster.height <= edge,
        "{}x{} exceeds the requested {edge} edge",
        raster.width,
        raster.height
    );
    assert_eq!(
        raster.rgba.len(),
        (raster.width * raster.height * 4) as usize,
        "raster byte length does not match its dimensions"
    );
    assert!(raster.rgba.len() <= app_icons::MAX_BYTES);
    let opaque = raster
        .rgba
        .chunks_exact(4)
        .filter(|pixel| pixel[3] == 255)
        .count();
    assert!(opaque > 0, "real icon raster has no opaque pixel");
    assert!(
        raster.rgba.chunks_exact(4).any(|pixel| pixel[3] != 0),
        "real icon raster is fully transparent"
    );
    let distinct: HashSet<(u8, u8, u8, u8)> = raster
        .rgba
        .chunks_exact(4)
        .map(|pixel| (pixel[0], pixel[1], pixel[2], pixel[3]))
        .collect();
    assert!(
        distinct.len() > 1,
        "real icon raster is a flat placeholder rather than actual artwork"
    );
}

fn desktop_run(mode: COINIT, opposite: COINIT) {
    let cancel = AtomicBool::new(false);
    // A caller-owned apartment must survive every nested icon read unchanged.
    assert_eq!(
        unsafe { CoInitializeEx(None, mode) }.0,
        0,
        "caller initialization failed for mode {}",
        mode.0
    );
    let executable = installed_executable();
    let app = desktop(&executable);
    let real = app_icons::read(&app, 32, &cancel).expect("Real Shell desktop icon read failed");
    validate(&real, 32);
    println!(
        "desktop executable={} raster={}x{} bytes={} mode={}",
        executable,
        real.width,
        real.height,
        real.rgba.len(),
        mode.0
    );
    for edge in [16, 24, 32, 48, 128] {
        let raster = app_icons::read(&app, edge, &cancel).unwrap_or_else(|error| {
            panic!("Real Shell desktop icon read at {edge} failed: {error}")
        });
        validate(&raster, edge);
        println!(
            "edge={edge} raster={}x{} bytes={}",
            raster.width,
            raster.height,
            raster.rgba.len()
        );
    }
    // Opaque desktop IDs and unavailable catalog rows are refused, never resolved as file icons.
    assert_eq!(
        app_icons::read(&desktop(&app.source_id), 32, &cancel).unwrap_err(),
        Error::InvalidIdentity,
        "An opaque desktop Shell ID was accepted as a launch path"
    );
    let mut unavailable = app.clone();
    unavailable.kind = Kind::Unavailable;
    unavailable.handler = None;
    unavailable.unavailable = Some(Unavailable::NotNativeExecutable);
    assert_eq!(
        app_icons::read(&unavailable, 32, &cancel).unwrap_err(),
        Error::Unavailable(Unavailable::NotNativeExecutable)
    );
    assert_eq!(
        unsafe { CoInitializeEx(None, opposite) },
        RPC_E_CHANGED_MODE,
        "Nested icon reads changed the caller apartment"
    );
    // Warm the Shell icon cache and fixed initialization before observing repeated reads.
    app_icons::read(&app, 32, &cancel).unwrap();
    app_icons::read(&app, 32, &cancel).unwrap();
    let handles_before = handles();
    let gdi_before = gdi_objects();
    let mut handle_samples = Vec::new();
    let mut gdi_samples = Vec::new();
    for _ in 0..12 {
        let raster = app_icons::read(&app, 32, &cancel).unwrap();
        validate(&raster, 32);
        handle_samples.push(handles());
        gdi_samples.push(gdi_objects());
    }
    // The backend has no cache or thread: after warmup its scoped GDI objects must not accumulate.
    assert_eq!(
        gdi_samples.last().copied().unwrap(),
        gdi_before,
        "Repeated icon reads retained GDI objects: {gdi_samples:?} (before {gdi_before})"
    );
    assert!(
        handle_samples.last().copied().unwrap() <= handles_before + 2,
        "Repeated icon reads retained process handles: {handle_samples:?} (before {handles_before})"
    );
    // A cancellation observed after a completed read is reported without retaining resources.
    cancel.store(true, Ordering::Release);
    assert_eq!(
        app_icons::read(&app, 32, &cancel).unwrap_err(),
        Error::Cancelled
    );
    assert_eq!(
        gdi_objects(),
        gdi_before,
        "Cancelled read retained GDI objects"
    );
    assert!(
        handles() <= handles_before + 2,
        "Cancelled read retained handles"
    );
    cancel.store(false, Ordering::Release);
    app_icons::read(&app, 32, &cancel).expect("Icon read after cancellation failed");
    unsafe { CoUninitialize() };
    assert_eq!(
        unsafe { CoInitializeEx(None, opposite) }.0,
        0,
        "Unbalanced nested COM initialization"
    );
    unsafe { CoUninitialize() };
    assert!(unsafe { GetModuleHandleW(w!("WebView2Loader.dll")) }.is_err());
    assert!(unsafe { GetModuleHandleW(w!("Microsoft.Web.WebView2.Core.dll")) }.is_err());
    println!(
        "mode={} handles_before={} handles={handle_samples:?} gdi_before={gdi_before} gdi={gdi_samples:?}",
        mode.0,
        handles_before
    );
}

#[test]
fn real_desktop_application_icon_across_caller_apartments() {
    std::thread::spawn(|| desktop_run(COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED))
        .join()
        .expect("STA icon acceptance failed");
    std::thread::spawn(|| desktop_run(COINIT_MULTITHREADED, COINIT_APARTMENTTHREADED))
        .join()
        .expect("MTA icon acceptance failed");
}

#[test]
#[ignore = "requires an interactive Windows Shell and reads the current user's installed app metadata"]
fn real_packaged_application_icon_through_the_appsfolder() {
    std::thread::spawn(|| {
        let cancel = AtomicBool::new(false);
        assert_eq!(unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.0, 0);
        let catalog = apps::scan(&cancel).expect("Real Shell catalog scan failed");
        let packaged_rows: Vec<&Application> = catalog
            .applications
            .iter()
            .filter(|app| app.kind == Kind::Packaged)
            .collect();
        let desktops: Vec<&Application> = catalog
            .applications
            .iter()
            .filter(|app| app.kind == Kind::Desktop)
            .collect();
        // A real catalog desktop row must also resolve through this backend.
        let mut desktop_ok = 0;
        for app in desktops.iter().take(4) {
            match app_icons::read(app, 32, &cancel) {
                Ok(raster) => {
                    validate(&raster, 32);
                    desktop_ok += 1;
                    println!(
                        "catalog desktop name={:?} raster={}x{}",
                        app.name, raster.width, raster.height
                    );
                }
                Err(error) => println!("catalog desktop name={:?} error={error}", app.name),
            }
        }
        assert!(
            desktop_ok > 0,
            "No installed catalog desktop application icon could be read"
        );
        if packaged_rows.is_empty() {
            println!(
                "HOST GAP: the current user's AppsFolder exposes no packaged application; no packaged icon success is claimed"
            );
            unsafe { CoUninitialize() };
            return;
        }
        let mut packaged_ok = 0;
        let mut failures = Vec::new();
        for app in packaged_rows.iter().take(6) {
            match app_icons::read(app, 32, &cancel) {
                Ok(raster) => {
                    validate(&raster, 32);
                    packaged_ok += 1;
                    println!(
                        "packaged name={:?} identity={:?} raster={}x{} alpha_opaque_pixels={}",
                        app.name,
                        app.source_id,
                        raster.width,
                        raster.height,
                        raster.rgba.chunks_exact(4).filter(|p| p[3] == 255).count()
                    );
                }
                Err(error) => {
                    println!(
                        "packaged name={:?} identity={:?} error={error}",
                        app.name, app.source_id
                    );
                    failures.push((app.source_id.clone(), error));
                }
            }
        }
        println!(
            "host catalog scanned={} packaged={} desktop={} packaged_icon_ok={packaged_ok} packaged_icon_failed={}",
            catalog.scanned,
            packaged_rows.len(),
            desktops.len(),
            failures.len()
        );
        assert!(
            packaged_ok > 0,
            "No packaged application icon could be read from the real AppsFolder: {failures:?}"
        );
        // A strict packaged identity that does not exist must fail instead of yielding a placeholder.
        assert!(app_icons::read(&packaged("No.Such_Family!App"), 32, &cancel).is_err());
        unsafe { CoUninitialize() };
    })
    .join()
    .expect("Packaged icon acceptance failed");
}
