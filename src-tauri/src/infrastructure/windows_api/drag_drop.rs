use std::{
    cell::{RefCell, UnsafeCell},
    ffi::{c_void, OsString},
    os::windows::ffi::OsStringExt,
    path::PathBuf,
    ptr,
    rc::Rc,
};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;
use windows_core::Ref as WRef;

use windows::{
    core::{implement, BOOL, GUID},
    Win32::{
        Foundation::{DRAGDROP_E_INVALIDHWND, FILETIME, HWND, LPARAM, POINTL, SIZE},
        System::{
            Com::{
                IDataObject, IStream, DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL, TYMED_ISTREAM,
            },
            DataExchange::RegisterClipboardFormatW,
            Memory::{GlobalLock, GlobalSize, GlobalUnlock},
            Ole::{
                IDropTarget, IDropTarget_Impl, RegisterDragDrop, ReleaseStgMedium, RevokeDragDrop,
                CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE,
            },
            SystemServices::MODIFIERKEYS_FLAGS,
        },
        UI::{
            Shell::{
                DragFinish, DragQueryFileW, CFSTR_FILECONTENTS, CFSTR_FILEDESCRIPTORW,
                CFSTR_INETURLA, CFSTR_INETURLW, HDROP,
            },
            WindowsAndMessaging::EnumChildWindows,
        },
    },
};

use crate::app::commands::file_cmd::image_ext_from_bytes;

#[derive(Clone, Serialize)]
struct DropPayload {
    paths: Vec<String>,
}

#[repr(C)]
#[allow(non_snake_case)]
struct FileGroupDescriptorW {
    cItems: u32,
    fgd: [FileDescriptorW; 1],
}

#[repr(C)]
#[allow(non_snake_case)]
struct FileDescriptorW {
    dwFlags: u32,
    clsid: GUID,
    sizel: SIZE,
    pointl: POINTL,
    dwFileAttributes: u32,
    ftCreationTime: FILETIME,
    ftLastAccessTime: FILETIME,
    ftLastWriteTime: FILETIME,
    nFileSizeHigh: u32,
    nFileSizeLow: u32,
    cFileName: [u16; 260],
}

#[derive(Default)]
pub struct DragDropController {
    drop_targets: Vec<(HWND, IDropTarget)>,
    registration_errors: Vec<String>,
}

thread_local! {
    // OLE drag/drop registration is apartment and thread-affine. The destroyed-mode
    // prototype therefore keeps the replaceable controller on the UI thread instead
    // of placing COM interfaces or HWNDs in Tauri's Send + Sync managed state.
    static REPLACEABLE_DRAG_DROP: RefCell<Option<DragDropController>> = const {
        RefCell::new(None)
    };
}

impl DragDropController {
    pub fn new(hwnd: HWND, app_handle: AppHandle) -> Self {
        let mut controller = DragDropController::default();

        let app_handle = Rc::new(app_handle);
        let mut callback = |child| controller.inject_in_hwnd(child, app_handle.clone());
        let mut trait_obj: &mut dyn FnMut(HWND) -> bool = &mut callback;
        let closure_pointer_pointer: *mut c_void = unsafe { std::mem::transmute(&mut trait_obj) };
        let lparam = LPARAM(closure_pointer_pointer as _);
        unsafe extern "system" fn enumerate_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
            unsafe {
                let closure = &mut *(lparam.0 as *mut c_void as *mut &mut dyn FnMut(HWND) -> bool);
                closure(hwnd).into()
            }
        }
        let _ = unsafe { EnumChildWindows(Some(hwnd), Some(enumerate_callback), lparam) };

        controller
    }

    fn inject_in_hwnd(&mut self, hwnd: HWND, app_handle: Rc<AppHandle>) -> bool {
        let drop_target: IDropTarget = EmojiDropTarget::new(app_handle).into();
        if unsafe { RevokeDragDrop(hwnd) } == Err(DRAGDROP_E_INVALIDHWND.into()) {
            self.registration_errors
                .push(format!("invalid child HWND {hwnd:?}"));
            return true;
        }
        match unsafe { RegisterDragDrop(hwnd, &drop_target) } {
            Ok(()) => self.drop_targets.push((hwnd, drop_target)),
            Err(error) => self
                .registration_errors
                .push(format!("RegisterDragDrop failed for {hwnd:?}: {error}")),
        }
        true
    }

    fn has_registered_targets(&self) -> bool {
        !self.drop_targets.is_empty()
    }

    fn installation_error(&self) -> String {
        if self.registration_errors.is_empty() {
            "no eligible WebView child HWNDs were registered".to_string()
        } else {
            format!(
                "no WebView child HWNDs were registered: {}",
                self.registration_errors.join("; ")
            )
        }
    }
}

