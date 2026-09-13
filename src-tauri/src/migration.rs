use std::fs;
use std::path::{Path, PathBuf};

/// 数据目录重定向文件：内容为数据目录的绝对路径（无换行）。
const DATA_REDIRECT_FILE: &str = "datapath.txt";

/// 旧应用标识目录名（commit 972cb86 将 identifier 从 com.tiez.app 改为 com.tiez，
/// 导致默认数据目录从 %APPDATA%\com.tiez.app 变为 %APPDATA%\com.tiez）。
pub const LEGACY_IDENTIFIER_DIR_NAME: &str = "com.tiez.app";

/// 旧标识数据目录的评估结果。
#[derive(Debug, PartialEq, Eq)]
pub enum LegacyDataResolution {
    /// 没有可采用的旧标识数据。
    None,
    /// 默认目录尚未初始化，可无歧义地原地采用旧目录。
    Adopt { legacy_dir: PathBuf },
    /// 两侧都有数据库，不能静默选择，需用户决定。
    Ambiguous { legacy_dir: PathBuf },
}

/// 评估旧应用标识（com.tiez.app）数据目录应如何处理。
///
/// 判定规则：
/// - 按旧版运行时语义解析旧标识目录：其 datapath.txt 有效（目标存在）时，
///   实际数据目录是重定向目标；无效/为空时回落到旧目录本身；
/// - 解析出的数据目录没有 clipboard.db 视为无数据，返回 `None`；
/// - 默认目录也没有 clipboard.db（未初始化）时返回 `Adopt`；
/// - 两边都有 clipboard.db 时返回 `Ambiguous`，绝不按体积或修改时间取舍。
pub fn evaluate_legacy_identifier_data(default_dir: &Path) -> LegacyDataResolution {
    let Some(parent) = default_dir.parent() else {
        return LegacyDataResolution::None;
    };
    let legacy_config_dir = parent.join(LEGACY_IDENTIFIER_DIR_NAME);
    let Some(legacy_dir) = resolve_legacy_data_dir(&legacy_config_dir) else {
        return LegacyDataResolution::None;
    };
    if legacy_dir == default_dir || !legacy_dir.join("clipboard.db").is_file() {
        return LegacyDataResolution::None;
    }
    if default_dir.join("clipboard.db").is_file() {
        LegacyDataResolution::Ambiguous { legacy_dir }
    } else {
        LegacyDataResolution::Adopt { legacy_dir }
    }
}

/// 按旧版运行时语义解析旧标识目录的实际数据目录。
///
/// 优先返回带有数据库的重定向目标（旧版运行时使用该目录）；目标没有数据库
/// 时回落到旧目录本身的数据库（设置重定向之前的历史数据）。两者都没有
/// 数据库时返回 `None`。
fn resolve_legacy_data_dir(legacy_config_dir: &Path) -> Option<PathBuf> {
    if !legacy_config_dir.is_dir() {
        return None;
    }
    let redirect = legacy_config_dir.join(DATA_REDIRECT_FILE);
    if redirect.is_file() {
        if let Ok(content) = fs::read_to_string(&redirect) {
            let target = content.trim();
            if !target.is_empty() {
                let target_path = PathBuf::from(target);
                // 无效重定向：旧版运行时回落到旧目录本身，这里同样回落。
                if target_path.join("clipboard.db").is_file() {
                    return Some(target_path);
                }
            }
        }
    }
    if legacy_config_dir.join("clipboard.db").is_file() {
        Some(legacy_config_dir.to_path_buf())
    } else {
        None
    }
}

/// 以写入 datapath.txt 重定向的方式原地采用 `legacy_dir` 作为数据目录。
///
/// 不复制、不移动、不删除任何文件，因此 WAL/SHM、附件及数据库内的绝对路径
/// 全部保持一致。失败时原样返回错误，两个目录都不会被改动。
pub fn write_data_redirect(default_dir: &Path, target_dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(default_dir)?;
    std::fs::write(
        default_dir.join(DATA_REDIRECT_FILE),
        target_dir.to_string_lossy().to_string(),
    )
}

/// `resolve_data_dir` 的纯决策结果。
#[derive(Debug, PartialEq, Eq)]
pub struct DataDirChoice {
    /// 本次启动使用的数据目录。
    pub dir: PathBuf,
    /// 默认目录既无 datapath.txt（含无效内容）也未激活便携模式时为 true，
    /// 此时才允许评估/采用旧应用标识数据目录。
    pub legacy_allowed: bool,
}

/// 数据目录选择的决策逻辑（无副作用，便于测试）：
/// 便携目录 > 有效重定向 > 默认目录；任何已存在的 datapath.txt（即使指向
/// 不存在的路径）都视为用户显式配置，禁止再自动采用旧标识目录。
pub fn choose_data_dir(
    default_dir: &Path,
    redirect_file_exists: bool,
    redirect_content: Option<&str>,
    portable: Option<&Path>,
) -> DataDirChoice {
    let custom = redirect_content
        .map(str::trim)
        .filter(|content| !content.is_empty() && Path::new(content).exists());
    let dir = match (portable, custom) {
        (Some(portable_dir), _) => portable_dir.to_path_buf(),
        (None, Some(content)) => PathBuf::from(content),
        (None, None) => default_dir.to_path_buf(),
    };
    DataDirChoice {
        dir,
        legacy_allowed: !redirect_file_exists && portable.is_none(),
    }
}

/// 应用“采用旧标识目录”的决定：写入 datapath.txt 并返回旧目录；
/// 写入失败时返回默认目录，旧数据保持原样（下次启动会再次尝试）。
pub fn adopt_legacy_dir(default_dir: &Path, legacy_dir: &Path) -> PathBuf {
    match write_data_redirect(default_dir, legacy_dir) {
        Ok(()) => legacy_dir.to_path_buf(),
        Err(error) => {
            println!(
                ">>> [MIGRATION] Could not write datapath.txt redirect to adopt {:?} ({}); falling back to the default dir.",
                legacy_dir, error
            );
            default_dir.to_path_buf()
        }
    }
}

/// 启动期迁移上下文：描述“贴汁”/旧标识迁移执行前已存在的显式配置。
/// 存在任一显式配置时，迁移不得覆盖或改写它。
#[derive(Debug, Default, Clone)]
pub struct MigrationContext {
    /// 默认目录是否已存在 datapath.txt（内容无效也算显式配置）。
    pub redirect_file_exists: bool,
    /// 当前 datapath.txt 的有效目标目录。
    pub redirect_target: Option<PathBuf>,
    /// 便携模式目录（可执行文件旁的 data/）。
    pub portable_dir: Option<PathBuf>,
}

impl MigrationContext {
    /// 是否存在用户显式配置（重定向文件或便携模式）。
    pub fn has_explicit_config(&self) -> bool {
        self.redirect_file_exists || self.portable_dir.is_some()
    }
}

/// 启动数据目录解析结果。
#[derive(Debug)]
pub struct StartupDirResolution {
    /// 本次启动使用的数据目录。
    pub dir: PathBuf,
    /// 解析开始时的显式配置快照。
    pub ctx: MigrationContext,
    /// 旧安装目录清理必须跳过的受保护目录（默认目录、显式配置目标、
    /// 最终数据目录及其祖先均不得删除）。
    pub protected_dirs: Vec<PathBuf>,
}

