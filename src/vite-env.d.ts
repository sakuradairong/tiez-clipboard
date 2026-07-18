/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_EDITION?: "local" | "cloud";
  readonly VITE_API_BASE_URL?: string;
  readonly VITE_ANNOUNCEMENT_PING_URL?: string;
  readonly VITE_ENABLE_UPDATER?: "true" | "false";
  readonly VITE_REPOSITORY_URL?: string;
  readonly VITE_RELEASES_URL?: string;
  readonly VITE_ISSUES_URL?: string;
  readonly VITE_OFFICIAL_WEBSITE_URL?: string;
  readonly VITE_SUPPORT_EMAIL?: string;
  readonly VITE_AI_DEFAULT_API_KEY?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
