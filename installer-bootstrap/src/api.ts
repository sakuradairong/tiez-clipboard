export interface InstallerContext {
  product_version: string;
  default_install_dir: string;
  default_available: boolean;
  setup_embedded: boolean;
  windows_host: boolean;
  existing_version: string | null;
  existing_location: string | null;
  previous_location_differs: boolean;
}

export interface DirCheck {
  ok: boolean;
  normalized: string;
  passes_custom_dir: boolean;
  raw_tail: string;
  disk: "ok" | "low" | "unknown";
  free_bytes: number | null;
  need_bytes: number;
  error: string | null;
}

export interface InstallOutcome {
  exit_code: number;
  kind: string;
  setup_path: string | null;
  raw_tail: string;
  detail: string | null;
}

export function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function demoMode(): string | null {
  if (typeof location === "undefined") return null;
  return new URLSearchParams(location.search).get("demo");
}

function browserContext(): InstallerContext {
  const demo = demoMode();
  return {
    product_version: "0.3.12",
    default_install_dir: "C:\\Users\\Example\\AppData\\Local\\TieZ",
    default_available: demo !== null,
    setup_embedded: demo !== null,
    windows_host: demo !== null,
    existing_version: demo === "installed" ? "0.3.11" : null,
    existing_location: demo === "installed" ? "D:\\Apps\\TieZ" : null,
    previous_location_differs: demo === "installed",
  };
}

export async function loadContext(): Promise<InstallerContext> {
  if (!inTauri()) return browserContext();
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<InstallerContext>("installer_context");
}

export async function checkInstallDir(installDir: string): Promise<DirCheck> {
  if (!inTauri()) {
    const context = browserContext();
    const trimmed = installDir.trim();
    if (!trimmed || trimmed.includes('"')) {
      return {
        ok: false,
        normalized: installDir,
        passes_custom_dir: false,
        raw_tail: "",
        disk: "unknown",
        free_bytes: null,
        need_bytes: 0,
        error: "not_absolute",
      };
    }
    const same = trimmed.replace(/\//g, "\\").replace(/\\+$/, "").toLowerCase() === context.default_install_dir.toLowerCase();
    return {
      ok: true,
      normalized: trimmed.replace(/\//g, "\\").replace(/\\+$/, ""),
      passes_custom_dir: !same,
      raw_tail: same ? "/S" : `/S /D=${trimmed}`,
      disk: "unknown",
      free_bytes: null,
      need_bytes: 0,
      error: null,
    };
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<DirCheck>("check_install_dir", { installDir });
}

export async function browseInstallDir(title: string): Promise<string | null> {
  if (!inTauri()) return null;
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<string | null>("browse_install_dir", { title });
}

export async function listenProgress(onStage: (stage: string) => void): Promise<() => void> {
  if (!inTauri()) return () => {};
  const { listen } = await import("@tauri-apps/api/event");
  const unlisten = await listen<{ stage: string }>("install-progress", (event) => {
    onStage(event.payload.stage);
  });
  return unlisten;
}

export async function runInstall(installDir: string, onStage: (stage: string) => void): Promise<InstallOutcome> {
  if (!inTauri()) {
    onStage("extract");
    await delay(350);
    onStage("install");
    await delay(700);
    const demo = demoMode();
    if (demo === "fail") {
      return {
        exit_code: 2,
        kind: "script_abort",
        setup_path: "C:\\Users\\Example\\AppData\\Local\\Temp\\tiez-bootstrapper\\0.3.12\\TieZ_0.3.12_x64-setup.exe",
        raw_tail: "/S",
        detail: null,
      };
    }
    if (demo === "success" || demo === "installed") {
      return { exit_code: 0, kind: "success", setup_path: null, raw_tail: "/S", detail: null };
    }
    return { exit_code: -1, kind: "setup_not_embedded", setup_path: null, raw_tail: "/S", detail: null };
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<InstallOutcome>("install", { installDir });
}

export async function launchInstalled(): Promise<void> {
  if (!inTauri()) return;
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("launch_installed");
}

export async function revealSetup(): Promise<void> {
  if (!inTauri()) return;
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("reveal_setup");
}

export async function closeWizard(): Promise<void> {
  if (!inTauri()) {
    window.close();
    return;
  }
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("close_wizard");
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => {
    window.setTimeout(resolve, ms);
  });
}
