use std::fs;
use std::path::Path;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(embedded_setup)");
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "installer_context",
            "check_install_dir",
            "browse_install_dir",
            "install",
            "launch_installed",
            "reveal_setup",
            "close_wizard",
        ]),
    ))
    .expect("failed to run tauri-build");

    println!("cargo:rerun-if-env-changed=TIEZ_SETUP_EXE");
    println!("cargo:rerun-if-changed=build.rs");

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR");
    let dest = Path::new(&out_dir).join("embedded-setup.exe");
    let source = std::env::var("TIEZ_SETUP_EXE").unwrap_or_default();
    let source = source.trim();
    if source.is_empty() {
        let _ = fs::remove_file(&dest);
        return;
    }

    println!("cargo:rerun-if-changed={source}");
    let bytes = fs::read(source).unwrap_or_else(|error| {
        panic!("TIEZ_SETUP_EXE={source} could not be read: {error}");
    });
    if bytes.len() < 1024 || bytes.first() != Some(&b'M') || bytes.get(1) != Some(&b'Z') {
        panic!(
            "TIEZ_SETUP_EXE={source} is not a Windows executable (expected an MZ header and at least 1024 bytes). Refusing to embed a placeholder."
        );
    }
    fs::write(&dest, &bytes).unwrap_or_else(|error| {
        panic!("failed to stage embedded setup: {error}");
    });
    println!("cargo:rustc-cfg=embedded_setup");
}
