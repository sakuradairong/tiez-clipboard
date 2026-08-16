import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const repositoryRoot = join(dirname(fileURLToPath(import.meta.url)), "..");

function read(relativePath) {
  return readFileSync(join(repositoryRoot, relativePath), "utf8");
}

function requireText(text, expected, message) {
  if (!text.includes(expected)) {
    throw new Error(`${message}\nMissing text: ${expected}`);
  }
}

function forbidText(text, forbidden, message) {
  if (text.includes(forbidden)) {
    throw new Error(`${message}\nForbidden text: ${forbidden}`);
  }
}

function section(text, start, end) {
  const startIndex = text.indexOf(start);
  const endIndex = text.indexOf(end, startIndex + start.length);
  if (startIndex < 0 || endIndex < 0) {
    throw new Error(`Unable to locate release-contract section: ${start} ... ${end}`);
  }
  return text.slice(startIndex, endIndex);
}

const releaseWorkflow = read(".github/workflows/release.yml");
const tauriRelease = section(
  releaseWorkflow,
  "  publish-tauri:",
  "  publish-winui-windows:",
);
const nativeRelease = releaseWorkflow.slice(
  releaseWorkflow.indexOf("  publish-winui-windows:"),
);

for (const forbidden of [
  "windows-latest",
  "x86_64-pc-windows-msvc",
  "--bundles nsis",
  "--bundles msi",
  "--bundles nsis,msi",
  "updaterJsonPreferNsis",
]) {
  forbidText(
    tauriRelease,
    forbidden,
    "The Tauri release matrix must not publish a Windows WebView2 application.",
  );
}

for (const expected of [
  "name: Windows x64 native WinUI MSIX",
  "      - publish-tauri",
  "test-release-readiness.ps1 -Configuration Release",
  "RequireSigning = $true",
  "AppInstallerBaseUri =",
  "Expected signed MSIX, SHA256, and App Installer assets",
]) {
  requireText(
    nativeRelease,
    expected,
    "The Windows release must fail closed and publish the signed native package set.",
  );
}

requireText(
  releaseWorkflow,
  "run: npm run verify:windows-release",
  "Release preflight must enforce this contract before publishing.",
);

const fallbackWorkflow = read(".github/workflows/build-platforms.yml");
for (const forbidden of ["windows-latest", "nsis", "msi", "WinUI"]) {
  forbidText(
    fallbackWorkflow,
    forbidden,
    "The Tauri platform workflow is only for Linux and macOS fallback packages.",
  );
}

const nativeWorkflow = read(".github/workflows/winui3-probe.yml");
requireText(
  nativeWorkflow,
  "  pull_request:",
  "The native Windows package must be validated on pull requests.",
);
requireText(
  nativeWorkflow,
  "run: npm run verify:windows-release",
  "Native Windows CI must verify the publishing contract.",
);
requireText(
  nativeWorkflow,
  "run: ./test-release-readiness.ps1 -Configuration Release",
  "Native Windows CI must enforce the five-run, 100-cycle readiness gate.",
);
requireText(
  nativeWorkflow,
  "run: ./package-msix.ps1 -SkipBuild",
  "Native Windows CI must structurally validate an MSIX.",
);

forbidText(
  releaseWorkflow + nativeWorkflow,
  "LifecycleCycles 10",
  "Production Windows workflows must not reduce lifecycle validation to ten cycles.",
);

const readinessScript = read(
  "experiments/winui3-main-window/test-release-readiness.ps1",
);
for (const expected of [
  "[int]$RunCount = 5",
  "[int]$LifecycleCycles = 100",
  "[double]$MaxMedianReadyMs = 750",
  "[double]$MaxWorstReadyMs = 1500",
  "[double]$MaxPeakWorkingSetMiB = 512",
  "[double]$MaxPrivateMemoryGrowthMiB = 64",
]) {
  requireText(
    readinessScript,
    expected,
    "The native release-readiness defaults must retain their production thresholds.",
  );
}

const packageManifest = JSON.parse(read("package.json"));
if (
  packageManifest.scripts?.["winui:test:release"] !==
  "pwsh -NoProfile -File experiments/winui3-main-window/test-release-readiness.ps1 -Configuration Release"
) {
  throw new Error("package.json must expose the canonical WinUI release-readiness command.");
}

const readme = read("README.md");
requireText(
  readme,
  "| **Windows** | Windows 10 or 11, x64 | Signed MSIX `.msix`, App Installer `.appinstaller` |",
  "The English install table must identify the native Windows packages.",
);
forbidText(
  section(readme, "## Install", "## Development"),
  "NSIS `.exe`, MSI `.msi`",
  "The public Windows install guide must not advertise the retired Tauri packages.",
);

const chineseReadme = read("README.zh-CN.md");
requireText(
  chineseReadme,
  "| **Windows** | Windows 10 或 11，x64 | 已签名 MSIX `.msix`、App Installer `.appinstaller` |",
  "The Chinese install table must identify the native Windows packages.",
);
forbidText(
  section(chineseReadme, "## 下载安装", "## 本地开发"),
  "NSIS `.exe`、MSI `.msi`",
  "The Chinese Windows install guide must not advertise the retired Tauri packages.",
);

console.log("Windows native release contract verified.");
