import fs from "node:fs";

function readJson(path) {
  return JSON.parse(fs.readFileSync(path, "utf8"));
}

function cargoPackageVersion(path, packageName) {
  const content = fs.readFileSync(path, "utf8");
  const packageSection = content.match(/\[package\]([\s\S]*?)(?:\n\[|$)/)?.[1];
  const name = packageSection?.match(/^name\s*=\s*"([^"]+)"/m)?.[1];
  const version = packageSection?.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
  if (name !== packageName || !version) {
    throw new Error(`Cannot read package ${packageName} from ${path}`);
  }
  return version;
}

function cargoLockVersion(path, packageName) {
  const blocks = fs
    .readFileSync(path, "utf8")
    .split(/\r?\n(?=\[\[package\]\])/);
  for (const block of blocks) {
    if (block.match(/^name\s*=\s*"([^"]+)"/m)?.[1] === packageName) {
      const version = block.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
      if (version) return version;
    }
  }
  throw new Error(`Cannot read package ${packageName} from ${path}`);
}

const pkg = readJson("package.json");
const packageLock = readJson("package-lock.json");
const tauri = readJson("src-tauri/tauri.conf.json");
const sources = {
  "package.json": pkg.version,
  "package-lock.json": packageLock.version,
  "package-lock.json root": packageLock.packages[""].version,
  "src-tauri/tauri.conf.json": tauri.version,
  "src-tauri/Cargo.toml": cargoPackageVersion(
    "src-tauri/Cargo.toml",
    "tiez-app",
  ),
  "src-tauri/Cargo.lock": cargoLockVersion(
    "src-tauri/Cargo.lock",
    "tiez-app",
  ),
  "winui rust-core/Cargo.toml": cargoPackageVersion(
    "experiments/winui3-main-window/rust-core/Cargo.toml",
    "tiez-winui-core",
  ),
  "winui rust-core/Cargo.lock": cargoLockVersion(
    "experiments/winui3-main-window/rust-core/Cargo.lock",
    "tiez-winui-core",
  ),
};

const mismatches = Object.entries(sources).filter(
  ([, version]) => version !== pkg.version,
);
if (mismatches.length > 0) {
  const details = Object.entries(sources)
    .map(([source, version]) => `${source}=${version}`)
    .join(", ");
  throw new Error(`Release versions do not match: ${details}`);
}

if (
  process.env.GITHUB_REF_TYPE === "tag" &&
  process.env.GITHUB_REF_NAME !== `v${pkg.version}`
) {
  throw new Error(
    `Tag ${process.env.GITHUB_REF_NAME} does not match v${pkg.version}`,
  );
}

if (process.argv.includes("--print")) {
  process.stdout.write(pkg.version);
} else {
  console.log(`Validated TieZ v${pkg.version} across ${Object.keys(sources).length} sources`);
}
