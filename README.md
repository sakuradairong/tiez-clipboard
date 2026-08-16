<div align="center">
  <img src="docs/images/logo.png" alt="TieZ logo" width="220" />
  <h1>TieZ</h1>
  <p>A fast, local-first clipboard manager for Windows, Linux, and macOS.</p>
  <p>
    <img src="https://img.shields.io/badge/status-community%20maintained-4CAF50" alt="Community maintained" />
    <a href="https://www.gnu.org/licenses/gpl-3.0"><img src="https://img.shields.io/badge/license-GPL--3.0-FF9800" alt="GPL-3.0 license" /></a>
    <img src="https://img.shields.io/badge/platform-Windows%20%7C%20Linux%20%7C%20macOS-f44336" alt="Windows, Linux, and macOS" />
    <img src="https://img.shields.io/badge/stack-WinUI%203%20(Windows)%20%7C%20Tauri%202%20(Linux%2FmacOS)%20%7C%20Rust-2196F3" alt="WinUI 3 on Windows, Tauri 2 on Linux and macOS, and Rust" />
  </p>
  <p>
    <a href="./README.md">English</a> | <a href="./README.zh-CN.md">简体中文</a>
  </p>
  <p>
    <a href="../../releases"><strong>Download</strong></a>
    · <a href="./CHANGELOG.md">Changelog</a>
    · <a href="./CONTRIBUTING.md">Contributing</a>
  </p>
</div>

---

TieZ keeps frequently used clipboard content close at hand without moving your history into a hosted service. It captures common clipboard formats, makes them searchable, and provides focused workflows for organizing, syncing, and pasting content.

## Highlights

| Area | Capabilities |
| :--- | :--- |
| **Capture and paste** | Text, rich HTML, images, files, clipboard history, quick paste, and sequential paste |
| **Organize and find** | Full-text search, source filtering, pinned items, multi-color tags, and configurable cleanup rules |
| **Sync and transfer** | WebDAV history sync, MQTT connectivity, and LAN content or file transfer |
| **Productivity** | Emoji library, external-editor round trips, QR-code recognition, and global shortcuts |
| **Privacy** | Local-first storage and automatic masking of sensitive preview content |
| **Desktop experience** | Edge docking, light and dark modes, and four visual themes |

## Theme Preview

<div align="center">
  <table>
    <tr>
      <td align="center"><b>Frosted Glass</b><br><img src="docs/images/毛玻璃.png" alt="Frosted Glass theme" width="220" /></td>
      <td align="center"><b>Notebook</b><br><img src="docs/images/书.png" alt="Notebook theme" width="220" /></td>
      <td align="center"><b>Sticky Note</b><br><img src="docs/images/便利贴.png" alt="Sticky Note theme" width="220" /></td>
      <td align="center"><b>3D</b><br><img src="docs/images/3d.png" alt="3D theme" width="220" /></td>
    </tr>
  </table>
</div>

## Install

Download the latest build from the [Releases page](../../releases), then choose the package for your platform.

| Platform | Requirement | Packages |
| :--- | :--- | :--- |
| **Windows** | Windows 10 or 11, x64 | Signed MSIX `.msix`, App Installer `.appinstaller` |
| **Linux** | Ubuntu 22.04 or a compatible x64 desktop | DEB `.deb`, AppImage |
| **macOS** | macOS 11+, Apple Silicon or Intel | DMG `.dmg` |

### Platform Notes

- **Windows application:** The published Windows app uses a Chinese-first native WinUI 3 main window and does not ship a WebView2 main-window runtime. App Installer provides publisher-verified in-place updates.
- **Migrating an older Windows install:** Export a TieZ backup, fully exit the legacy tray process, install the `.appinstaller` (preferred) or `.msix`, and confirm the native app can read the existing history before uninstalling the old NSIS/MSI package. Never run the two applications together against the same data directory; a shared database mutex rejects the second writer.
- **Linux clipboard capture:** Supports X11/XWayland and Wayland compositors that implement the data-control protocol.
- **Linux automatic paste:** Uses `xdotool` on X11 or `wtype` on Wayland. If neither is available, TieZ leaves the requested content on the clipboard for manual paste.
- **macOS permissions:** Clipboard history and automatic paste may require Pasteboard and Accessibility access in System Settings.
- **macOS distribution:** Current builds are not signed or notarized with an Apple Developer certificate, so Gatekeeper may require manual approval on first launch.
- **OCR:** System OCR uses Windows Runtime and is available on Windows only. QR-code recognition is cross-platform.
- **Portable builds:** A portable ZIP is not currently published.

## Development

For the production Windows application, install Node.js LTS, Rust, PowerShell 7, and Visual Studio 2022 with the Desktop development with C++ workload, then build the native WinUI release:

```powershell
npm install
npm run winui:build
npm run winui:test:release
npm run winui:package
```

The release-readiness command starts five isolated native processes, runs 100
show/close-to-tray cycles in the first process, and enforces startup-latency and
memory ceilings before a Windows package can be published.

Linux/macOS development and the source-level Windows rollback path retain the Tauri frontend and its platform prerequisites:

```bash
npm install
npm run tauri:dev
```

Run the main validation commands before submitting a change:

```bash
npm test
npm run build
npm run test:rust
npm run verify:windows-release
```

See [CONTRIBUTING.md](./CONTRIBUTING.md) for contribution scope and pull request guidance.

## Project Status

TieZ is maintained as a community fork. Current priorities are reproducible builds, reliable releases, cross-platform compatibility, and focused bug fixes. Large feature proposals should be discussed in an issue before implementation.

- Report bugs and request features through [Issues](../../issues).
- Review release changes in [CHANGELOG.md](./CHANGELOG.md).
- Follow [SECURITY.md](./SECURITY.md) for responsible disclosure.
- Review legacy updater endpoints, signing credentials, and distribution infrastructure before publishing binaries from a new fork.

## License

TieZ is distributed under the [GNU General Public License v3.0](./LICENSE).

<div align="center">
  <strong>If TieZ is useful to you, consider leaving a Star.</strong>
</div>