impl Drop for DragDropController {
    fn drop(&mut self) {
        for (hwnd, _) in self.drop_targets.drain(..) {
            // Releasing the COM targets without revoking the HWND registration
            // leaves OLE holding stale callbacks across main-window generations.
            let _ = unsafe { RevokeDragDrop(hwnd) };
        }
    }
}

#[implement(IDropTarget)]
pub struct EmojiDropTarget {
    app_handle: Rc<AppHandle>,
    cursor_effect: UnsafeCell<DROPEFFECT>,
    enter_is_valid: UnsafeCell<bool>,
}

impl EmojiDropTarget {
    pub fn new(app_handle: Rc<AppHandle>) -> Self {
        Self {
            app_handle,
            cursor_effect: UnsafeCell::new(DROPEFFECT_NONE),
            enter_is_valid: UnsafeCell::new(false),
        }
    }

    fn format_etc(cf_format: u16, lindex: i32, tymed: u32) -> FORMATETC {
        FORMATETC {
            cfFormat: cf_format,
            ptd: ptr::null_mut(),
            dwAspect: DVASPECT_CONTENT.0,
            lindex,
            tymed,
        }
    }

    unsafe fn has_format(data_obj: &IDataObject, cf_format: u16, tymed: u32, lindex: i32) -> bool {
        let format = Self::format_etc(cf_format, lindex, tymed);
        unsafe { data_obj.QueryGetData(&format).is_ok() }
    }

    unsafe fn read_hglobal_bytes(hglobal: windows::Win32::Foundation::HGLOBAL) -> Option<Vec<u8>> {
        unsafe {
            if hglobal.0.is_null() {
                return None;
            }
            let size = GlobalSize(hglobal);
            if size == 0 {
                return None;
            }
            let ptr = GlobalLock(hglobal) as *const u8;
            if ptr.is_null() {
                return None;
            }
            let slice = std::slice::from_raw_parts(ptr, size);
            let bytes = slice.to_vec();
            let _ = GlobalUnlock(hglobal);
            Some(bytes)
        }
    }

    unsafe fn read_hglobal_wide_string(
        hglobal: windows::Win32::Foundation::HGLOBAL,
    ) -> Option<String> {
        unsafe {
            if hglobal.0.is_null() {
                return None;
            }
            let size = GlobalSize(hglobal);
            if size < 2 {
                return None;
            }
            let ptr = GlobalLock(hglobal) as *const u16;
            if ptr.is_null() {
                return None;
            }
            let len = size / 2;
            let slice = std::slice::from_raw_parts(ptr, len);
            let end = slice.iter().position(|c| *c == 0).unwrap_or(len);
            let text = String::from_utf16_lossy(&slice[..end]);
            let _ = GlobalUnlock(hglobal);
            if text.is_empty() {
                None
            } else {
                Some(text)
            }
        }
    }

    unsafe fn read_hglobal_string(hglobal: windows::Win32::Foundation::HGLOBAL) -> Option<String> {
        let bytes = unsafe { Self::read_hglobal_bytes(hglobal)? };
        let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
        let text = String::from_utf8_lossy(&bytes[..end]).to_string();
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }

    unsafe fn iterate_file_paths(data_obj: &IDataObject) -> Option<(Vec<PathBuf>, HDROP)> {
        unsafe {
            let drop_format = Self::format_etc(CF_HDROP.0, -1, TYMED_HGLOBAL.0 as u32);

            match data_obj.GetData(&drop_format) {
                Ok(medium) => {
                    let hdrop = HDROP(medium.u.hGlobal.0 as _);
                    let item_count = DragQueryFileW(hdrop, 0xFFFFFFFF, None);
                    let mut paths = Vec::new();

                    for i in 0..item_count {
                        let character_count = DragQueryFileW(hdrop, i, None) as usize;
                        let str_len = character_count + 1;
                        let mut path_buf = vec![0; str_len];
                        DragQueryFileW(hdrop, i, Some(&mut path_buf));
                        paths.push(OsString::from_wide(&path_buf[0..character_count]).into());
                    }

                    Some((paths, hdrop))
                }
                Err(_) => None,
            }
        }
    }

