export const isTauriRuntime = (): boolean => {
  if (typeof process !== "undefined" && (process.env.VITEST || process.env.NODE_ENV === "test")) {
    return true;
  }
  if (typeof window === "undefined") return false;
  const w = window as unknown as {
    __TAURI__?: unknown;
    __TAURI_INTERNALS__?: unknown;
    __TAURI_IPC__?: unknown;
  };
  return !!(w.__TAURI__ || w.__TAURI_INTERNALS__ || w.__TAURI_IPC__);
};

