//! Read-only Windows application discovery. No process launch, registry write or command interpreter.
//! Call off the dispatcher; cancellation is cooperative between non-abortable Shell COM calls.
use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use windows::core::{w, Interface, PCWSTR, PWSTR};
use windows::Win32::Foundation::{PROPERTYKEY, RPC_E_CHANGED_MODE};
use windows::Win32::Storage::FileSystem::GetBinaryTypeW;
use windows::Win32::System::Com::{
    CoInitializeEx, CoTaskMemFree, CoUninitialize, IBindCtx, COINIT_APARTMENTTHREADED,
    COINIT_DISABLE_OLE1DDE,
};
use windows::Win32::System::WindowsProgramming::{SCS_32BIT_BINARY, SCS_64BIT_BINARY};
use windows::Win32::UI::Shell::PropertiesSystem::PSGetPropertyKeyFromName;
use windows::Win32::UI::Shell::{
    AssocQueryStringW, BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem2,
    SHAssocEnumHandlers, SHGetKnownFolderItem, ASSOCF_VERIFY, ASSOCSTR_FRIENDLYAPPNAME,
    ASSOC_FILTER_NONE, KF_FLAG_DEFAULT, SIGDN_NORMALDISPLAY,
};

const MAX_ITEMS: usize = 4096;
const MAX_ASSOCIATIONS: usize = 256;
const MAX_WIDE: usize = 32768;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    Desktop,
    Packaged,
    Unavailable,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Unavailable {
    MissingTarget,
    InvalidIdentity,
    NotNativeExecutable,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Application {
    pub name: String,
    /// Original Shell ID, retained for matching without pretending an opaque desktop ID is launchable.
    pub source_id: String,
    pub handler: Option<String>,
    pub kind: Kind,
    pub unavailable: Option<Unavailable>,
}
#[derive(Default)]
pub struct Catalog {
    pub applications: Vec<Application>,
    pub scanned: usize,
    pub unreadable: usize,
    pub duplicates: usize,
    pub limited: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Content {
    Text,
    Code,
    RichText,
    Image,
    File,
    Video,
    Url,
    Link,
}
impl Content {
    fn extension(self) -> &'static str {
        match self {
            Self::Image => ".png",
            Self::RichText | Self::Url | Self::Link => ".html",
            Self::Video => ".mp4",
            _ => ".txt",
        }
    }
    fn default_association(self) -> &'static str {
        match self {
            Self::Url | Self::Link => "http",
            _ => self.extension(),
        }
    }
    fn keywords(self) -> &'static [&'static str] {
        match self {
            Self::Text | Self::Code => &[
                "text",
                "note",
                "code",
                "edit",
                "write",
                "office",
                "word",
                "记事本",
                "文档",
            ],
            Self::RichText => &[
                "word", "office", "write", "writer", "wps", "browser", "chrome", "edge", "firefox",
                "document", "html",
            ],
            Self::Image => &[
                "photo", "paint", "image", "adobe", "picture", "snip", "viewer", "画图", "照片",
                "看图",
            ],
            Self::Url | Self::Link => &["browser", "chrome", "edge", "firefox", "web", "internet"],
            _ => &[],
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecommendationReason {
    Association,
    Keyword,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Recommendation {
    pub application: Application,
    pub reason: RecommendationReason,
}
pub struct Recommendations {
    pub applications: Vec<Recommendation>,
    pub association_error: Option<&'static str>,
    pub unreadable: usize,
    pub limited: bool,
    /// Display only. System opening always delegates to the current Windows association.
    pub default_name: Option<String>,
}
struct Apartment(bool);
impl Apartment {
    fn enter() -> Result<Self, &'static str> {
        let result =
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        if result.is_ok() {
            Ok(Self(true))
        } else if result == RPC_E_CHANGED_MODE {
            Ok(Self(false))
        } else {
            Err("apps_com")
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
struct TaskString(PWSTR);
impl Drop for TaskString {
    fn drop(&mut self) {
        unsafe { CoTaskMemFree(Some(self.0 .0.cast())) };
    }
}
fn string(value: PWSTR) -> Result<String, &'static str> {
    let owned = TaskString(value);
    if owned.0.is_null() {
        return Err("apps_property");
    }
    // Shell strings are COM-allocated NUL-terminated UTF-16; bound conversion/allocation.
    for size in 0..MAX_WIDE {
        if unsafe { *owned.0 .0.add(size) } == 0 {
            let units = unsafe { std::slice::from_raw_parts(owned.0 .0, size) };
            return String::from_utf16(units).map_err(|_| "apps_property");
        }
    }
    Err("apps_property_size")
}
fn property(name: PCWSTR) -> Result<PROPERTYKEY, &'static str> {
    let mut key = PROPERTYKEY::default();
    unsafe { PSGetPropertyKeyFromName(name, &mut key) }.map_err(|_| "apps_property_key")?;
    Ok(key)
}
fn cancelled(cancel: &AtomicBool) -> Result<(), &'static str> {
    if cancel.load(Ordering::Acquire) {
        Err("apps_cancelled")
    } else {
        Ok(())
    }
}
fn packaged(value: &str) -> bool {
    value.len() <= 255
        && value.split_once('!').is_some_and(|(family, app)| {
            !family.is_empty()
                && !app.is_empty()
                && !app.contains('!')
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_.!-".contains(&byte))
        })
}
fn native_executable(value: &str) -> bool {
    if !Path::new(value).is_absolute() || value.chars().any(char::is_control) {
        return false;
    }
    if !Path::new(value)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
    {
        return false;
    }
    let wide: Vec<u16> = value.encode_utf16().chain([0]).collect();
    if wide.len() > MAX_WIDE {
        return false;
    }
    let mut kind = 0;
    unsafe { GetBinaryTypeW(PCWSTR(wide.as_ptr()), &mut kind) }.is_ok()
        && matches!(kind, SCS_32BIT_BINARY | SCS_64BIT_BINARY)
}
fn classify(
    name: String,
    source_id: String,
    target: String,
    host: Option<u32>,
    native: impl FnOnce(&str) -> bool,
) -> Application {
    let (handler, kind, unavailable) = if matches!(host, Some(1 | 2)) {
        // Windows also reports packaged desktop applications as host 2.
        if packaged(&source_id) {
            (Some(source_id.clone()), Kind::Packaged, None)
        } else {
            (None, Kind::Unavailable, Some(Unavailable::InvalidIdentity))
        }
    } else if !target.is_empty() && native(&target) {
        (Some(target), Kind::Desktop, None)
    } else {
        let reason = if target.is_empty() {
            Unavailable::MissingTarget
        } else {
            Unavailable::NotNativeExecutable
        };
        (None, Kind::Unavailable, Some(reason))
    };
    Application {
        name,
        source_id,
        handler,
        kind,
        unavailable,
    }
}
fn identity(application: &Application) -> String {
    if application.handler.is_none() && application.source_id.is_empty() {
        return application.name.clone();
    }
    let handler = application
        .handler
        .as_deref()
        .unwrap_or(&application.source_id);
    if application.kind == Kind::Desktop {
        handler.to_lowercase()
    } else {
        handler.to_owned()
    }
}
fn normalize(catalog: &mut Catalog) {
    catalog
        .applications
        .sort_by_cached_key(|app| (app.name.to_lowercase(), app.source_id.clone()));
    let mut seen = HashSet::new();
    catalog.applications.retain(|app| {
        let keep = seen.insert((app.kind as u8, identity(app)));
        if !keep {
            catalog.duplicates += 1;
        }
        keep
    });
}
/// Read the per-user Shell AppsFolder. Unsupported entries remain visible with typed reasons.
/// A top-level COM/enumeration failure is an error, never an empty successful catalog.
pub fn scan(cancel: &AtomicBool) -> Result<Catalog, &'static str> {
    cancelled(cancel)?;
    let _apartment = Apartment::enter()?;
    let id_key = property(w!("System.AppUserModel.ID"))?;
    let target_key = property(w!("System.Link.TargetParsingPath"))?;
    let host_key = property(w!("System.AppUserModel.HostEnvironment"))?;
    let folder: IShellItem2 =
        unsafe { SHGetKnownFolderItem(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None) }
            .map_err(|_| "apps_folder")?;
    let items: IEnumShellItems =
        unsafe { folder.BindToHandler(None::<&IBindCtx>, &BHID_EnumItems) }
            .map_err(|_| "apps_enumeration")?;
    let mut result = Catalog::default();
    loop {
        cancelled(cancel)?;
        let mut next = [None];
        let mut fetched = 0;
        unsafe { items.Next(&mut next, Some(&mut fetched)) }.map_err(|_| "apps_enumeration")?;
        if fetched == 0 {
            break;
        }
        if result.scanned == MAX_ITEMS {
            result.limited = true;
            break;
        }
        result.scanned += 1;
        let Some(item) = next[0].take() else {
            result.unreadable += 1;
            continue;
        };
        let Ok(item) = item.cast::<IShellItem2>() else {
            result.unreadable += 1;
            continue;
        };
        let name = unsafe { item.GetDisplayName(SIGDN_NORMALDISPLAY) }
            .ok()
            .and_then(|s| string(s).ok());
        let Some(name) = name.filter(|s| !s.trim().is_empty() && !s.chars().any(char::is_control))
        else {
            result.unreadable += 1;
            continue;
        };
        let id = unsafe { item.GetString(&id_key) }
            .ok()
            .and_then(|s| string(s).ok())
            .unwrap_or_default();
        let target = unsafe { item.GetString(&target_key) }
            .ok()
            .and_then(|s| string(s).ok())
            .unwrap_or_default();
        let host = unsafe { item.GetUInt32(&host_key) }.ok();
        result
            .applications
            .push(classify(name, id, target, host, native_executable));
    }
    cancelled(cancel)?;
    normalize(&mut result);
    Ok(result)
}
fn default_name(content: Content) -> Option<String> {
    let association: Vec<u16> = content
        .default_association()
        .encode_utf16()
        .chain([0])
        .collect();
    let mut buffer = vec![0u16; MAX_WIDE];
    let mut size = buffer.len() as u32;
    unsafe {
        AssocQueryStringW(
            ASSOCF_VERIFY,
            ASSOCSTR_FRIENDLYAPPNAME,
            PCWSTR(association.as_ptr()),
            PCWSTR::null(),
            Some(PWSTR(buffer.as_mut_ptr())),
            &mut size,
        )
    }
    .ok()
    .ok()?;
    let end = buffer.iter().position(|unit| *unit == 0)?;
    let name = String::from_utf16(&buffer[..end]).ok()?;
    (!name.trim().is_empty() && !name.chars().any(char::is_control)).then_some(name)
}
fn merge(content: Content, catalog: &Catalog, associated: Vec<Application>) -> Vec<Recommendation> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for app in associated {
        if app.handler.is_some() && seen.insert((app.kind as u8, identity(&app))) {
            result.push(Recommendation {
                application: app,
                reason: RecommendationReason::Association,
            });
        }
    }
    for app in &catalog.applications {
        if app.handler.is_some()
            && content
                .keywords()
                .iter()
                .any(|word| app.name.to_lowercase().contains(word))
            && seen.insert((app.kind as u8, identity(app)))
        {
            result.push(Recommendation {
                application: app.clone(),
                reason: RecommendationReason::Keyword,
            });
        }
    }
    result
}
/// Native Open-With handlers precede legacy keyword suggestions. Never launches a handler.
/// Failure to read associations is explicit; the independent catalog/keyword choices remain usable.
pub fn recommendations(
    content: Content,
    catalog: &Catalog,
    cancel: &AtomicBool,
) -> Result<Recommendations, &'static str> {
    cancelled(cancel)?;
    let _apartment = Apartment::enter()?;
    let extension: Vec<u16> = content.extension().encode_utf16().chain([0]).collect();
    let mut associated = Vec::new();
    let mut unreadable = 0;
    let mut limited = false;
    let mut association_error = None;
    match unsafe { SHAssocEnumHandlers(PCWSTR(extension.as_ptr()), ASSOC_FILTER_NONE) } {
        Err(_) => association_error = Some("apps_associations"),
        Ok(items) => {
            for index in 0..=MAX_ASSOCIATIONS {
                cancelled(cancel)?;
                let mut next = [None];
                let mut fetched = 0;
                if unsafe { items.Next(&mut next, Some(&mut fetched)) }.is_err() {
                    association_error = Some("apps_associations");
                    break;
                }
                if fetched == 0 {
                    break;
                }
                if index == MAX_ASSOCIATIONS {
                    limited = true;
                    break;
                }
                let Some(item) = next[0].take() else {
                    unreadable += 1;
                    continue;
                };
                let Some(id) = (unsafe { item.GetName() })
                    .ok()
                    .and_then(|s| string(s).ok())
                else {
                    unreadable += 1;
                    continue;
                };
                let matched = catalog.applications.iter().find(|app| {
                    app.handler.is_some()
                        && (app.source_id == id
                            || app
                                .handler
                                .as_deref()
                                .is_some_and(|value| value.eq_ignore_ascii_case(&id)))
                });
                if let Some(app) = matched {
                    associated.push(app.clone());
                    continue;
                }
                let label = unsafe { item.GetUIName() }
                    .ok()
                    .and_then(|s| string(s).ok())
                    .unwrap_or_default();
                if label.trim().is_empty() || label.chars().any(char::is_control) {
                    unreadable += 1;
                    continue;
                }
                let host = packaged(&id).then_some(1);
                let app = classify(label, id.clone(), id, host, native_executable);
                if app.handler.is_some() {
                    associated.push(app);
                } else {
                    unreadable += 1;
                }
            }
        }
    }
    cancelled(cancel)?;
    let name = default_name(content);
    cancelled(cancel)?;
    Ok(Recommendations {
        applications: merge(content, catalog, associated),
        association_error,
        unreadable,
        limited,
        default_name: name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn desktop(name: &str, id: &str, path: &str) -> Application {
        classify(name.into(), id.into(), path.into(), Some(0), |_| true)
    }
    #[test]
    fn unavailable_entries_without_shell_ids_do_not_collapse_unrelated_names() {
        let mut catalog = Catalog {
            applications: ["First missing", "Second missing"]
                .into_iter()
                .map(|name| classify(name.into(), "".into(), "".into(), None, |_| false))
                .collect(),
            ..Default::default()
        };
        normalize(&mut catalog);
        assert_eq!(catalog.applications.len(), 2);
        assert_eq!(catalog.duplicates, 0);
    }
    #[test]
    fn opaque_desktop_ids_become_paths_and_unsupported_entries_remain_explicit() {
        let app = desktop(
            "Editor",
            "Opaque.Editor",
            r"C:\Program Files\原生 editor.exe",
        );
        assert_eq!(app.source_id, "Opaque.Editor");
        assert_eq!(
            app.handler.as_deref(),
            Some(r"C:\Program Files\原生 editor.exe")
        );
        assert_eq!(app.kind, Kind::Desktop);
        let missing = classify(
            "Missing".into(),
            "Opaque.Missing".into(),
            "".into(),
            Some(0),
            |_| panic!("No target to validate"),
        );
        assert!(missing.handler.is_none());
        assert_eq!(missing.unavailable, Some(Unavailable::MissingTarget));
        let script = classify(
            "Script".into(),
            "Script.ID".into(),
            r"C:\script.cmd".into(),
            Some(0),
            |_| false,
        );
        assert_eq!(script.unavailable, Some(Unavailable::NotNativeExecutable));
    }
    #[test]
    fn packaged_ids_are_strict_and_not_confused_with_desktop_ids() {
        for host in [1, 2] {
            for id in ["Family_123!App", "Family.1_ab!Microsoft.Editor-1"] {
                let app = classify("Package".into(), id.into(), "".into(), Some(host), |_| {
                    panic!("No desktop validation")
                });
                assert_eq!(app.handler.as_deref(), Some(id));
                assert_eq!(app.kind, Kind::Packaged);
            }
        }
        for id in ["Family!", "!App", "A!B!C", "A!B /c", "A!B\n", "中文!App"] {
            let app = classify("Bad".into(), id.into(), "".into(), Some(1), |_| true);
            assert_eq!(app.unavailable, Some(Unavailable::InvalidIdentity));
        }
        assert!(!packaged(&format!("{}!App", "x".repeat(255))));
        let desktop = desktop("Desktop", "Contains!Bang", r"C:\editor.exe");
        assert_eq!(desktop.kind, Kind::Desktop);
    }
    #[test]
    fn normalization_is_stable_global_path_dedup_and_keeps_unavailable_rows() {
        let mut catalog = Catalog {
            applications: vec![
                desktop("z editor", "AliasZ", r"C:\Editor.exe"),
                desktop("A editor", "AliasA", r"c:\EDITOR.exe"),
                classify(
                    "Missing".into(),
                    "Missing.ID".into(),
                    "".into(),
                    Some(0),
                    |_| false,
                ),
            ],
            ..Default::default()
        };
        normalize(&mut catalog);
        assert_eq!(catalog.applications.len(), 2);
        assert_eq!(catalog.duplicates, 1);
        assert_eq!(catalog.applications[0].source_id, "AliasA");
        assert_eq!(catalog.applications[1].kind, Kind::Unavailable);
    }
    #[test]
    fn association_first_keyword_supplement_has_no_duplicate_or_unsupported_suggestions() {
        let first = desktop("Native Choice", "First", r"C:\first.exe");
        let editor = desktop("Code Editor", "Code", r"C:\code.exe");
        let catalog = Catalog {
            applications: vec![
                first.clone(),
                editor.clone(),
                classify(
                    "Text Missing".into(),
                    "Missing".into(),
                    "".into(),
                    Some(0),
                    |_| false,
                ),
            ],
            ..Default::default()
        };
        let list = merge(Content::Text, &catalog, vec![first.clone(), first]);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].reason, RecommendationReason::Association);
        assert_eq!(list[1].reason, RecommendationReason::Keyword);
        assert_eq!(list[1].application, editor);
        assert!(merge(Content::Video, &catalog, vec![]).is_empty());
        assert!(merge(Content::File, &catalog, vec![]).is_empty());
    }
    #[test]
    fn legacy_extensions_keywords_and_current_default_protocol_are_preserved() {
        assert_eq!(Content::Code.extension(), ".txt");
        assert_eq!(Content::RichText.extension(), ".html");
        assert_eq!(Content::Video.extension(), ".mp4");
        assert_eq!(Content::Url.default_association(), "http");
        assert_eq!(Content::Link.extension(), ".html");
        for (content, name) in [
            (Content::Text, "记事本"),
            (Content::Image, "照片"),
            (Content::RichText, "WPS"),
            (Content::Url, "Firefox"),
        ] {
            let catalog = Catalog {
                applications: vec![desktop(name, "ID", r"C:\owned.exe")],
                ..Default::default()
            };
            assert_eq!(merge(content, &catalog, vec![]).len(), 1);
        }
    }
    #[test]
    fn cancellation_prevents_any_com_enumeration_or_default_lookup() {
        let cancel = AtomicBool::new(true);
        assert!(matches!(scan(&cancel), Err("apps_cancelled")));
        assert!(matches!(
            recommendations(Content::Text, &Catalog::default(), &cancel),
            Err("apps_cancelled")
        ));
    }
    #[test]
    fn native_validation_rejects_commands_scripts_missing_and_non_absolute_targets() {
        for path in [
            "cmd.exe",
            r"C:\missing editor.exe",
            r"C:\script.cmd",
            r"C:\file.txt",
            "C:\\editor.exe\n",
        ] {
            assert!(!native_executable(path));
        }
        assert!(native_executable(
            std::env::current_exe().unwrap().to_str().unwrap()
        ));
    }
}