    unsafe fn read_stream(stream: &IStream) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        let mut buffer = [0u8; 8192];
        loop {
            let mut read = 0u32;
            let hr = unsafe {
                stream.Read(
                    buffer.as_mut_ptr() as *mut c_void,
                    buffer.len() as u32,
                    Some(&mut read),
                )
            };
            if hr.is_err() {
                return None;
            }
            if read == 0 {
                break;
            }
            out.extend_from_slice(&buffer[..read as usize]);
        }
        Some(out)
    }

    unsafe fn read_file_contents(data_obj: &IDataObject, index: i32) -> Option<Vec<u8>> {
        let cf_file_contents = RegisterClipboardFormatW(CFSTR_FILECONTENTS);
        if cf_file_contents == 0 {
            return None;
        }
        let format_stream =
            Self::format_etc(cf_file_contents as u16, index, TYMED_ISTREAM.0 as u32);
        if let Ok(mut medium) = data_obj.GetData(&format_stream) {
            let stream_ptr = &medium.u.pstm as *const _ as *const Option<IStream>;
            let stream = unsafe { &*stream_ptr };
            let bytes = stream.as_ref().and_then(|s| Self::read_stream(s));
            ReleaseStgMedium(&mut medium);
            if bytes.as_ref().map(|b| !b.is_empty()).unwrap_or(false) {
                return bytes;
            }
        }

        let format_hglobal =
            Self::format_etc(cf_file_contents as u16, index, TYMED_HGLOBAL.0 as u32);
        if let Ok(mut medium) = data_obj.GetData(&format_hglobal) {
            let bytes = Self::read_hglobal_bytes(medium.u.hGlobal);
            ReleaseStgMedium(&mut medium);
            return bytes.filter(|b| !b.is_empty());
        }

        None
    }

    unsafe fn read_virtual_files(data_obj: &IDataObject) -> Vec<(String, Vec<u8>)> {
        unsafe {
            let cf_file_descriptor = RegisterClipboardFormatW(CFSTR_FILEDESCRIPTORW);
            if cf_file_descriptor == 0 {
                return Vec::new();
            }

            let format = Self::format_etc(cf_file_descriptor as u16, -1, TYMED_HGLOBAL.0 as u32);
            let mut out = Vec::new();

            let Ok(mut medium) = data_obj.GetData(&format) else {
                return out;
            };

            let hglobal = medium.u.hGlobal;
            let ptr = GlobalLock(hglobal) as *const FileGroupDescriptorW;
            if ptr.is_null() {
                ReleaseStgMedium(&mut medium);
                return out;
            }

            let count = (*ptr).cItems as usize;
            let first = (*ptr).fgd.as_ptr();
            let descriptors = std::slice::from_raw_parts(first, count);
            for (index, descriptor) in descriptors.iter().enumerate() {
                let name_end = descriptor
                    .cFileName
                    .iter()
                    .position(|c| *c == 0)
                    .unwrap_or(descriptor.cFileName.len());
                if name_end == 0 {
                    continue;
                }
                let name = String::from_utf16_lossy(&descriptor.cFileName[..name_end]);
                if let Some(bytes) = Self::read_file_contents(data_obj, index as i32) {
                    out.push((name, bytes));
                }
            }

            let _ = GlobalUnlock(hglobal);
            ReleaseStgMedium(&mut medium);
            out
        }
    }

    unsafe fn read_inet_url(data_obj: &IDataObject) -> Option<String> {
        unsafe {
            let cf_inet_w = RegisterClipboardFormatW(CFSTR_INETURLW);
            if cf_inet_w != 0 {
                let format = Self::format_etc(cf_inet_w as u16, -1, TYMED_HGLOBAL.0 as u32);
                if let Ok(mut medium) = data_obj.GetData(&format) {
                    let url = Self::read_hglobal_wide_string(medium.u.hGlobal);
                    ReleaseStgMedium(&mut medium);
                    if url.is_some() {
                        return url;
                    }
                }
            }

            let cf_inet_a = RegisterClipboardFormatW(CFSTR_INETURLA);
            if cf_inet_a != 0 {
                let format = Self::format_etc(cf_inet_a as u16, -1, TYMED_HGLOBAL.0 as u32);
                if let Ok(mut medium) = data_obj.GetData(&format) {
                    let url = Self::read_hglobal_string(medium.u.hGlobal);
                    ReleaseStgMedium(&mut medium);
                    return url;
                }
            }

            None
        }
    }

    fn emit_file_drop(app_handle: &AppHandle, paths: Vec<String>) {
        let payload = DropPayload { paths };
        let _ = app_handle.emit("tauri://file-drop", payload.clone());
        let _ = app_handle.emit("tauri://drag-drop", payload);
    }

    fn emit_drag_enter(app_handle: &AppHandle) {
        let payload = DropPayload { paths: Vec::new() };
        let _ = app_handle.emit("tauri://file-drop-hover", payload.clone());
        let _ = app_handle.emit("tauri://drag-enter", payload);
    }

    fn emit_drag_leave(app_handle: &AppHandle) {
        let payload = DropPayload { paths: Vec::new() };
        let _ = app_handle.emit("tauri://file-drop-cancelled", payload.clone());
        let _ = app_handle.emit("tauri://drag-leave", payload);
    }

    fn ensure_temp_dir() -> Option<PathBuf> {
        let dir = std::env::temp_dir().join("TieZ_DragDrop");
        if std::fs::create_dir_all(&dir).is_err() {
            return None;
        }
        Some(dir)
    }

    fn sanitize_filename(name: &str) -> Option<String> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        let file_name = std::path::Path::new(name)
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.trim().to_string());
        file_name.filter(|s| !s.is_empty())
    }

    fn ensure_extension(file_name: &str, bytes: &[u8]) -> String {
        if std::path::Path::new(file_name).extension().is_some() {
            return file_name.to_string();
        }
        if let Some(ext) = image_ext_from_bytes(bytes) {
            return format!("{}.{}", file_name, ext);
        }
        format!("{}.bin", file_name)
    }

    fn save_temp_bytes(name: Option<&str>, bytes: &[u8]) -> Option<String> {
        if bytes.is_empty() {
            return None;
        }
        let dir = Self::ensure_temp_dir()?;
        let base = name
            .and_then(Self::sanitize_filename)
            .unwrap_or_else(|| format!("drop_{}", Uuid::new_v4()));
        let file_name = Self::ensure_extension(&base, bytes);
        let path = dir.join(file_name);
        if std::fs::write(&path, bytes).is_err() {
            return None;
        }
        Some(path.to_string_lossy().to_string())
    }

    async fn save_url_to_temp(url: String) -> Option<String> {
        let trimmed = url.trim();
        if trimmed.is_empty() {
            return None;
        }
        let parsed = reqwest::Url::parse(trimmed).ok()?;
        let scheme = parsed.scheme();
        if scheme != "http" && scheme != "https" {
            return None;
        }

        let file_name = parsed
            .path_segments()
            .and_then(|mut segments| segments.next_back())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .ok()?;
        let response = client.get(parsed).send().await.ok()?;
        if !response.status().is_success() {
            return None;
        }
        let bytes = response.bytes().await.ok()?;
        if bytes.is_empty() {
            return None;
        }

        Self::save_temp_bytes(file_name.as_deref(), &bytes)
    }
}

