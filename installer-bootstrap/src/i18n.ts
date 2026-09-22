export const zh = {
  welcomeTitle: "安装 TieZ",
  welcomeBody: "为这台电脑安装剪贴板历史。安装向导只负责界面，真正写入文件的仍是现有的安装程序。",
  shortcutNote: "安装会在桌面和开始菜单创建 TieZ 快捷方式。",
  updateNote: "以后的更新由 TieZ 应用内更新完成，不会再次打开本向导。",
  installedNote: "已检测到 TieZ {version}。继续会覆盖安装，不会先运行卸载程序。",
  previousLocation: "上次安装在 {location}。保持下面的默认路径时，安装程序会沿用它记住的位置。",
  start: "开始安装",
  cancel: "取消",
  pathTitle: "安装位置",
  pathHelp: "默认是当前用户目录。只有改成别的位置时，才会把该路径传给安装程序。",
  defaultPathNote: "不修改时，若这台电脑还留着上次的安装路径，会继续用那条路径。",
  browse: "浏览…",
  back: "返回",
  next: "安装",
  diskOk: "目标磁盘有足够的可用空间。",
  diskLow: "目标磁盘剩余空间可能不够，仍可以继续。",
  diskUnknown: "无法估算剩余空间，仍可以继续。",
  commandPreview: "将要执行的参数",
  progressTitle: "正在安装",
  progressExtract: "正在准备安装程序…",
  progressInstall: "正在安装 TieZ…",
  progressHint: "安装程序在静默运行，这里不会显示百分比。",
  finishOk: "安装完成",
  finishFail: "安装未完成",
  launchNow: "立即启动 TieZ",
  close: "关闭",
  retry: "重试",
  reveal: "显示安装程序",
  exitCode: "退出码 {code}",
  notEmbedded: "这个向导构建没有嵌入安装程序。请在 Windows 上用内层 NSIS setup.exe 重新构建。",
  windowsOnly: "TieZ 安装向导只能在 Windows 上安装。",
  invalidDir: "请选择一个绝对路径，路径中不要包含引号。",
  genericFail: "安装程序没有成功结束。",
  stepWelcome: "欢迎",
  stepPath: "位置",
  stepProgress: "安装",
  stepFinish: "完成",
  language: "English",
  version: "版本 {version}",
  working: "请稍候",
} as const;

export type MessageKey = keyof typeof zh;

export const en: Record<MessageKey, string> = {
  welcomeTitle: "Install TieZ",
  welcomeBody: "Install clipboard history on this PC. This wizard is only the window; the existing installer still writes the files.",
  shortcutNote: "Setup creates TieZ shortcuts on the desktop and in the Start menu.",
  updateNote: "Later updates use TieZ's in-app updater and do not open this wizard again.",
  installedNote: "TieZ {version} is already installed. Continuing overwrites the files and does not run the uninstaller first.",
  previousLocation: "Last installed at {location}. Leaving the default path lets setup reuse the location it remembered.",
  start: "Continue",
  cancel: "Cancel",
  pathTitle: "Install location",
  pathHelp: "The default is this user's folder. The path is sent to setup only when you change it.",
  defaultPathNote: "If you leave the default, setup reuses a previous install location when one is still remembered.",
  browse: "Browse…",
  back: "Back",
  next: "Install",
  diskOk: "The destination drive has enough free space.",
  diskLow: "The destination drive may be short on space. You can still continue.",
  diskUnknown: "Free space could not be estimated. You can still continue.",
  commandPreview: "Arguments that will be used",
  progressTitle: "Installing",
  progressExtract: "Preparing the installer…",
  progressInstall: "Installing TieZ…",
  progressHint: "Setup is running silently, so this window has no percentage.",
  finishOk: "Installation complete",
  finishFail: "Installation did not finish",
  launchNow: "Launch TieZ",
  close: "Close",
  retry: "Try again",
  reveal: "Show installer",
  exitCode: "Exit code {code}",
  notEmbedded: "This wizard build does not contain the installer. Rebuild it on Windows with the inner NSIS setup.exe.",
  windowsOnly: "The TieZ install wizard can install only on Windows.",
  invalidDir: "Choose an absolute path without quotes.",
  genericFail: "Setup did not finish successfully.",
  stepWelcome: "Welcome",
  stepPath: "Location",
  stepProgress: "Install",
  stepFinish: "Finish",
  language: "中文",
  version: "Version {version}",
  working: "Please wait",
};

export type Lang = "zh" | "en";

export function detectLang(): Lang {
  const lang = typeof navigator === "undefined" ? "en" : navigator.language.toLowerCase();
  return lang.startsWith("zh") ? "zh" : "en";
}

export function translate(lang: Lang, key: MessageKey, vars: Record<string, string> = {}): string {
  const template = lang === "zh" ? zh[key] : en[key];
  return template.replace(/\{(\w+)\}/g, (_, name: string) => vars[name] ?? "");
}
