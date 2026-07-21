import { useState } from "react";
import { open, save, ask, message } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import { ArchiveRestore, ChevronDown, ChevronRight, Download, Loader2 } from "lucide-react";

interface DataSettingsGroupProps {
    t: (key: string) => string;
    collapsed: boolean;
    onToggle: () => void;
    dataPath: string;
}

interface BackupInfo {
    formatVersion: number;
    appVersion: string;
    createdAt: number;
    entryCount: number;
    fileCount: number;
    totalBytes: number;
    path: string;
}

const formatBytes = (bytes: number) => {
    if (bytes < 1024) return `${bytes} B`;
    const units = ["KB", "MB", "GB", "TB"];
    let value = bytes / 1024;
    let index = 0;
    while (value >= 1024 && index < units.length - 1) {
        value /= 1024;
        index += 1;
    }
    return `${value.toFixed(value >= 10 ? 1 : 2)} ${units[index]}`;
};

const DataSettingsGroup = ({ t, collapsed, onToggle, dataPath }: DataSettingsGroupProps) => {
    const [backupBusy, setBackupBusy] = useState<"export" | "restore" | null>(null);

    const exportBackup = async () => {
        if (backupBusy) return;
        const date = new Date().toISOString().slice(0, 10);
        const destination = await save({
            title: t('backup_export'),
            defaultPath: `TieZ-backup-${date}.tiez-backup`,
            filters: [{ name: "TieZ Backup", extensions: ["tiez-backup"] }]
        });
        if (!destination) return;

        setBackupBusy("export");
        try {
            const info = await invoke<BackupInfo>("create_backup", { destination });
            await message(
                t('backup_export_success')
                    .replace('{count}', String(info.entryCount))
                    .replace('{size}', formatBytes(info.totalBytes)),
                { title: t('backup_export'), kind: 'info' }
            );
        } catch (error) {
            await message(
                t('backup_failed').replace('{e}', String(error)),
                { title: t('error'), kind: 'error' }
            );
        } finally {
            setBackupBusy(null);
        }
    };

    const restoreBackup = async () => {
        if (backupBusy) return;
        const selected = await open({
            title: t('backup_restore'),
            multiple: false,
            directory: false,
            filters: [{ name: "TieZ Backup", extensions: ["tiez-backup"] }]
        });
        if (!selected || Array.isArray(selected)) return;

        setBackupBusy("restore");
        try {
            const info = await invoke<BackupInfo>("inspect_backup", { path: selected });
            const confirmed = await ask(
                t('backup_restore_confirm')
                    .replace('{version}', info.appVersion)
                    .replace('{count}', String(info.entryCount))
                    .replace('{size}', formatBytes(info.totalBytes))
                    .replace('{date}', new Date(info.createdAt).toLocaleString()),
                {
                    title: t('backup_restore'),
                    kind: 'warning',
                    okLabel: t('confirm'),
                    cancelLabel: t('cancel')
                }
            );
            if (!confirmed) return;

            await invoke("schedule_backup_restore", { path: selected });
            await message(t('backup_restore_scheduled'), {
                title: t('backup_restore'),
                kind: 'info'
            });
            await invoke("relaunch");
        } catch (error) {
            await message(
                t('backup_failed').replace('{e}', String(error)),
                { title: t('error'), kind: 'error' }
            );
        } finally {
            setBackupBusy(null);
        }
    };

    return (
    <div className={`settings-group ${collapsed ? 'collapsed' : ''}`}>
        <div className="group-header" onClick={onToggle}>
            <h3 style={{ margin: 0 }}>{t('data_management')}</h3>
            {collapsed ? <ChevronRight size={16} /> : <ChevronDown size={16} />}
        </div>
        {!collapsed && (
            <div className="group-content">
                <div className="setting-item column no-border">
                    <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '8px' }}>
                        <span className="item-label" style={{ textTransform: 'uppercase', fontSize: '11px', opacity: 0.8 }}>{t('data_path')}</span>
                        <div style={{ display: 'flex', gap: '8px' }}>
                            <button
                                className="btn-icon"
                                onClick={() => {
                                    open({
                                        directory: true,
                                        multiple: false,
                                        title: t('change_data_path')
                                    }).then(async (selected) => {
                                        if (selected) {
                                            const newPath = selected as string;
                                            const confirm = await ask(
                                                t('data_move_confirm').replace('{path}', newPath),
                                                { title: t('change_data_path'), kind: 'warning', okLabel: t('confirm'), cancelLabel: t('cancel') }
                                            );

                                            if (confirm) {
                                                try {
                                                    // Logic Update:
                                                    // We DO NOT copy the file here because the DB is locked/in-use.
                                                    // Instead, we just set the path and restart.
                                                    // The backend 'main.rs' startup logic will handle the migration (copying)
                                                    // if it detects a custom path with no DB using the default DB as source.

                                                    await invoke("set_data_path", { newPath });

                                                    await message(
                                                        t('data_move_success'),
                                                        { title: t('notice'), kind: 'info' }
                                                    );

                                                    await invoke("relaunch");
                                                } catch (e: unknown) {
                                                    console.error(e);
                                                    const errorMsg = e instanceof Error ? e.message : String(e);
                                                    await message(
                                                        t('data_move_failed').replace('{e}', errorMsg),
                                                        { title: t('error'), kind: 'error' }
                                                    );
                                                }
                                            }
                                        }
                                    });
                                }}
                                style={{ width: 'auto', padding: '4px 12px', fontSize: '10px', textTransform: 'uppercase', height: '24px' }}
                            >
                                {t('change_app')}
                            </button>
                            <button
                                className="btn-icon"
                                onClick={() => invoke("open_data_folder").catch(console.error)}
                                title={t('open_folder') || "Open"}
                                style={{ width: 'auto', padding: '4px 12px', fontSize: '10px', textTransform: 'uppercase', height: '24px' }}
                            >
                                {t('open_folder')}
                            </button>
                        </div>
                    </div>
                    <div className="data-panel" style={{ fontSize: '11px', color: 'var(--text-secondary)', wordBreak: 'break-all' }}>
                        {dataPath}
                    </div>
                </div>
                <div className="setting-item column">
                    <div style={{ display: 'flex', justifyContent: 'space-between', gap: '12px', alignItems: 'center', width: '100%' }}>
                        <div style={{ minWidth: 0 }}>
                            <div className="item-label">{t('backup_center')}</div>
                            <div className="item-description">{t('backup_center_hint')}</div>
                        </div>
                        <div style={{ display: 'flex', gap: '8px', flexShrink: 0 }}>
                            <button
                                className="btn-icon"
                                onClick={() => void exportBackup()}
                                disabled={backupBusy !== null}
                                style={{ width: 'auto', padding: '4px 12px', fontSize: '10px', height: '26px', gap: '5px' }}
                            >
                                {backupBusy === "export" ? <Loader2 size={12} className="animate-spin" /> : <Download size={12} />}
                                {t('backup_export')}
                            </button>
                            <button
                                className="btn-icon"
                                onClick={() => void restoreBackup()}
                                disabled={backupBusy !== null}
                                style={{ width: 'auto', padding: '4px 12px', fontSize: '10px', height: '26px', gap: '5px' }}
                            >
                                {backupBusy === "restore" ? <Loader2 size={12} className="animate-spin" /> : <ArchiveRestore size={12} />}
                                {t('backup_restore')}
                            </button>
                        </div>
                    </div>
                </div>
            </div>
        )}
    </div>
    );
};

export default DataSettingsGroup;