// IDropTarget_Impl fixes these COM callback signatures as safe methods with raw out-pointers.
#[allow(non_snake_case, clippy::not_unsafe_ptr_arg_deref)]
impl IDropTarget_Impl for EmojiDropTarget_Impl {
    fn DragEnter(
        &self,
        pDataObj: WRef<'_, IDataObject>,
        _grfKeyState: MODIFIERKEYS_FLAGS,
        _pt: &POINTL,
        pdwEffect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        let data_obj = pDataObj.as_ref().expect("Received null IDataObject");
        let cf_file_descriptor = unsafe { RegisterClipboardFormatW(CFSTR_FILEDESCRIPTORW) };
        let cf_inet_url_w = unsafe { RegisterClipboardFormatW(CFSTR_INETURLW) };
        let cf_inet_url_a = unsafe { RegisterClipboardFormatW(CFSTR_INETURLA) };
        let valid = unsafe {
            EmojiDropTarget::has_format(data_obj, CF_HDROP.0, TYMED_HGLOBAL.0 as u32, -1)
                || (cf_file_descriptor != 0
                    && EmojiDropTarget::has_format(
                        data_obj,
                        cf_file_descriptor as u16,
                        TYMED_HGLOBAL.0 as u32,
                        -1,
                    ))
                || (cf_inet_url_w != 0
                    && EmojiDropTarget::has_format(
                        data_obj,
                        cf_inet_url_w as u16,
                        TYMED_HGLOBAL.0 as u32,
                        -1,
                    ))
                || (cf_inet_url_a != 0
                    && EmojiDropTarget::has_format(
                        data_obj,
                        cf_inet_url_a as u16,
                        TYMED_HGLOBAL.0 as u32,
                        -1,
                    ))
        };

        unsafe {
            *self.enter_is_valid.get() = valid;
        }

        if valid {
            EmojiDropTarget::emit_drag_enter(self.app_handle.as_ref());
        }

        let cursor_effect = if valid {
            DROPEFFECT_COPY
        } else {
            DROPEFFECT_NONE
        };

        unsafe {
            *pdwEffect = cursor_effect;
            *self.cursor_effect.get() = cursor_effect;
        }

        Ok(())
    }

