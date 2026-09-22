# TieZ install wizard

Optional Windows install wizard. It is a separate Tauri app (`com.tiez.installer`) and does not add commands to the clipboard app.

The wizard embeds `TieZ_<version>_x64-setup.exe` and starts that existing NSIS installer. It does not download the setup. The first install uses `/S` only. `/P`, `/UPDATE`, `/R`, and `/NS` are never passed. If the user changes the install directory, `/D=` is appended last and is not quoted. After a successful install, "Launch TieZ" reads `MainBinaryName` from `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\TieZ` and starts that executable. It does not pass `/R`.

`latest.json` stays pointed at the inner NSIS setup. The wizard is an additional release file, `TieZ-<version>-windows-x64-installer.exe`, not the primary download.

There is no license page: the main app does not set `bundle.license`.

## Linux checks

This environment cannot produce a Windows `setup.exe` or the wizard executable.

```bash
cd installer-bootstrap
npm ci
npm test
npm run build
cargo test --manifest-path Cargo.toml -p tiez-installer-core
node scripts/assert-release-contract.mjs
```

`npm run build` only builds the wizard page. It does not embed NSIS and does not create `tiez-installer.exe`.

## Windows build

Build the inner NSIS setup first (the main app's `tauri build --bundles nsis`), then:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\installer-bootstrap\scripts\embed-and-build.ps1 -SetupPath C:\path\TieZ_0.3.12_x64-setup.exe
```

Output: `installer-bootstrap\dist-bootstrapper\TieZ-0.3.12-windows-x64-installer.exe`.

Set `TIEZ_SETUP_EXE` to that setup before `npm run tauri:build` if you build by hand. A build without that variable still compiles, and the wizard reports that no installer was embedded instead of pretending the install succeeded.

CI: `.github/workflows/bootstrapper.yml` builds the embedded wizard on `windows-latest`. The release workflow uploads it beside the NSIS and MSI drafts after `publish-tauri`, without running `tauri-action` a second time.
