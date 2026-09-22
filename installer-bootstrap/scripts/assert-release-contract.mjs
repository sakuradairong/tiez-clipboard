import fs from "node:fs";

const release = fs.readFileSync(new URL("../../.github/workflows/release.yml", import.meta.url), "utf8");
const mainConf = JSON.parse(fs.readFileSync(new URL("../../src-tauri/tauri.conf.json", import.meta.url), "utf8"));
const shellConf = JSON.parse(fs.readFileSync(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8"));

if (!release.includes("updaterJsonPreferNsis: true")) {
  throw new Error("release.yml must keep updaterJsonPreferNsis: true so latest.json points at the inner NSIS setup");
}
if (release.includes("updaterJsonPreferNsis: false")) {
  throw new Error("release.yml must not turn updaterJsonPreferNsis off");
}

const wizardJob = release.slice(release.indexOf("publish-bootstrapper:"));
if (!wizardJob.startsWith("publish-bootstrapper:")) {
  throw new Error("release.yml is missing the publish-bootstrapper job");
}
if (wizardJob.includes("tauri-apps/tauri-action")) {
  throw new Error("the install wizard job must not run tauri-action; that would publish another latest.json");
}

const installMode = mainConf.bundle?.windows?.nsis?.installMode;
if (installMode !== "currentUser") {
  throw new Error(`main installMode must stay currentUser, found ${installMode}`);
}
const endpoints = mainConf.plugins?.updater?.endpoints ?? [];
for (const endpoint of endpoints) {
  if (String(endpoint).includes("windows-x64-installer") || String(endpoint).includes("tiez-installer")) {
    throw new Error(`updater endpoint must not target the wizard: ${endpoint}`);
  }
}
if (mainConf.bundle?.createUpdaterArtifacts !== true) {
  throw new Error("main app must keep createUpdaterArtifacts enabled");
}

if (shellConf.identifier !== "com.tiez.installer") {
  throw new Error(`wizard identifier must be com.tiez.installer, found ${shellConf.identifier}`);
}
if (shellConf.identifier === mainConf.identifier) {
  throw new Error("wizard must not reuse the main app identifier");
}
if (shellConf.bundle?.active !== false || shellConf.bundle?.createUpdaterArtifacts !== false) {
  throw new Error("wizard bundle must stay inactive and must not create updater artifacts");
}

console.log("release contract ok: updater stays on the inner NSIS, wizard is a separate bundle");