/// 实际启动顺序的编排（无系统副作用，便于测试）：
/// 读取当前显式配置 → 贴汁数据迁移（尊重显式配置）→ 重新读取重定向 →
/// 目录选择 → 旧标识目录兼容（含歧义确认）。
///
/// `tiezhi_dirs` 为待检查的旧“贴汁”数据目录（生产环境由
/// `discover_old_tiezhi_dirs` 从环境变量发现，测试中注入临时目录）。
/// `confirm_use_legacy` 在两侧数据库并存时调用（参数为旧数据目录与默认目录），
/// 返回是否采用旧数据；生产环境为原生“是/否”对话框，测试中注入。
pub fn resolve_startup_data_dir(
    default_dir: &Path,
    portable_dir: Option<&Path>,
    tiezhi_dirs: &[PathBuf],
    confirm_use_legacy: &dyn Fn(&Path, &Path) -> bool,
) -> StartupDirResolution {
    // 1. 先读取当前显式配置，迁移必须尊重它（不得覆盖已有重定向）。
    let ctx = {
        let redirect_file_exists = default_dir.join(DATA_REDIRECT_FILE).is_file();
        let redirect_target = if redirect_file_exists {
            std::fs::read_to_string(default_dir.join(DATA_REDIRECT_FILE))
                .ok()
                .map(|content| content.trim().to_string())
                .filter(|content| !content.is_empty() && Path::new(content).exists())
                .map(PathBuf::from)
        } else {
            None
        };
        MigrationContext {
            redirect_file_exists,
            redirect_target,
            portable_dir: portable_dir.map(|p| p.to_path_buf()),
        }
    };

    // 2. 贴汁数据迁移（仅在无显式配置时改写数据/重定向）。
    migrate_old_tiezhi_data(tiezhi_dirs, default_dir, &ctx);

    // 3. 迁移可能刚写入重定向（仅当此前不存在），重新读取后选择目录。
    let redirect_file_exists = default_dir.join(DATA_REDIRECT_FILE).is_file();
    let redirect_content = std::fs::read_to_string(default_dir.join(DATA_REDIRECT_FILE)).ok();
    let choice = choose_data_dir(
        default_dir,
        redirect_file_exists,
        redirect_content.as_deref(),
        portable_dir,
    );
    let mut dir = choice.dir;

    // 4. 旧标识目录兼容：仅在无显式配置时评估。
    if choice.legacy_allowed {
        match evaluate_legacy_identifier_data(default_dir) {
            LegacyDataResolution::None => {}
            LegacyDataResolution::Adopt { legacy_dir } => {
                println!(
                    ">>> [MIGRATION] Found legacy identifier data dir {:?}; adopting in place via datapath.txt.",
                    legacy_dir
                );
                dir = adopt_legacy_dir(default_dir, &legacy_dir);
            }
            LegacyDataResolution::Ambiguous { legacy_dir } => {
                if confirm_use_legacy(&legacy_dir, default_dir) {
                    println!(
                        ">>> [MIGRATION] User chose to use legacy data dir {:?}; adopting in place.",
                        legacy_dir
                    );
                    dir = adopt_legacy_dir(default_dir, &legacy_dir);
                } else {
                    println!(
                        ">>> [MIGRATION] User kept the current data dir; legacy data left untouched at {:?}.",
                        legacy_dir
                    );
                }
            }
        }
    }

    let mut protected_dirs = vec![default_dir.to_path_buf()];
    if let Some(target) = &ctx.redirect_target {
        protected_dirs.push(target.clone());
    }
    if let Some(portable) = portable_dir {
        protected_dirs.push(portable.to_path_buf());
    }
    if !protected_dirs.contains(&dir) {
        protected_dirs.push(dir.clone());
    }

    StartupDirResolution {
        dir,
        ctx,
        protected_dirs,
    }
}

/// 发现可能的旧“贴汁”数据目录（生产环境使用）。
pub fn discover_old_tiezhi_dirs(default_app_dir: &Path) -> Vec<PathBuf> {
    // Check parent of current app dir (AppData\Roaming or AppData\Local)
    let mut old_app_dirs_to_check = Vec::new();
    if let Some(parent) = default_app_dir.parent() {
        old_app_dirs_to_check.push(parent.join("贴汁"));
    }

    // Also check AppData\Local explicitly
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        old_app_dirs_to_check.push(std::path::PathBuf::from(&local_app_data).join("贴汁"));
    }

    // Also check AppData\Roaming explicitly
    if let Ok(roaming_app_data) = std::env::var("APPDATA") {
        old_app_dirs_to_check.push(std::path::PathBuf::from(&roaming_app_data).join("贴汁"));
    }

    old_app_dirs_to_check
}

/// 每次启动清理旧“贴汁”安装残留（注册表、开始菜单/桌面快捷方式、安装目录）。
/// `protected_dirs` 内的目录及其祖先包含的目录一律不删除。
pub fn cleanup_old_install_residues(protected_dirs: &[PathBuf]) {
    let custom_path = cleanup_old_install_registry();
    cleanup_old_start_menu();
    cleanup_old_install_folder(custom_path, protected_dirs);
}

/// 贴汁目录数据迁移。所有分支都保证非破坏性：不按体积覆盖数据库、
/// 不删除旧目录；无法判断时保留双方数据并提示恢复方式。
/// 已存在显式配置（datapath.txt 或便携模式）时直接跳过，绝不覆盖当前配置。
fn migrate_old_tiezhi_data(old_app_dirs: &[PathBuf], default_app_dir: &Path, ctx: &MigrationContext) {
    for old_app_dir in old_app_dirs {
        if !(old_app_dir.exists() && old_app_dir.is_dir()) {
            continue;
        }
        println!(
            ">>> [MIGRATION] Found old data folder at: {:?}",
            old_app_dir
        );
        if ctx.has_explicit_config() {
            println!(
                ">>> [MIGRATION] Explicit data path config present (datapath.txt or portable mode); leaving '贴汁' data untouched."
            );
            continue;
        }
        let new_db = default_app_dir.join("clipboard.db");
        let old_db = old_app_dir.join("clipboard.db");

        // 1. 用户在旧目录配置过自定义数据路径（datapath.txt）：仅迁移该配置，
        //    旧目录其余内容保持原样。
        let old_redirect = old_app_dir.join(DATA_REDIRECT_FILE);
        let mut redirect_migrated = false;
        if old_redirect.exists() {
            println!(">>> [MIGRATION] Found custom data path configuration. Migrating...");
            let _ = std::fs::create_dir_all(default_app_dir);
            if std::fs::copy(&old_redirect, default_app_dir.join(DATA_REDIRECT_FILE)).is_ok() {
                redirect_migrated = true;
                println!(">>> [MIGRATION] Migrated datapath.txt successfully.");
            }
        }

        // 2. Data Migration Logic
        if redirect_migrated {
            // 用户显式配置优先，不再改动任何数据。
        } else if !default_app_dir.exists() {
            // 整目录重命名，连同 WAL/SHM、附件一起保留。
            println!(">>> [MIGRATION] Renaming old data folder '贴汁' to 'TieZ'...");
            if std::fs::rename(old_app_dir, default_app_dir).is_err() {
                println!(">>> [MIGRATION ERROR] Rename failed; leaving old data untouched.");
            }
        } else if old_db.exists() && !new_db.exists() {
            // 原地采用：写入 datapath.txt 指向旧目录，不复制数据库（避免丢 WAL、
            // 破坏附件绝对路径），也不删除旧目录。
            println!(">>> [MIGRATION] Adopting old '贴汁' data in place via datapath.txt redirect...");
            match write_data_redirect(default_app_dir, old_app_dir) {
                Ok(()) => println!(">>> [MIGRATION] Redirect written; old data adopted in place."),
                Err(error) => println!(
                    ">>> [MIGRATION ERROR] Could not write redirect ({}); both directories left untouched.",
                    error
                ),
            }
        } else if old_db.exists() && new_db.exists() {
            // 两边都有数据库：不做任何覆盖或删除，提示手动恢复。
            println!(
                ">>> [MIGRATION] Both '贴汁' and the current data directory contain databases; keeping both untouched. To use the old data, point {} at {:?} or change the data directory in settings.",
                DATA_REDIRECT_FILE, old_app_dir
            );
        } else {
            println!(">>> [MIGRATION] Old folder has no database; leaving it untouched.");
        }
    }
}

