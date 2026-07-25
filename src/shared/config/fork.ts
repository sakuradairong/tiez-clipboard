const normalizeValue = (value?: string): string | null => {
  const trimmed = value?.trim();
  return trimmed ? trimmed : null;
};

export const githubHttpsUrlOrFallback = (
  value: string | undefined,
  fallback: string
): string => {
  const candidate = normalizeValue(value);
  if (!candidate) return fallback;
  try {
    const url = new URL(candidate);
    if (
      url.protocol === "https:" &&
      url.hostname.toLowerCase() === "github.com" &&
      !url.username &&
      !url.password &&
      !url.port
    ) {
      url.pathname = url.pathname.replace(/\/+$/, "") || "/";
      return url.toString();
    }
  } catch {
    // Fall through to the GitHub URL allowed by the Tauri opener capability.
  }
  return fallback;
};

const DEFAULT_REPOSITORY_URL =
  "https://github.com/sakuradairong/tiez-clipboard";
const REPOSITORY_URL = githubHttpsUrlOrFallback(
  import.meta.env.VITE_REPOSITORY_URL,
  DEFAULT_REPOSITORY_URL
);
const RELEASES_URL = githubHttpsUrlOrFallback(
  import.meta.env.VITE_RELEASES_URL,
  `${REPOSITORY_URL}/releases`
);
const ISSUES_URL = githubHttpsUrlOrFallback(
  import.meta.env.VITE_ISSUES_URL,
  `${REPOSITORY_URL}/issues`
);
const OFFICIAL_WEBSITE_URL = githubHttpsUrlOrFallback(
  import.meta.env.VITE_OFFICIAL_WEBSITE_URL,
  REPOSITORY_URL
);

const SUPPORT_EMAIL = normalizeValue(import.meta.env.VITE_SUPPORT_EMAIL);
const THEME_STORE_API_BASE = normalizeValue(import.meta.env.VITE_API_BASE_URL);
const ANNOUNCEMENT_PING_URL = normalizeValue(
  import.meta.env.VITE_ANNOUNCEMENT_PING_URL
);

const ENABLE_UPDATER = import.meta.env.VITE_ENABLE_UPDATER === "true";

export const FORK_LINKS = {
  repository: REPOSITORY_URL,
  releases: RELEASES_URL,
  issues: ISSUES_URL,
  website: OFFICIAL_WEBSITE_URL,
  supportEmail: SUPPORT_EMAIL,
} as const;

export const FORK_SERVICES = {
  themeStoreApiBase: THEME_STORE_API_BASE,
  announcementPingUrl: ANNOUNCEMENT_PING_URL,
  updaterEnabled: ENABLE_UPDATER,
} as const;
