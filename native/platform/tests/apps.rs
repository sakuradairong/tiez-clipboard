//! Explicit desktop acceptance. Reads installed app metadata only; never invokes an application.
#![cfg(windows)]
use std::sync::atomic::AtomicBool;
use tiez_native_platform::apps::{self, Catalog, Content, Kind, RecommendationReason};
use windows::core::w;
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::System::Com::{
    CoInitializeEx, CoUninitialize, COINIT, COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessHandleCount};

fn handles() -> u32 {
    let mut count = 0;
    unsafe { GetProcessHandleCount(GetCurrentProcess(), &mut count) }.unwrap();
    count
}
fn validate(catalog: &Catalog) {
    assert!(
        catalog.scanned > 0,
        "Interactive AppsFolder unexpectedly empty"
    );
    assert!(!catalog.limited, "Real catalog exceeded explicit item cap");
    assert_eq!(
        catalog.applications.len() + catalog.unreadable + catalog.duplicates,
        catalog.scanned
    );
    for app in &catalog.applications {
        match app.kind {
            Kind::Desktop => {
                let path = std::path::Path::new(app.handler.as_ref().unwrap());
                assert!(path.is_absolute() && path.is_file());
                assert!(app.unavailable.is_none());
            }
            Kind::Packaged => {
                assert_eq!(app.handler.as_ref(), Some(&app.source_id));
                assert!(app.source_id.contains('!'));
                assert!(app.unavailable.is_none());
            }
            Kind::Unavailable => {
                assert!(app.handler.is_none());
                assert!(app.unavailable.is_some());
            }
        }
    }
}
fn run(mode: COINIT, opposite: COINIT) {
    let cancel = AtomicBool::new(false);
    // An explicit caller-owned apartment must survive nested discovery calls unchanged.
    assert_eq!(unsafe { CoInitializeEx(None, mode) }.0, 0);
    let mut catalog = apps::scan(&cancel).expect("Real Shell scan failed");
    validate(&catalog);
    for kind in [
        Content::Text,
        Content::Code,
        Content::RichText,
        Content::Image,
        Content::File,
        Content::Video,
        Content::Url,
        Content::Link,
    ] {
        let recommendations =
            apps::recommendations(kind, &catalog, &cancel).expect("Native recommendations failed");
        let mut seen = std::collections::HashSet::new();
        let mut keyword_seen = false;
        for suggestion in &recommendations.applications {
            assert!(suggestion.application.handler.is_some());
            if suggestion.reason == RecommendationReason::Keyword {
                keyword_seen = true;
            } else {
                assert!(!keyword_seen, "Association lost priority");
            }
            let key = if suggestion.application.kind == Kind::Desktop {
                suggestion
                    .application
                    .handler
                    .as_ref()
                    .unwrap()
                    .to_lowercase()
            } else {
                suggestion.application.handler.clone().unwrap()
            };
            assert!(
                seen.insert((suggestion.application.kind as u8, key)),
                "Duplicate recommendation"
            );
        }
        println!("content={kind:?} recommended={} unreadable={} limited={} association_error={:?} has_default={}",recommendations.applications.len(),recommendations.unreadable,recommendations.limited,recommendations.association_error,recommendations.default_name.is_some());
    }
    // Warm Shell caches before observing repeated scoped enumeration/release.
    apps::scan(&cancel).unwrap();
    let before = handles();
    let mut samples = Vec::new();
    for _ in 0..8 {
        catalog = apps::scan(&cancel).unwrap();
        validate(&catalog);
        samples.push(handles());
    }
    assert_eq!(
        unsafe { CoInitializeEx(None, opposite) },
        RPC_E_CHANGED_MODE,
        "Discovery changed caller apartment"
    );
    println!("mode={} scanned={} available={} packaged={} unavailable={} unreadable={} duplicates={} handles_before={} samples={samples:?}",mode.0,catalog.scanned,catalog.applications.iter().filter(|a|a.handler.is_some()).count(),catalog.applications.iter().filter(|a|a.kind==Kind::Packaged).count(),catalog.applications.iter().filter(|a|a.kind==Kind::Unavailable).count(),catalog.unreadable,catalog.duplicates,before);
    // Reject growth beyond a small fixed Shell/ETW initialization allowance after warmup.
    assert!(
        samples.last().copied().unwrap() <= before + 4,
        "Repeated scans retained process handles"
    );
    assert!(unsafe { GetModuleHandleW(w!("WebView2Loader.dll")) }.is_err());
    assert!(unsafe { GetModuleHandleW(w!("Microsoft.Web.WebView2.Core.dll")) }.is_err());
    unsafe { CoUninitialize() };
    assert_eq!(
        unsafe { CoInitializeEx(None, opposite) }.0,
        0,
        "Unbalanced nested COM initialization"
    );
    unsafe { CoUninitialize() };
}
#[test]
#[ignore = "requires an interactive Windows Shell and reads the current user's installed app metadata"]
fn real_shell_catalog_associations_and_nested_apartments_release() {
    std::thread::spawn(|| run(COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED))
        .join()
        .unwrap();
    std::thread::spawn(|| run(COINIT_MULTITHREADED, COINIT_APARTMENTTHREADED))
        .join()
        .unwrap();
}