/// v0.2.8 Rename Migration: Registry Cleanup - Returns found install location if any
pub fn cleanup_old_install_registry() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        use winreg::enums::*;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let path = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall";
        let mut found_install_loc = None;

        println!(
            ">>> [CLEANUP] Scanning registry for old versions at HKCU\\{}",
            path
        );

        if let Ok(key) = hkcu.open_subkey_with_flags(path, KEY_READ | KEY_WRITE) {
            for subkey_name in key.enum_keys().filter_map(|x| x.ok()) {
                if let Ok(subkey) = key.open_subkey(&subkey_name) {
                    let name: String = subkey.get_value("DisplayName").unwrap_or_default();

                    if name.contains("贴汁") {
                        println!(
                            ">>> [CLEANUP] Found old registry entry: {} ({}).",
                            subkey_name, name
                        );

                        // Try to get InstallLocation
                        if let Ok(loc) = subkey.get_value::<String, _>("InstallLocation") {
                            if !loc.is_empty() {
                                println!(
                                    ">>> [CLEANUP] Found InstallLocation in registry: {}",
                                    loc
                                );
                                found_install_loc = Some(PathBuf::from(loc));
                            }
                        }
                        // Fallback: Try to parse from UninstallString "C:\path\to\uninstall.exe"
                        if found_install_loc.is_none() {
                            if let Ok(uninstall_str) =
                                subkey.get_value::<String, _>("UninstallString")
                            {
                                println!(">>> [CLEANUP] Found UninstallString: {}", uninstall_str);
                                // Simple heuristic: remove quotes and find parent of executable
                                let clean_str = uninstall_str.replace("\"", "");
                                let p = std::path::Path::new(&clean_str);
                                if let Some(parent) = p.parent() {
                                    println!(">>> [CLEANUP] inferred install path from uninstaller: {:?}", parent);
                                    found_install_loc = Some(parent.to_path_buf());
                                }
                            }
                        }

                        println!(">>> [CLEANUP] Deleting registry key...");
                        if let Err(e) = key.delete_subkey_all(&subkey_name) {
                            println!(">>> [CLEANUP ERROR] Failed to delete registry key: {}", e);
                        } else {
                            println!(">>> [CLEANUP] Registry entry deleted.");
                        }
                    }
                }
            }
        }
        return found_install_loc;
    }
    #[cfg(not(windows))]
    None
}

/// v0.2.8 Rename Migration: Start Menu & Desktop Cleanup
pub fn cleanup_old_start_menu() {
    #[cfg(windows)]
    {
        if let Ok(app_data) = std::env::var("APPDATA") {
            let start_menu =
                std::path::Path::new(&app_data).join("Microsoft\\Windows\\Start Menu\\Programs");
            println!(">>> [CLEANUP] Checking Start Menu at: {:?}", start_menu);

            // Delete old shortcut
            let old_lnk = start_menu.join("贴汁.lnk");
            if old_lnk.exists() {
                println!(
                    ">>> [CLEANUP] Deleting old start menu shortcut: {:?}",
                    old_lnk
                );
                let _ = fs::remove_file(old_lnk);
            }

            // Delete old start menu folder
            let old_folder = start_menu.join("贴汁");
            if old_folder.exists() && old_folder.is_dir() {
                println!(
                    ">>> [CLEANUP] Deleting old start menu folder: {:?}",
                    old_folder
                );
                let _ = fs::remove_dir_all(old_folder);
            }
        }

        // Desktop Cleanup
        if let Ok(user_profile) = std::env::var("USERPROFILE") {
            let desktop = std::path::Path::new(&user_profile).join("Desktop");
            let old_desktop_lnk = desktop.join("贴汁.lnk");
            println!(
                ">>> [CLEANUP] Checking Desktop shortcut at: {:?}",
                old_desktop_lnk
            );
            if old_desktop_lnk.exists() {
                println!(
                    ">>> [CLEANUP] Deleting old desktop shortcut: {:?}",
                    old_desktop_lnk
                );
                let _ = fs::remove_file(old_desktop_lnk);
            }
        }
    }
}

/// Helper function to manually enable autostart via registry
pub fn enable_autostart_manually(app_name: &str, exe_path: &str) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use winreg::enums::*;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let path = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
        let (key, _) = hkcu.create_subkey(path)?;
        key.set_value(app_name, &exe_path)?;
        Ok(())
    }
    #[cfg(not(windows))]
    Ok(())
}

/// v0.2.8 Rename Migration: Clean up old installation directory
pub fn cleanup_old_install_folder(custom_path: Option<PathBuf>, protected_dirs: &[PathBuf]) {
    #[cfg(windows)]
    {
        // Try to find and delete old installation folder
        // Common installation paths
        let mut possible_paths = vec![
            std::env::var("LOCALAPPDATA")
                .ok()
                .map(|p| PathBuf::from(p).join("Programs").join("贴汁")),
            std::env::var("ProgramFiles")
                .ok()
                .map(|p| PathBuf::from(p).join("贴汁")),
            std::env::var("ProgramFiles(x86)")
                .ok()
                .map(|p| PathBuf::from(p).join("贴汁")),
            // Also check direct local appdata just in case
            std::env::var("LOCALAPPDATA")
                .ok()
                .map(|p| PathBuf::from(p).join("贴汁")),
        ];

        // Add custom path from registry if found
        if let Some(path) = custom_path {
            println!(
                ">>> [CLEANUP] Adding custom path from registry to cleanup list: {:?}",
                path
            );
            possible_paths.push(Some(path));
        }

        cleanup_install_folders_at(possible_paths, protected_dirs);
    }
}

/// 目录是否包含用户数据（数据库、重定向或附件）。便携模式的 `data/` 子目录
/// 也按同一规则检查，避免把安装目录内的便携历史当残留删除。
#[cfg(windows)]
fn dir_contains_user_data(dir: &Path) -> bool {
    dir.join("clipboard.db").is_file()
        || dir.join(DATA_REDIRECT_FILE).is_file()
        || dir.join("attachments").is_dir()
}

/// 解析为可比较的真实绝对路径：消除 8.3 短路径名、`..`、`\\?\` 扩展
/// 前缀、符号链接与目录联接等 Windows 路径别名。
/// 解析失败（不存在、访问失败、循环链接等）返回 `None`；调用方必须
/// 跳过删除，不得回退到不可靠的字符串比较。
#[cfg(windows)]
fn canonicalize_for_compare(path: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(path).ok()
}

/// `child` 是否位于 `ancestor` 内（含相等）。输入必须是
/// `canonicalize_for_compare` 解析后的真实路径；仍按大小写不敏感比较，
/// 以容忍不同来源的大小写拼写。
#[cfg(windows)]
fn path_is_within_ci(child: &Path, ancestor: &Path) -> bool {
    let normalize = |p: &Path| -> Vec<String> {
        p.components()
            .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
            .collect()
    };
    let child_parts = normalize(child);
    let ancestor_parts = normalize(ancestor);
    child_parts.len() >= ancestor_parts.len()
        && child_parts[..ancestor_parts.len()] == ancestor_parts[..]
}

/// 清理前置护栏：预先解析受保护目录与运行程序目录为真实路径。
#[cfg(windows)]
struct CleanupGuard {
    resolved_protected: Vec<PathBuf>,
    resolved_exe_dir: PathBuf,
}

/// 构建清理护栏。返回 `None` 表示无法确认安全，调用方必须放弃目录清理：
/// - `exe_dir` 为 `None`（运行程序目录不可得）；
/// - 运行程序目录无法解析；
/// - 受保护目录**存在性无法判定**（权限不足、元数据错误等——注意不能用
///   `exists()`，它把这类错误当作不存在而丢弃保护）；
/// - 受保护目录存在但无法解析为真实路径。
///
/// 只有 `try_exists()` 明确返回不存在时才忽略该受保护目录。
#[cfg(windows)]
fn prepare_cleanup_guard(protected_dirs: &[PathBuf], exe_dir: Option<&Path>) -> Option<CleanupGuard> {
    let exe_dir = exe_dir?;
    let resolved_exe_dir = canonicalize_for_compare(exe_dir)?;
    let mut resolved_protected = Vec::new();
    for protected in protected_dirs {
        match protected.try_exists() {
            // 明确不存在：不会因删除候选而丢失。
            Ok(false) => continue,
            Ok(true) => {}
            Err(error) => {
                println!(
                    ">>> [CLEANUP] Existence of protected dir {:?} could not be determined ({}); skipping install folder cleanup entirely.",
                    protected, error
                );
                return None;
            }
        }
        match canonicalize_for_compare(protected) {
            Some(resolved) => resolved_protected.push(resolved),
            None => {
                println!(
                    ">>> [CLEANUP] Protected dir {:?} could not be resolved; skipping install folder cleanup entirely.",
                    protected
                );
                return None;
            }
        }
    }
    Some(CleanupGuard {
        resolved_protected,
        resolved_exe_dir,
    })
}

