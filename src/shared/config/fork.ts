const normalizeUrl = (value?: string): string | null => {
  const trimmed = value?.trim();
  return trimmed ? trimmed : null;
};

const REPOSITORY_URL =
  normalizeUrl(import.meta.env.VITE_REPOSITORY_URL) ??
  "https://github.com/sakuradairong/tiez-clipboard";

const RELEASES_URL =
  normalizeUrl(import.meta.env.VITE_RELEASES_URL) ?? `${REPOSITORY_URL}/releases`;

const ISSUES_URL =
  normalizeUrl(import.meta.env.VITE_ISSUES_URL) ?? `${REPOSITORY_URL}/issues`;

const OFFICIAL_WEBSITE_URL =
  normalizeUrl(import.meta.env.VITE_OFFICIAL_WEBSITE_URL) ?? REPOSITORY_URL;

const SUPPORT_EMAIL = normalizeUrl(import.meta.env.VITE_SUPPORT_EMAIL);
const THEME_STORE_API_BASE = normalizeUrl(import.meta.env.VITE_API_BASE_URL);
const ANNOUNCEMENT_PING_URL = normalizeUrl(
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
