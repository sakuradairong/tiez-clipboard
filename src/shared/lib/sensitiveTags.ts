/** Keep aligned with database::SENSITIVE_TAGS and its case-insensitive matching. */
export const BUILTIN_SENSITIVE_TAG_NAMES = ["sensitive", "密码", "password"] as const;

export const hasSensitiveTag = (tags?: readonly string[]): boolean =>
  tags?.some((tag) =>
    BUILTIN_SENSITIVE_TAG_NAMES.some((name) => name === tag.toLowerCase())
  ) ?? false;
