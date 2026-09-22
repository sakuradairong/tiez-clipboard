//! Decisions the TieZ install wizard is allowed to make.
//!
//! The wizard embeds the existing Tauri NSIS setup and starts it itself.
//! The only flag it may pass is `/S`. When the user picks a directory other
//! than the current-user default, it appends one unquoted `/D=` argument and
//! that argument is last. `/P`, `/UPDATE`, `/R`, and `/NS` belong to the
//! in-app updater or to shortcut suppression and are rejected here.

/// Extra free space, on top of twice the embedded setup, before the wizard warns.
pub const DISK_HEADROOM_BYTES: u64 = 150 * 1024 * 1024;

/// NSIS documented exit codes (Appendix D). Not yet observed on this repo's setup.exe.
pub fn classify_exit(code: i32) -> ExitKind {
    match code {
        0 => ExitKind::Success,
        1 => ExitKind::UserCancelled,
        2 => ExitKind::ScriptAbort,
        other => ExitKind::Other(other),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitKind {
    Success,
    UserCancelled,
    ScriptAbort,
    Other(i32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SilentInstall {
    /// Everything after the setup executable. Pass this as one raw argument
    /// so `/D=` stays unquoted, including when the path contains spaces.
    pub raw_tail: String,
    pub passes_custom_dir: bool,
    pub normalized_dir: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgError {
    EmptyPath,
    NotAbsolute,
    QuotesOrControl,
    TooLong,
    ForbiddenFlag,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchError {
    MissingMainBinaryName,
    MissingInstallLocation,
    NotAbsolute,
    QuotesOrControl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedError {
    Empty,
    NotExecutable,
    TooSmall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskCheck {
    Ok,
    Low { free_bytes: u64, need_bytes: u64 },
    Unknown,
}

/// Flags the shell must never put on the NSIS command line.
pub const FORBIDDEN_SHELL_FLAGS: &[&str] = &["/P", "/UPDATE", "/R", "/NS"];

pub fn required_free_bytes(setup_bytes: u64) -> u64 {
    setup_bytes
        .saturating_mul(2)
        .saturating_add(DISK_HEADROOM_BYTES)
}

pub fn disk_space_check(free_bytes: Option<u64>, setup_bytes: u64) -> DiskCheck {
    let need_bytes = required_free_bytes(setup_bytes);
    match free_bytes {
        None => DiskCheck::Unknown,
        Some(free_bytes) if free_bytes < need_bytes => DiskCheck::Low {
            free_bytes,
            need_bytes,
        },
        Some(_) => DiskCheck::Ok,
    }
}

/// Reject anything that is not a plausible embedded NSIS executable.
pub fn validate_embedded_setup(bytes: &[u8]) -> Result<(), EmbedError> {
    if bytes.is_empty() {
        return Err(EmbedError::Empty);
    }
    if bytes.len() < 2 || bytes[0] != b'M' || bytes[1] != b'Z' {
        return Err(EmbedError::NotExecutable);
    }
    if bytes.len() < 1024 {
        return Err(EmbedError::TooSmall);
    }
    Ok(())
}

pub fn install_dirs_equivalent(left: &str, right: &str) -> bool {
    let left = normalize_windows_path(left);
    let right = normalize_windows_path(right);
    !left.is_empty() && left.eq_ignore_ascii_case(&right)
}

/// Build the silent first-install command tail.
///
/// `default_dir` is the current-user default (`%LOCALAPPDATA%\TieZ`). When the
/// selected directory matches it, `/D=` is omitted so NSIS can reuse the path
/// it remembered. A different absolute path appends `/D=` with no quotes.
pub fn build_silent_install(
    selected_dir: &str,
    default_dir: &str,
) -> Result<SilentInstall, ArgError> {
    let normalized_dir = validate_install_dir(selected_dir)?;
    let same = install_dirs_equivalent(&normalized_dir, default_dir);
    let raw_tail = if same {
        "/S".to_string()
    } else {
        format!("/S /D={normalized_dir}")
    };
    if tail_contains_forbidden_flag(&raw_tail) {
        return Err(ArgError::ForbiddenFlag);
    }
    Ok(SilentInstall {
        raw_tail,
        passes_custom_dir: !same,
        normalized_dir,
    })
}

pub fn validate_install_dir(path: &str) -> Result<String, ArgError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(ArgError::EmptyPath);
    }
    if trimmed.chars().count() > 240 {
        return Err(ArgError::TooLong);
    }
    if has_quotes_or_control(trimmed) {
        return Err(ArgError::QuotesOrControl);
    }
    if !is_absolute_windows(trimmed) {
        return Err(ArgError::NotAbsolute);
    }
    let normalized = normalize_windows_path(trimmed);
    if !is_absolute_windows(&normalized) || has_dot_segment(&normalized) {
        return Err(ArgError::NotAbsolute);
    }
    Ok(normalized)
}

/// Join `MainBinaryName` from the uninstall key to `InstallLocation`.
/// An empty name is an error. This does not substitute `TieZ.exe`.
pub fn resolve_launch_exe(
    main_binary_name: &str,
    install_location: &str,
) -> Result<String, LaunchError> {
    let name = main_binary_name.trim();
    if name.is_empty() {
        return Err(LaunchError::MissingMainBinaryName);
    }
    if has_quotes_or_control(name) || has_quotes_or_control(install_location) {
        return Err(LaunchError::QuotesOrControl);
    }
    if is_absolute_windows(name) {
        let normalized = normalize_windows_path(name);
        if has_dot_segment(&normalized) {
            return Err(LaunchError::NotAbsolute);
        }
        return Ok(normalized);
    }
    let file = name.replace('/', "\\");
    if file.contains(':') || file.starts_with('\\') || has_dot_segment(&file) {
        return Err(LaunchError::NotAbsolute);
    }
    let location = install_location.trim();
    if location.is_empty() {
        return Err(LaunchError::MissingInstallLocation);
    }
    if !is_absolute_windows(location) {
        return Err(LaunchError::NotAbsolute);
    }
    let location = normalize_windows_path(location);
    if has_dot_segment(&location) {
        return Err(LaunchError::NotAbsolute);
    }
    Ok(format!("{location}\\{file}"))
}

/// True when a token before `/D=` is an updater or shortcut flag.
pub fn tail_contains_forbidden_flag(tail: &str) -> bool {
    let head = match split_install_dir_switch(tail) {
        Some((head, _)) => head,
        None => tail,
    };
    head.split_whitespace().any(is_forbidden_flag)
}

pub fn is_forbidden_flag(token: &str) -> bool {
    FORBIDDEN_SHELL_FLAGS.iter().any(|flag| token == *flag)
}

fn split_install_dir_switch(tail: &str) -> Option<(&str, &str)> {
    if let Some(rest) = tail.strip_prefix("/D=") {
        return Some(("", rest));
    }
    let marker = " /D=";
    let index = tail.find(marker)?;
    Some((&tail[..index], &tail[index + marker.len()..]))
}

fn has_quotes_or_control(value: &str) -> bool {
    value
        .chars()
        .any(|ch| ch == '"' || ch == '\n' || ch == '\r' || ch == '\0' || ch == '\t')
}

fn has_dot_segment(path: &str) -> bool {
    path.split('\\')
        .any(|segment| segment == "." || segment == "..")
}

pub fn is_absolute_windows(path: &str) -> bool {
    let bytes = path.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        return true;
    }
    path.starts_with("\\\\") && path.chars().filter(|ch| *ch == '\\').count() >= 3
}

pub fn normalize_windows_path(path: &str) -> String {
    let replaced = path.trim().replace('/', "\\");
    let mut out = String::with_capacity(replaced.len());
    let mut chars = replaced.chars().peekable();
    if replaced.starts_with("\\\\") {
        out.push('\\');
        out.push('\\');
        chars.next();
        chars.next();
    }
    let mut previous_slash = false;
    for ch in chars {
        if ch == '\\' {
            if previous_slash {
                continue;
            }
            previous_slash = true;
            out.push(ch);
        } else {
            previous_slash = false;
            out.push(ch);
        }
    }
    let is_drive_root = out.len() == 3 && out.as_bytes()[1] == b':' && out.ends_with('\\');
    if out.ends_with('\\') && !is_drive_root && out.len() > 1 {
        out.pop();
    }
    out
}

/// Registry locations of the Evergreen WebView2 runtime.
pub fn webview2_client_key() -> &'static str {
    r"Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"
}

pub fn webview2_client_wow64_key() -> &'static str {
    r"SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"
}

/// Uninstall key written by the inner NSIS setup. Not the remembered-path key.
pub const UNINSTALL_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\TieZ";

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT: &str = r"C:\Users\me\AppData\Local\TieZ";

    fn assert_shell_policy(tail: &str) {
        assert!(
            tail == "/S" || tail.starts_with("/S /D="),
            "unexpected tail: {tail}"
        );
        assert!(
            !tail_contains_forbidden_flag(tail),
            "forbidden flag in {tail}"
        );
        assert!(!tail.contains("/P"));
        assert!(!tail.contains("/UPDATE"));
        assert!(!tail.contains(" /R"));
        assert!(!tail.contains("/NS"));
        assert!(!tail.contains('"'), "tail must not quote /D=: {tail}");
        if let Some(dir) = tail.strip_prefix("/S /D=") {
            assert!(!dir.is_empty());
            assert!(is_absolute_windows(dir));
            let rest = tail.strip_prefix("/S ").unwrap();
            assert!(rest.starts_with("/D="));
            assert_eq!(tail.matches("/D=").count(), 1);
        } else {
            assert_eq!(tail, "/S");
        }
    }

    #[test]
    fn default_directory_is_silent_only() {
        let plan = build_silent_install(DEFAULT, DEFAULT).unwrap();
        assert_eq!(plan.raw_tail, "/S");
        assert!(!plan.passes_custom_dir);
        assert_shell_policy(&plan.raw_tail);
    }

    #[test]
    fn equivalent_paths_do_not_pass_d() {
        let plan = build_silent_install(r"c:/users/me/appdata/local/tiez/", DEFAULT).unwrap();
        assert_eq!(plan.raw_tail, "/S");
        assert!(!plan.passes_custom_dir);
        assert!(plan.normalized_dir.eq_ignore_ascii_case(DEFAULT));
    }

    #[test]
    fn custom_directory_appends_unquoted_d_last() {
        let plan = build_silent_install(r"C:\Program Files\TieZ", DEFAULT).unwrap();
        assert_eq!(plan.raw_tail, r"/S /D=C:\Program Files\TieZ");
        assert!(plan.passes_custom_dir);
        assert_shell_policy(&plan.raw_tail);
        assert!(plan.raw_tail.ends_with(r"C:\Program Files\TieZ"));
    }

    #[test]
    fn slash_style_custom_directory_is_normalized() {
        let plan = build_silent_install(r"D:/Tools/TieZ/", DEFAULT).unwrap();
        assert_eq!(plan.raw_tail, r"/S /D=D:\Tools\TieZ");
        assert_shell_policy(&plan.raw_tail);
    }

    #[test]
    fn path_containing_flag_letters_is_still_only_s_and_d() {
        let plan = build_silent_install(r"C:\P\UPDATE\NS\TieZ", DEFAULT).unwrap();
        assert_eq!(plan.raw_tail, r"/S /D=C:\P\UPDATE\NS\TieZ");
        assert!(!tail_contains_forbidden_flag(&plan.raw_tail));
        assert_shell_policy(&plan.raw_tail);
    }

    #[test]
    fn rejects_relative_quoted_and_parent_paths() {
        assert_eq!(
            build_silent_install(r"TieZ", DEFAULT).unwrap_err(),
            ArgError::NotAbsolute
        );
        assert_eq!(
            build_silent_install(r"C:\TieZ\..\Other", DEFAULT).unwrap_err(),
            ArgError::NotAbsolute
        );
        assert_eq!(
            build_silent_install("\"C:\\TieZ\"", DEFAULT).unwrap_err(),
            ArgError::QuotesOrControl
        );
        assert_eq!(
            build_silent_install("   ", DEFAULT).unwrap_err(),
            ArgError::EmptyPath
        );
    }

    #[test]
    fn rejects_a_smuggled_flag_token_before_d() {
        assert!(is_forbidden_flag("/P"));
        assert!(is_forbidden_flag("/UPDATE"));
        assert!(is_forbidden_flag("/R"));
        assert!(is_forbidden_flag("/NS"));
        assert!(!is_forbidden_flag("/S"));
        assert!(!is_forbidden_flag("/D=C:\\TieZ"));
        assert!(tail_contains_forbidden_flag("/S /P /D=C:\\TieZ"));
        assert!(tail_contains_forbidden_flag("/S /R"));
        assert!(tail_contains_forbidden_flag("/P /R /UPDATE"));
        assert!(!tail_contains_forbidden_flag("/S"));
        assert!(!tail_contains_forbidden_flag(
            r"/S /D=C:\Program Files\TieZ"
        ));
    }

    #[test]
    fn launch_reads_main_binary_name_and_does_not_invent_tiez_exe() {
        let exe = resolve_launch_exe("tiez-app.exe", r"C:\Users\me\AppData\Local\TieZ").unwrap();
        assert_eq!(exe, r"C:\Users\me\AppData\Local\TieZ\tiez-app.exe");

        let renamed = resolve_launch_exe("TieZ.exe", r"D:\Apps\TieZ").unwrap();
        assert_eq!(renamed, r"D:\Apps\TieZ\TieZ.exe");

        let absolute = resolve_launch_exe(r"E:\Bin\tiez-app.exe", r"C:\ignored").unwrap();
        assert_eq!(absolute, r"E:\Bin\tiez-app.exe");

        assert_eq!(
            resolve_launch_exe("", r"C:\Users\me\AppData\Local\TieZ").unwrap_err(),
            LaunchError::MissingMainBinaryName
        );
        assert_eq!(
            resolve_launch_exe("tiez-app.exe", "").unwrap_err(),
            LaunchError::MissingInstallLocation
        );
        assert_ne!(
            resolve_launch_exe("", r"C:\TieZ").unwrap_err(),
            LaunchError::NotAbsolute
        );
    }

    #[test]
    fn launch_rejects_parent_segments() {
        assert_eq!(
            resolve_launch_exe(r"..\tiez-app.exe", r"C:\TieZ").unwrap_err(),
            LaunchError::NotAbsolute
        );
    }

    #[test]
    fn exit_codes_follow_nsis_documentation() {
        assert_eq!(classify_exit(0), ExitKind::Success);
        assert_eq!(classify_exit(1), ExitKind::UserCancelled);
        assert_eq!(classify_exit(2), ExitKind::ScriptAbort);
        assert_eq!(classify_exit(5), ExitKind::Other(5));
    }

    #[test]
    fn disk_check_warns_below_headroom() {
        assert_eq!(disk_space_check(None, 10), DiskCheck::Unknown);
        assert!(matches!(
            disk_space_check(Some(1024), 10),
            DiskCheck::Low { .. }
        ));
        let plenty = required_free_bytes(1024) + 1;
        assert_eq!(disk_space_check(Some(plenty), 1024), DiskCheck::Ok);
    }

    #[test]
    fn embedded_setup_must_look_like_an_executable() {
        assert_eq!(validate_embedded_setup(&[]).unwrap_err(), EmbedError::Empty);
        assert_eq!(
            validate_embedded_setup(b"Not an exe").unwrap_err(),
            EmbedError::NotExecutable
        );
        let mut tiny = vec![b'M', b'Z'];
        tiny.resize(64, 0);
        assert_eq!(
            validate_embedded_setup(&tiny).unwrap_err(),
            EmbedError::TooSmall
        );
        let mut ok = vec![b'M', b'Z'];
        ok.resize(1024, 0);
        assert!(validate_embedded_setup(&ok).is_ok());
    }

    #[test]
    fn webview2_and_uninstall_keys_are_stable() {
        assert!(webview2_client_key().contains("{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"));
        assert!(webview2_client_wow64_key().contains("WOW6432Node"));
        assert!(UNINSTALL_SUBKEY.ends_with(r"\TieZ"));
        assert!(!UNINSTALL_SUBKEY.contains("贴汁"));
    }
}
