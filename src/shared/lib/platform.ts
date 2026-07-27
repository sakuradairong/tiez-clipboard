export const isMacPlatform = (): boolean => {
  if (typeof navigator === "undefined") return false;

  return (
    /Mac|iPhone|iPad|iPod/i.test(navigator.userAgent) ||
    /Mac/i.test(navigator.platform)
  );
};

export const detectsWindowsPlatform = (userAgent: string, platform: string): boolean =>
  /Windows|Win32|Win64/i.test(userAgent) || /Win/i.test(platform);

export const isWindowsPlatform = (): boolean => {
  if (typeof navigator === "undefined") return false;

  return detectsWindowsPlatform(navigator.userAgent, navigator.platform);
};