    fn DragOver(
        &self,
        _grfKeyState: MODIFIERKEYS_FLAGS,
        _pt: &POINTL,
        pdwEffect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        unsafe {
            *pdwEffect = *self.cursor_effect.get();
        }
        Ok(())
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        if unsafe { *self.enter_is_valid.get() } {
            EmojiDropTarget::emit_drag_leave(self.app_handle.as_ref());
        }
        Ok(())
    }

    fn Drop(
        &self,
        pDataObj: WRef<'_, IDataObject>,
        _grfKeyState: MODIFIERKEYS_FLAGS,
        _pt: &POINTL,
        _pdwEffect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        if !unsafe { *self.enter_is_valid.get() } {
            return Ok(());
        }

        let data_obj = pDataObj.as_ref().expect("Received null IDataObject");

        if let Some((paths, hdrop)) = unsafe { EmojiDropTarget::iterate_file_paths(data_obj) } {
            let paths: Vec<String> = paths
                .into_iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect();
            if !paths.is_empty() {
                EmojiDropTarget::emit_file_drop(self.app_handle.as_ref(), paths);
            }
            unsafe { DragFinish(hdrop) };
        } else {
            let virtual_files = unsafe { EmojiDropTarget::read_virtual_files(data_obj) };
            if !virtual_files.is_empty() {
                let app_handle = self.app_handle.as_ref().clone();
                std::thread::spawn(move || {
                    let mut saved = Vec::new();
                    for (name, bytes) in virtual_files {
                        if let Some(path) =
                            EmojiDropTarget::save_temp_bytes(Some(name.as_str()), &bytes)
                        {
                            saved.push(path);
                        }
                    }
                    if !saved.is_empty() {
                        EmojiDropTarget::emit_file_drop(&app_handle, saved);
                    }
                });
            } else if let Some(url) = unsafe { EmojiDropTarget::read_inet_url(data_obj) } {
                let app_handle = self.app_handle.as_ref().clone();
                tauri::async_runtime::spawn(async move {
                    if let Some(path) = EmojiDropTarget::save_url_to_temp(url).await {
                        EmojiDropTarget::emit_file_drop(&app_handle, vec![path]);
                    }
                });
            }
        }

        if unsafe { *self.enter_is_valid.get() } {
            EmojiDropTarget::emit_drag_leave(self.app_handle.as_ref());
            unsafe {
                *self.enter_is_valid.get() = false;
            }
        }

        Ok(())
    }
}

pub fn create_emoji_drag_drop(app_handle: AppHandle) -> Option<DragDropController> {
    if let Some(window) = app_handle.get_webview_window("main") {
        if let Ok(hwnd) = window.hwnd() {
            return Some(DragDropController::new(hwnd, app_handle));
        }
    }
    None
}

fn create_emoji_drag_drop_result(app_handle: AppHandle) -> Result<DragDropController, String> {
    let window = app_handle
        .get_webview_window("main")
        .ok_or_else(|| "main WebView window is unavailable".to_string())?;
    let hwnd = window
        .hwnd()
        .map_err(|error| format!("failed to obtain the main window HWND: {error}"))?;
    Ok(DragDropController::new(hwnd, app_handle))
}

pub fn register_emoji_drag_drop(app_handle: AppHandle) {
    // Preserve the default product mode's original process-lifetime registration.
    if let Some(controller) = create_emoji_drag_drop(app_handle) {
        std::mem::forget(controller);
    }
}

pub fn replace_emoji_drag_drop_on_current_thread(app_handle: AppHandle) -> Result<(), String> {
    let controller = create_emoji_drag_drop_result(app_handle)?;
    if !controller.has_registered_targets() {
        return Err(controller.installation_error());
    }
    REPLACEABLE_DRAG_DROP.with(|slot| {
        let previous = slot.replace(Some(controller));
        drop(previous);
    });
    Ok(())
}

pub fn revoke_emoji_drag_drop_on_current_thread() {
    REPLACEABLE_DRAG_DROP.with(|slot| {
        let controller = slot.replace(None);
        drop(controller);
    });
}