/// 删除候选的旧安装目录，带数据安全护栏：
/// - 候选与受保护目录均先解析为真实路径（消除短路径、`..`、扩展前缀、
///   目录联接等别名）后比较；任何一方无法解析即跳过（fail closed）；
/// - 候选本身是重解析点（符号链接/目录联接）时跳过，避免删除语义差异
///   波及链接目标；
/// - 受保护目录（当前数据目录、显式重定向目标、便携目录）本身或其任何
///   祖先目录、运行中程序目录及其祖先一律跳过；运行程序目录不可得、
///   受保护目录存在性无法判定时整体放弃清理；
/// - 目录本身或其 `data/` 子目录（便携模式）含用户数据时跳过。
#[cfg(windows)]
fn cleanup_install_folders_at(possible_paths: Vec<Option<PathBuf>>, protected_dirs: &[PathBuf]) {
    // 运行程序目录不可得（current_exe 失败或无父目录）时无法确认任何
    // 候选的安全性，整体放弃目录清理。
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.to_path_buf()));
    let Some(exe_dir) = exe_dir else {
        println!(
            ">>> [CLEANUP] Running program directory unavailable; skipping install folder cleanup entirely."
        );
        return;
    };
    let Some(guard) = prepare_cleanup_guard(protected_dirs, Some(&exe_dir)) else {
        return;
    };

    for path_opt in possible_paths.iter() {
        let Some(path) = path_opt else {
            continue;
        };
        println!(">>> [CLEANUP] Checking installation path: {:?}", path);
        if !(path.exists() && path.is_dir()) {
            continue;
        }

        // 重解析点（符号链接/目录联接）不作为删除目标：不同实现的删除
        // 语义不一致，且元数据不可用时同样无法确认安全。
        let is_reparse_point = fs::symlink_metadata(path)
            .map(|meta| meta.file_type().is_symlink())
            .unwrap_or(true);
        if is_reparse_point {
            println!(
                ">>> [CLEANUP] Skipping {:?}: it is a symlink/junction or its metadata is unavailable.",
                path
            );
            continue;
        }

        // 解析候选真实路径；失败即跳过，不回退字符串比较。
        let Some(resolved_path) = canonicalize_for_compare(path) else {
            println!(
                ">>> [CLEANUP] Skipping {:?}: could not resolve the real path.",
                path
            );
            continue;
        };

        // Safety check: never delete the running program's directory or any
        // of its ancestors.
        if path_is_within_ci(&guard.resolved_exe_dir, &resolved_path) {
            println!(
                ">>> [CLEANUP] Skipping {:?}: it is or contains the running program directory.",
                path
            );
            continue;
        }

        // Safety check: never delete a directory that is (an ancestor
        // of) a protected data directory.
        if guard
            .resolved_protected
            .iter()
            .any(|protected| path_is_within_ci(protected, &resolved_path))
        {
            println!(
                ">>> [CLEANUP] Skipping {:?}: it is or contains a protected data directory.",
                path
            );
            continue;
        }

        // Safety check: never treat a directory holding user data
        // (including portable data/ inside it) as an installation
        // leftover, even if the registry pointed here.
        if dir_contains_user_data(path) || dir_contains_user_data(&path.join("data")) {
            println!(
                ">>> [CLEANUP] Skipping {:?}: contains user data (database, redirect, attachments, or portable data).",
                path
            );
            continue;
        }

        println!(">>> [CLEANUP] Found old installation folder: {:?}", path);
        // Try to delete - this might fail if files are in use
        match fs::remove_dir_all(path) {
            Ok(_) => println!(">>> [CLEANUP] Successfully deleted old installation folder"),
            Err(e) => println!(
                ">>> [CLEANUP] Could not delete old installation folder: {}",
                e
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_SEQ: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tiez_migration_test_{}_{}_{}",
            tag,
            std::process::id(),
            TEST_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 兄弟目录布局：parent/default 与 parent/com.tiez.app。
    fn legacy_layout(tag: &str) -> (PathBuf, PathBuf) {
        let parent = temp_dir(tag);
        let default = parent.join("com.tiez");
        let legacy = parent.join(LEGACY_IDENTIFIER_DIR_NAME);
        std::fs::create_dir_all(&default).unwrap();
        std::fs::create_dir_all(&legacy).unwrap();
        (default, legacy)
    }

    fn write_db(dir: &Path, marker: &str) {
        std::fs::write(dir.join("clipboard.db"), marker).unwrap();
    }

    /// 无任何显式配置的迁移上下文。
    fn plain_ctx() -> MigrationContext {
        MigrationContext::default()
    }

    // ---------- evaluate_legacy_identifier_data ----------

    #[test]
    fn legacy_eval_adopts_when_only_legacy_has_data() {
        let (default, legacy) = legacy_layout("adopt");
        write_db(&legacy, "OLD");
        assert_eq!(
            evaluate_legacy_identifier_data(&default),
            LegacyDataResolution::Adopt {
                legacy_dir: legacy.clone()
            }
        );
    }

    #[test]
    fn legacy_eval_none_when_no_legacy_data() {
        let (default, legacy) = legacy_layout("no_legacy");
        write_db(&default, "NEW");
        // 旧目录存在但没有数据库。
        assert_eq!(
            evaluate_legacy_identifier_data(&default),
            LegacyDataResolution::None
        );
        // 旧目录完全不存在。
        std::fs::remove_dir_all(&legacy).unwrap();
        assert_eq!(
            evaluate_legacy_identifier_data(&default),
            LegacyDataResolution::None
        );
    }

    #[test]
    fn legacy_eval_ambiguous_when_both_have_data() {
        let (default, legacy) = legacy_layout("ambig");
        write_db(&default, "NEW");
        write_db(&legacy, "OLD");
        assert_eq!(
            evaluate_legacy_identifier_data(&default),
            LegacyDataResolution::Ambiguous {
                legacy_dir: legacy.clone()
            }
        );
    }

    // ---------- choose_data_dir（重定向/便携/重复启动） ----------

    #[test]
    fn choose_prefers_valid_redirect() {
        let parent = temp_dir("redirect");
        let default = parent.join("com.tiez");
        let custom = parent.join("custom");
        std::fs::create_dir_all(&default).unwrap();
        std::fs::create_dir_all(&custom).unwrap();
        let choice = choose_data_dir(
            &default,
            true,
            Some(custom.to_string_lossy().as_ref()),
            None,
        );
        assert_eq!(choice.dir, custom);
        assert!(!choice.legacy_allowed);
    }

    #[test]
    fn choose_invalid_redirect_falls_back_but_blocks_legacy() {
        let parent = temp_dir("bad_redirect");
        let default = parent.join("com.tiez");
        std::fs::create_dir_all(&default).unwrap();
        let missing = parent.join("missing");
        let choice = choose_data_dir(
            &default,
            true,
            Some(missing.to_string_lossy().as_ref()),
            None,
        );
        assert_eq!(choice.dir, default);
        // 无效重定向也是显式配置：不得自动采用旧标识目录。
        assert!(!choice.legacy_allowed);
        // 空内容同理。
        let choice = choose_data_dir(&default, true, Some("  "), None);
        assert_eq!(choice.dir, default);
        assert!(!choice.legacy_allowed);
    }

    #[test]
    fn choose_portable_overrides_redirect_and_blocks_legacy() {
        let parent = temp_dir("portable");
        let default = parent.join("com.tiez");
        let custom = parent.join("custom");
        let portable = parent.join("portable_data");
        for dir in [&default, &custom, &portable] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let choice = choose_data_dir(
            &default,
            true,
            Some(custom.to_string_lossy().as_ref()),
            Some(&portable),
        );
        assert_eq!(choice.dir, portable);
        assert!(!choice.legacy_allowed);

        let choice = choose_data_dir(&default, false, None, Some(&portable));
        assert_eq!(choice.dir, portable);
        assert!(!choice.legacy_allowed);
    }

    #[test]
    fn choose_default_allows_legacy() {
        let parent = temp_dir("plain");
        let default = parent.join("com.tiez");
        std::fs::create_dir_all(&default).unwrap();
        let choice = choose_data_dir(&default, false, None, None);
        assert_eq!(choice.dir, default);
        assert!(choice.legacy_allowed);
    }

    /// 重复启动：第一次采用旧目录后，第二次走显式重定向路径，
    /// 即使两侧数据库并存也不再触发歧义处理。
    #[test]
    fn repeat_startup_uses_written_redirect() {
        let (default, legacy) = legacy_layout("repeat");
        write_db(&legacy, "OLD");
        assert_eq!(adopt_legacy_dir(&default, &legacy), legacy);

        // 第二次启动：datapath.txt 已存在。
        let redirect_path = default.join(DATA_REDIRECT_FILE);
        assert!(redirect_path.is_file());
        let content = std::fs::read_to_string(&redirect_path).unwrap();
        let choice = choose_data_dir(&default, true, Some(content.trim()), None);
        assert_eq!(choice.dir, legacy);
        assert!(!choice.legacy_allowed);
        // 旧目录数据库原样保留。
        assert_eq!(
            std::fs::read(legacy.join("clipboard.db")).unwrap(),
            b"OLD".to_vec()
        );
    }

    // ---------- write_data_redirect / adopt_legacy_dir ----------

    #[test]
    fn redirect_write_failure_keeps_everything_untouched() {
        let (default, legacy) = legacy_layout("fail_write");
        write_db(&legacy, "OLD");
        // 用同名目录占住 datapath.txt 使写入失败。
        std::fs::create_dir_all(default.join(DATA_REDIRECT_FILE)).unwrap();
        assert!(write_data_redirect(&default, &legacy).is_err());
        assert_eq!(adopt_legacy_dir(&default, &legacy), default);
        assert!(legacy.join("clipboard.db").is_file());
    }

    // ---------- migrate_old_tiezhi_data（贴汁目录非破坏性迁移） ----------

    fn tiezhi_layout(tag: &str) -> (PathBuf, PathBuf) {
        let parent = temp_dir(tag);
        let default = parent.join("com.tiez");
        let old = parent.join("贴汁");
        std::fs::create_dir_all(&old).unwrap();
        (default, old)
    }

    #[test]
    fn tiezhi_renames_whole_dir_when_default_missing() {
        let (default, old) = tiezhi_layout("rename");
        write_db(&old, "OLD");
        std::fs::write(old.join("clipboard.db-wal"), b"WAL").unwrap();
        std::fs::create_dir_all(old.join("attachments")).unwrap();
        std::fs::write(old.join("attachments").join("a.png"), b"IMG").unwrap();

        migrate_old_tiezhi_data(&[old.clone()], &default, &plain_ctx());

        assert!(default.join("clipboard.db").is_file());
        assert!(default.join("clipboard.db-wal").is_file());
        assert!(default.join("attachments").join("a.png").is_file());
        assert!(!old.exists());
    }

    #[test]
    fn tiezhi_adopts_in_place_when_default_exists_without_db() {
        let (default, old) = tiezhi_layout("inplace");
        std::fs::create_dir_all(&default).unwrap();
        std::fs::write(default.join("tiez.log"), b"LOG").unwrap();
        write_db(&old, "OLD");
        std::fs::write(old.join("clipboard.db-wal"), b"WAL").unwrap();

        migrate_old_tiezhi_data(&[old.clone()], &default, &plain_ctx());

        // 不复制数据库，只写重定向；旧目录完整保留。
        assert!(!default.join("clipboard.db").exists());
        let redirect = std::fs::read_to_string(default.join(DATA_REDIRECT_FILE)).unwrap();
        assert_eq!(redirect, old.to_string_lossy().to_string());
        assert!(old.join("clipboard.db").is_file());
        assert!(old.join("clipboard.db-wal").is_file());
    }

    /// 回归：旧逻辑会在 old 更大且 new < 50KB 时替换 new 的数据库并删除
    /// 旧目录；现在必须双方都原样保留。
    #[test]
    fn tiezhi_never_replaces_or_deletes_when_both_dbs_exist() {
        let (default, old) = tiezhi_layout("both");
        std::fs::create_dir_all(&default).unwrap();
        write_db(&default, "NEW");
        write_db(&old, "OLD-OLD-OLD-OLD-OLD-OLD");

        migrate_old_tiezhi_data(&[old.clone()], &default, &plain_ctx());

        assert_eq!(
            std::fs::read(default.join("clipboard.db")).unwrap(),
            b"NEW".to_vec()
        );
        assert_eq!(
            std::fs::read(old.join("clipboard.db")).unwrap(),
            b"OLD-OLD-OLD-OLD-OLD-OLD".to_vec()
        );
        assert!(!default.join("clipboard.db.backup").exists());
        assert!(!default.join(DATA_REDIRECT_FILE).exists());
    }

    #[test]
    fn tiezhi_preserves_user_redirect_and_old_dir() {
        let (default, old) = tiezhi_layout("user_redirect");
        let custom = default.parent().unwrap().join("custom");
        std::fs::create_dir_all(&custom).unwrap();
        write_db(&old, "OLD");
        std::fs::write(
            old.join(DATA_REDIRECT_FILE),
            custom.to_string_lossy().as_bytes(),
        )
        .unwrap();

        migrate_old_tiezhi_data(&[old.clone()], &default, &plain_ctx());

        let redirect = std::fs::read_to_string(default.join(DATA_REDIRECT_FILE)).unwrap();
        assert_eq!(redirect, custom.to_string_lossy().to_string());
        // 旧目录不被删除。
        assert!(old.join("clipboard.db").is_file());
    }

    #[test]
    fn tiezhi_leaves_empty_old_dir_untouched() {
        let (default, old) = tiezhi_layout("empty_old");
        std::fs::create_dir_all(&default).unwrap();
        write_db(&default, "NEW");

        migrate_old_tiezhi_data(&[old.clone()], &default, &plain_ctx());

        assert!(old.exists());
        assert!(!default.join(DATA_REDIRECT_FILE).exists());
    }

    /// 迁移写重定向失败（datapath.txt 被目录占用）时，原数据保留。
    #[test]
    fn tiezhi_migration_failure_preserves_data() {
        let (default, old) = tiezhi_layout("mig_fail");
        std::fs::create_dir_all(&default).unwrap();
        write_db(&old, "OLD");
        std::fs::create_dir_all(default.join(DATA_REDIRECT_FILE)).unwrap();

        migrate_old_tiezhi_data(&[old.clone()], &default, &plain_ctx());

        assert!(old.join("clipboard.db").is_file());
        assert!(default.join(DATA_REDIRECT_FILE).is_dir()); // 占位目录原样
        assert!(!default.join("clipboard.db").exists());
    }

    // ---------- cleanup_install_folders_at（数据护栏） ----------

    #[cfg(windows)]
    #[test]
    fn cleanup_skips_dirs_with_user_data() {
        let parent = temp_dir("cleanup_guard");
        let with_db = parent.join("with_db");
        let with_redirect = parent.join("with_redirect");
        let with_attachments = parent.join("with_attachments");
        let plain_install = parent.join("plain_install");
        for dir in [&with_db, &with_redirect, &with_attachments, &plain_install] {
            std::fs::create_dir_all(dir).unwrap();
        }
        write_db(&with_db, "X");
        std::fs::write(with_redirect.join(DATA_REDIRECT_FILE), b"X").unwrap();
        std::fs::create_dir_all(with_attachments.join("attachments")).unwrap();
        std::fs::write(plain_install.join("TieZ.exe"), b"EXE").unwrap();

        cleanup_install_folders_at(
            vec![
                Some(with_db.clone()),
                Some(with_redirect.clone()),
                Some(with_attachments.clone()),
                Some(plain_install.clone()),
            ],
            &[],
        );

        assert!(with_db.exists());
        assert!(with_redirect.exists());
        assert!(with_attachments.exists());
        assert!(!plain_install.exists());
    }


    // ---------- 问题 3：旧标识目录自带重定向 ----------

    #[test]
    fn legacy_redirect_target_becomes_data_dir() {
        let (default, legacy) = legacy_layout("lr_valid");
        let custom = default.parent().unwrap().join("custom_data");
        std::fs::create_dir_all(&custom).unwrap();
        write_db(&custom, "CUSTOM");
        // 旧目录只剩 datapath.txt，数据库在自定义目录。
        std::fs::write(
            legacy.join(DATA_REDIRECT_FILE),
            custom.to_string_lossy().as_bytes(),
        )
        .unwrap();

        assert_eq!(
            evaluate_legacy_identifier_data(&default),
            LegacyDataResolution::Adopt {
                legacy_dir: custom.clone()
            }
        );
        // 端到端：启动后直接采用自定义目录。
        let resolution = resolve_startup_data_dir(&default, None, &[], &|_, _| {
            panic!("unexpected confirm")
        });
        assert_eq!(resolution.dir, custom);
    }

    #[test]
    fn legacy_invalid_or_empty_redirect_falls_back_to_legacy_dir() {
        let (default, legacy) = legacy_layout("lr_invalid");
        write_db(&legacy, "OLD");
        // 指向不存在路径的重定向：回落到旧目录本身。
        let missing = default.parent().unwrap().join("missing");
        std::fs::write(
            legacy.join(DATA_REDIRECT_FILE),
            missing.to_string_lossy().as_bytes(),
        )
        .unwrap();
        assert_eq!(
            evaluate_legacy_identifier_data(&default),
            LegacyDataResolution::Adopt {
                legacy_dir: legacy.clone()
            }
        );

        // 空内容重定向：同样回落。
        let (default2, legacy2) = legacy_layout("lr_empty");
        write_db(&legacy2, "OLD");
        std::fs::write(legacy2.join(DATA_REDIRECT_FILE), b"   ").unwrap();
        assert_eq!(
            evaluate_legacy_identifier_data(&default2),
            LegacyDataResolution::Adopt {
                legacy_dir: legacy2.clone()
            }
        );
    }

    #[test]
    fn legacy_redirect_without_any_db_is_ignored() {
        let (default, legacy) = legacy_layout("lr_nodb");
        // 重定向目标存在但没有数据库，旧目录也没有数据库。
        let custom = default.parent().unwrap().join("custom_empty");
        std::fs::create_dir_all(&custom).unwrap();
        std::fs::write(
            legacy.join(DATA_REDIRECT_FILE),
            custom.to_string_lossy().as_bytes(),
        )
        .unwrap();
        assert_eq!(
            evaluate_legacy_identifier_data(&default),
            LegacyDataResolution::None
        );
    }

    /// 旧重定向目标与当前默认目录都存数据库时必须询问，不得静默选择。
    #[test]
    fn legacy_redirect_and_default_db_are_ambiguous() {
        let (default, legacy) = legacy_layout("lr_ambig");
        let custom = default.parent().unwrap().join("custom_data");
        std::fs::create_dir_all(&custom).unwrap();
        write_db(&custom, "CUSTOM");
        write_db(&default, "NEW");
        std::fs::write(
            legacy.join(DATA_REDIRECT_FILE),
            custom.to_string_lossy().as_bytes(),
        )
        .unwrap();

        assert_eq!(
            evaluate_legacy_identifier_data(&default),
            LegacyDataResolution::Ambiguous {
                legacy_dir: custom.clone()
            }
        );

        let confirm_args = std::cell::RefCell::new(None);
        let resolution =
            resolve_startup_data_dir(&default, None, &[], &|legacy_dir, default_dir| {
                *confirm_args.borrow_mut() =
                    Some((legacy_dir.to_path_buf(), default_dir.to_path_buf()));
                false
            });
        assert_eq!(
            confirm_args.into_inner(),
            Some((custom.clone(), default.clone()))
        );
        // 用户拒绝：保持默认目录，不写任何重定向，数据原样。
        assert_eq!(resolution.dir, default);
        assert!(!default.join(DATA_REDIRECT_FILE).exists());
        assert!(custom.join("clipboard.db").is_file());
    }

    // ---------- 问题 2：迁移不得覆盖当前显式配置 ----------

    /// 当前已有有效重定向时，“贴汁”迁移不得覆盖它，也不复制数据库。
    #[test]
    fn tiezhi_does_not_overwrite_current_redirect() {
        let (default, old) = tiezhi_layout("ctx_redirect");
        let current_target = default.parent().unwrap().join("current_target");
        std::fs::create_dir_all(&current_target).unwrap();
        write_db(&old, "OLD");
        let old_target = default.parent().unwrap().join("old_target");
        std::fs::write(
            old.join(DATA_REDIRECT_FILE),
            old_target.to_string_lossy().as_bytes(),
        )
        .unwrap();
        std::fs::create_dir_all(&default).unwrap();
        std::fs::write(
            default.join(DATA_REDIRECT_FILE),
            current_target.to_string_lossy().as_bytes(),
        )
        .unwrap();

        let ctx = MigrationContext {
            redirect_file_exists: true,
            redirect_target: Some(current_target.clone()),
            portable_dir: None,
        };
        migrate_old_tiezhi_data(&[old.clone()], &default, &ctx);

        let content = std::fs::read_to_string(default.join(DATA_REDIRECT_FILE)).unwrap();
        assert_eq!(content, current_target.to_string_lossy().to_string());
        assert!(!default.join("clipboard.db").exists());
        assert!(old.join("clipboard.db").is_file());
    }

    /// 当前重定向无效（指向不存在的路径）也同样是显式配置，不得被覆盖。
    #[test]
    fn tiezhi_does_not_replace_invalid_current_redirect() {
        let (default, old) = tiezhi_layout("ctx_invalid");
        write_db(&old, "OLD");
        let missing = default.parent().unwrap().join("missing");
        std::fs::create_dir_all(&default).unwrap();
        std::fs::write(
            default.join(DATA_REDIRECT_FILE),
            missing.to_string_lossy().as_bytes(),
        )
        .unwrap();

        let ctx = MigrationContext {
            redirect_file_exists: true,
            redirect_target: None,
            portable_dir: None,
        };
        migrate_old_tiezhi_data(&[old.clone()], &default, &ctx);

        let content = std::fs::read_to_string(default.join(DATA_REDIRECT_FILE)).unwrap();
        assert_eq!(content, missing.to_string_lossy().to_string());
        assert!(!default.join("clipboard.db").exists());
        assert!(old.join("clipboard.db").is_file());
    }

    /// 便携模式下迁移整体跳过，任何数据与配置都不变。
    #[test]
    fn tiezhi_skips_data_migration_in_portable_mode() {
        let (default, old) = tiezhi_layout("ctx_portable");
        write_db(&old, "OLD");
        let portable = default.parent().unwrap().join("portable_data");
        std::fs::create_dir_all(&portable).unwrap();

        let ctx = MigrationContext {
            redirect_file_exists: false,
            redirect_target: None,
            portable_dir: Some(portable.clone()),
        };
        migrate_old_tiezhi_data(&[old.clone()], &default, &ctx);

        assert!(!default.exists() || default.read_dir().unwrap().next().is_none());
        assert!(!default.join(DATA_REDIRECT_FILE).exists());
        assert!(old.join("clipboard.db").is_file());
    }

    // ---------- 问题 2：真实启动顺序（resolve_startup_data_dir） ----------

    /// 重复启动稳定性：首次采用“贴汁”目录后，第二次启动读取已写重定向，
    /// 迁移因显式配置存在而跳过，内容不再变化。
    #[test]
    fn startup_repeated_tiezhi_adoption_is_stable() {
        let (default, old) = tiezhi_layout("s_repeat");
        std::fs::create_dir_all(&default).unwrap();
        write_db(&old, "OLD");

        let first = resolve_startup_data_dir(&default, None, &[old.clone()], &|_, _| {
            panic!("unexpected confirm")
        });
        assert_eq!(first.dir, old);
        let redirect_after_first =
            std::fs::read_to_string(default.join(DATA_REDIRECT_FILE)).unwrap();

        let second = resolve_startup_data_dir(&default, None, &[old.clone()], &|_, _| {
            panic!("unexpected confirm")
        });
        assert_eq!(second.dir, old);
        let redirect_after_second =
            std::fs::read_to_string(default.join(DATA_REDIRECT_FILE)).unwrap();
        assert_eq!(redirect_after_first, redirect_after_second);
        // 旧数据原样保留。
        assert_eq!(
            std::fs::read(old.join("clipboard.db")).unwrap(),
            b"OLD".to_vec()
        );
    }

    /// 当前重定向与“贴汁”迁移并存：启动后使用当前重定向目标，
    /// 重定向内容不被覆盖，重复启动稳定。
    #[test]
    fn startup_tiezhi_never_overwrites_current_redirect() {
        let (default, old) = tiezhi_layout("s_ctx");
        let current_target = default.parent().unwrap().join("current_target");
        std::fs::create_dir_all(&current_target).unwrap();
        write_db(&old, "OLD");
        let old_target = default.parent().unwrap().join("old_target");
        std::fs::write(
            old.join(DATA_REDIRECT_FILE),
            old_target.to_string_lossy().as_bytes(),
        )
        .unwrap();
        std::fs::create_dir_all(&default).unwrap();
        std::fs::write(
            default.join(DATA_REDIRECT_FILE),
            current_target.to_string_lossy().as_bytes(),
        )
        .unwrap();

        for _ in 0..2 {
            let resolution = resolve_startup_data_dir(&default, None, &[old.clone()], &|_, _| {
                panic!("unexpected confirm")
            });
            assert_eq!(resolution.dir, current_target);
        }
        let content = std::fs::read_to_string(default.join(DATA_REDIRECT_FILE)).unwrap();
        assert_eq!(content, current_target.to_string_lossy().to_string());
        assert!(old.join("clipboard.db").is_file());
    }

    /// 无效当前重定向：保持默认目录、重定向内容原样，“贴汁”数据不动。
    #[test]
    fn startup_invalid_current_redirect_respected() {
        let (default, old) = tiezhi_layout("s_inv");
        write_db(&old, "OLD");
        let missing = default.parent().unwrap().join("missing");
        std::fs::create_dir_all(&default).unwrap();
        std::fs::write(
            default.join(DATA_REDIRECT_FILE),
            missing.to_string_lossy().as_bytes(),
        )
        .unwrap();

        let resolution = resolve_startup_data_dir(&default, None, &[old.clone()], &|_, _| {
            panic!("unexpected confirm")
        });
        assert_eq!(resolution.dir, default);
        let content = std::fs::read_to_string(default.join(DATA_REDIRECT_FILE)).unwrap();
        assert_eq!(content, missing.to_string_lossy().to_string());
        assert!(!default.join("clipboard.db").exists());
        assert!(old.join("clipboard.db").is_file());
    }

    /// 便携模式优先于一切迁移：不写重定向、不迁移数据，两侧旧数据原样保留。
    #[test]
    fn startup_portable_mode_wins_and_preserves_all() {
        let (default, legacy) = legacy_layout("s_portable");
        write_db(&legacy, "LEGACY");
        let old = default.parent().unwrap().join("贴汁");
        std::fs::create_dir_all(&old).unwrap();
        write_db(&old, "OLD");
        let portable = default.parent().unwrap().join("portable_data");
        std::fs::create_dir_all(&portable).unwrap();

        let resolution = resolve_startup_data_dir(
            &default,
            Some(&portable),
            &[old.clone()],
            &|_, _| panic!("unexpected confirm"),
        );
        assert_eq!(resolution.dir, portable);
        assert!(!default.join(DATA_REDIRECT_FILE).exists());
        assert!(legacy.join("clipboard.db").is_file());
        assert!(old.join("clipboard.db").is_file());
    }

    /// 两份数据并存时用户确认采用旧数据：写重定向、数据不移动。
    #[test]
    fn startup_ambiguous_confirm_adopts_legacy() {
        let (default, legacy) = legacy_layout("s_ambig_yes");
        write_db(&default, "NEW");
        write_db(&legacy, "OLD");

        let resolution = resolve_startup_data_dir(&default, None, &[], &|_, _| true);
        assert_eq!(resolution.dir, legacy);
        let content = std::fs::read_to_string(default.join(DATA_REDIRECT_FILE)).unwrap();
        assert_eq!(content, legacy.to_string_lossy().to_string());
        assert_eq!(
            std::fs::read(legacy.join("clipboard.db")).unwrap(),
            b"OLD".to_vec()
        );
        assert_eq!(
            std::fs::read(default.join("clipboard.db")).unwrap(),
            b"NEW".to_vec()
        );
    }

    // ---------- 问题 1：清理护栏（便携数据与受保护目录） ----------

    #[cfg(windows)]
    #[test]
    fn cleanup_skips_portable_data_inside_install_dir() {
        let parent = temp_dir("cleanup_portable");
        let install_dir = parent.join("tiezhi_install");
        let portable_data = install_dir.join("data");
        std::fs::create_dir_all(portable_data.join("attachments")).unwrap();
        std::fs::write(portable_data.join("clipboard.db"), b"DB").unwrap();
        std::fs::write(portable_data.join("clipboard.db-wal"), b"WAL").unwrap();
        std::fs::write(portable_data.join("attachments").join("img.png"), b"IMG").unwrap();
        std::fs::write(install_dir.join("TieZ.exe"), b"EXE").unwrap();

        cleanup_install_folders_at(vec![Some(install_dir.clone())], &[]);

        // 顶层没有任何数据标记，但 data/ 内是便携历史：目录必须完整保留。
        assert!(portable_data.join("clipboard.db").is_file());
        assert!(portable_data.join("clipboard.db-wal").is_file());
        assert!(portable_data.join("attachments").join("img.png").is_file());
        assert!(install_dir.exists());
    }

    #[cfg(windows)]
    #[test]
    fn cleanup_skips_ancestor_of_protected_dir() {
        let parent = temp_dir("cleanup_ancestor");
        let install_dir = parent.join("install");
        let custom_data = install_dir.join("mydata");
        std::fs::create_dir_all(&custom_data).unwrap();
        write_db(&custom_data, "DATA");
        std::fs::write(install_dir.join("TieZ.exe"), b"EXE").unwrap();

        // 自定义数据目录位于候选安装目录内部：候选是其祖先，必须跳过。
        cleanup_install_folders_at(vec![Some(install_dir.clone())], &[custom_data.clone()]);
        assert!(custom_data.join("clipboard.db").is_file());
        assert!(install_dir.exists());

        // 相等情形与大小写差异同样跳过。
        let upper = parent.join("INSTALL");
        cleanup_install_folders_at(vec![Some(install_dir.clone())], &[upper]);
        assert!(install_dir.exists());
    }

    /// 受保护目录进入启动编排结果：最终数据目录、显式配置目标都被列入。
    #[test]
    fn startup_reports_protected_dirs() {
        let (default, legacy) = legacy_layout("s_protected");
        write_db(&legacy, "OLD");
        let resolution = resolve_startup_data_dir(&default, None, &[], &|_, _| {
            panic!("unexpected confirm")
        });
        assert!(resolution.protected_dirs.contains(&default));
        assert!(resolution.protected_dirs.contains(&legacy));
    }

    /// 将路径转换为 8.3 短路径形式；卷不支持或 API 失败时返回 None。
    #[cfg(windows)]
    fn to_short_path(path: &Path) -> Option<PathBuf> {
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::GetShortPathNameW;
        let wide: Vec<u16> = path
            .as_os_str()
            .to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let len = unsafe { GetShortPathNameW(PCWSTR::from_raw(wide.as_ptr()), None) };
        if len == 0 {
            return None;
        }
        let mut buffer = vec![0u16; len as usize];
        let written =
            unsafe { GetShortPathNameW(PCWSTR::from_raw(wide.as_ptr()), Some(&mut buffer)) };
        if written == 0 {
            return None;
        }
        Some(PathBuf::from(String::from_utf16_lossy(
            &buffer[..written as usize],
        )))
    }

    /// 用 cmd 内建 mklink /J 创建目录联接（无需管理员权限）。
    #[cfg(windows)]
    fn make_junction(link: &Path, target: &Path) -> bool {
        std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link.as_os_str())
            .arg(target.as_os_str())
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false)
    }

    /// 构造“安装目录内的自定义历史目录”布局，返回 (install, history)，
    /// history 含数据库、WAL 和附件。
    #[cfg(windows)]
    fn install_with_history(tag: &str) -> (PathBuf, PathBuf) {
        let parent = temp_dir(tag);
        let install = parent.join("Program Files").join("TieZ");
        let history = install.join("custom-history");
        std::fs::create_dir_all(history.join("attachments")).unwrap();
        std::fs::write(history.join("clipboard.db"), b"DB").unwrap();
        std::fs::write(history.join("clipboard.db-wal"), b"WAL").unwrap();
        std::fs::write(history.join("attachments").join("img.png"), b"IMG").unwrap();
        std::fs::create_dir_all(&install).unwrap();
        std::fs::write(install.join("TieZ.exe"), b"EXE").unwrap();
        (install, history)
    }

    /// 路径别名（..、扩展前缀、大小写、8.3 短名）不得绕过受保护目录护栏：
    /// 任何别名形式下，候选目录内的历史数据库、WAL、附件都必须原样保留。
    #[cfg(windows)]
    #[test]
    fn cleanup_resolves_windows_path_aliases() {
        let (install, history) = install_with_history("cleanup_alias");

        let mut candidates: Vec<PathBuf> = vec![
            // `..` 拼写别名
            install.join("..").join("TieZ"),
            // 扩展路径前缀
            PathBuf::from(format!("\\\\?\\{}", install.to_string_lossy())),
            // 大小写拼写差异
            install
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("PROGRAM FILES")
                .join("tiez"),
        ];
        // 8.3 短路径别名：卷不支持短名或 API 不可用时明确报告跳过。
        match to_short_path(&install) {
            Some(short) if short != install => candidates.push(short),
            Some(_) => eprintln!(
                "[test] 8.3 short names unavailable on this volume; short-path alias variant skipped"
            ),
            None => eprintln!(
                "[test] GetShortPathNameW failed; short-path alias variant skipped"
            ),
        }

        assert!(candidates.len() >= 3);
        for candidate in &candidates {
            cleanup_install_folders_at(vec![Some(candidate.clone())], &[history.clone()]);
            assert!(
                history.join("clipboard.db").is_file(),
                "history db lost for candidate {candidate:?}"
            );
            assert_eq!(
                std::fs::read(history.join("clipboard.db-wal")).unwrap(),
                b"WAL".to_vec(),
                "history WAL altered for candidate {candidate:?}"
            );
            assert!(
                history.join("attachments").join("img.png").is_file(),
                "attachment lost for candidate {candidate:?}"
            );
            assert!(install.join("TieZ.exe").is_file());
        }
    }

    /// 候选安装目录内含指向真实数据的目录联接：候选可被删除，但联接
    /// 目标的数据（数据库、WAL、附件）必须原样保留。
    #[cfg(windows)]
    #[test]
    fn cleanup_junction_inside_install_does_not_wipe_target() {
        let parent = temp_dir("cleanup_junction_child");
        let install = parent.join("install");
        let data = parent.join("real-data");
        std::fs::create_dir_all(&install).unwrap();
        std::fs::create_dir_all(data.join("attachments")).unwrap();
        std::fs::write(data.join("clipboard.db"), b"DB").unwrap();
        std::fs::write(data.join("clipboard.db-wal"), b"WAL").unwrap();
        std::fs::write(data.join("attachments").join("img.png"), b"IMG").unwrap();
        std::fs::write(install.join("TieZ.exe"), b"EXE").unwrap();

        if !make_junction(&install.join("history"), &data) {
            eprintln!("[test] junction creation unavailable; junction-child test skipped");
            return;
        }

        cleanup_install_folders_at(vec![Some(install.clone())], &[data.clone()]);
        assert!(data.join("clipboard.db").is_file());
        assert_eq!(
            std::fs::read(data.join("clipboard.db-wal")).unwrap(),
            b"WAL".to_vec()
        );
        assert!(data.join("attachments").join("img.png").is_file());
        assert!(!install.exists());
    }

    /// 候选本身是目录联接时不删除：链接保留，目标数据原样。
    #[cfg(windows)]
    #[test]
    fn cleanup_skips_junction_candidate_pointing_at_data() {
        let parent = temp_dir("cleanup_junction_self");
        let holder = parent.join("holder");
        let data = holder.join("real-data");
        let link = parent.join("tiezhi_link");
        std::fs::create_dir_all(data.join("attachments")).unwrap();
        std::fs::write(data.join("clipboard.db"), b"DB").unwrap();
        std::fs::write(data.join("attachments").join("img.png"), b"IMG").unwrap();

        if !make_junction(&link, &holder) {
            eprintln!("[test] junction creation unavailable; junction-candidate test skipped");
            return;
        }

        cleanup_install_folders_at(vec![Some(link.clone())], &[data.clone()]);
        assert!(link.exists(), "junction itself must not be deleted");
        assert!(data.join("clipboard.db").is_file());
        assert!(data.join("attachments").join("img.png").is_file());
    }

    /// 解析失败必须返回 None（fail closed 的基础），不存在路径无法解析。
    #[cfg(windows)]
    #[test]
    fn canonicalize_failure_returns_none() {
        let missing = temp_dir("cleanup_noresolve").join("missing");
        assert!(canonicalize_for_compare(&missing).is_none());
    }

    /// 无数据、无保护关系的普通安装残留仍会被清理。
    #[cfg(windows)]
    #[test]
    fn cleanup_still_removes_plain_residue() {
        let parent = temp_dir("cleanup_plain");
        let residue = parent.join("tiezhi_install");
        std::fs::create_dir_all(residue.join("bin")).unwrap();
        std::fs::write(residue.join("bin").join("TieZ.exe"), b"EXE").unwrap();

        cleanup_install_folders_at(vec![Some(residue.clone())], &[]);
        assert!(!residue.exists());
    }

    /// 构造“存在性无法判定”的受保护路径（故障注入）。优先用非法路径
    /// 字符（Windows ERROR_INVALID_NAME），不可用时用 icacls 拒绝读取
    /// 构造真实访问失败；两者都不可用返回 None（调用方明确报告跳过）。
    #[cfg(windows)]
    fn make_indeterminate_protected(parent: &Path) -> Option<(PathBuf, Option<String>)> {
        // 注入方式 1：非法字符路径 → 元数据错误而非“不存在”。
        let invalid = parent.join("bad<name>?dir");
        if matches!(invalid.try_exists(), Err(_)) {
            return Some((invalid, None));
        }
        // 注入方式 2：icacls 拒绝当前用户读取 → 访问被拒。
        let user = std::env::var("USERNAME").unwrap_or_default();
        if !user.is_empty() {
            let denied = parent.join("denied_dir");
            if std::fs::create_dir_all(&denied).is_ok() {
                let ok = std::process::Command::new("icacls")
                    .arg(&denied)
                    .args(["/deny", &format!("{user}:(R)")])
                    .output()
                    .map(|out| out.status.success())
                    .unwrap_or(false);
                if ok && matches!(denied.try_exists(), Err(_)) {
                    return Some((denied, Some(user)));
                }
            }
        }
        None
    }

    /// 恢复 icacls 注入的拒绝 ACE，便于临时目录清理。
    #[cfg(windows)]
    fn restore_acl(dir: &Path, user: &str) {
        let _ = std::process::Command::new("icacls")
            .arg(dir)
            .args(["/remove:d", user])
            .output();
    }

    /// 受保护目录存在性不可判定（故障注入）：护栏构建失败，且端到端
    /// 清理整体放弃——普通残留也不删除。
    #[cfg(windows)]
    #[test]
    fn guard_aborts_when_protected_existence_unknown() {
        let parent = temp_dir("guard_unknown");
        let Some((indeterminate, denied_user)) = make_indeterminate_protected(&parent) else {
            eprintln!(
                "[test] no fault-injection method available (invalid-name and icacls both unusable); test skipped"
            );
            return;
        };

        let real_exe_dir = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        assert!(prepare_cleanup_guard(&[indeterminate.clone()], Some(&real_exe_dir)).is_none());

        // 端到端：受保护目录存在性未知时整体放弃清理，普通残留保留。
        let residue = parent.join("tiezhi_install");
        std::fs::create_dir_all(&residue).unwrap();
        std::fs::write(residue.join("TieZ.exe"), b"EXE").unwrap();
        cleanup_install_folders_at(vec![Some(residue.clone())], &[indeterminate.clone()]);
        assert!(residue.exists(), "cleanup must abort entirely");

        if let Some(user) = denied_user {
            restore_acl(&indeterminate, &user);
        }
    }

    /// 运行程序目录不可得或无法解析时，护栏构建失败（fail closed）。
    #[cfg(windows)]
    #[test]
    fn guard_aborts_when_exe_dir_unavailable() {
        assert!(prepare_cleanup_guard(&[], None).is_none());

        let parent = temp_dir("guard_noexe");
        let missing_exe_dir = parent.join("missing");
        assert!(prepare_cleanup_guard(&[], Some(&missing_exe_dir)).is_none());

        // 正常传入真实程序目录时护栏可用。
        let real_exe_dir = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        assert!(prepare_cleanup_guard(&[], Some(&real_exe_dir)).is_some());
    }

    /// 明确不存在的受保护目录被忽略，不阻断清理。
    #[cfg(windows)]
    #[test]
    fn guard_ignores_definitively_missing_protected_dir() {
        let parent = temp_dir("guard_missing_ok");
        let missing = parent.join("absent");
        let real_exe_dir = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        assert!(prepare_cleanup_guard(&[missing], Some(&real_exe_dir)).is_some());
    }

    #[cfg(windows)]
    #[test]
    fn cleanup_never_deletes_current_exe_dir() {
        let current_dir = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
        cleanup_install_folders_at(vec![Some(current_dir.clone())], &[]);
        assert!(current_dir.exists());
    }
}
