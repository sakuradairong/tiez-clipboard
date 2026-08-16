<div align="center">
  <img src="docs/images/logo.png" alt="TieZ 标志" width="220" />
  <h1>TieZ</h1>
  <p>面向 Windows、Linux 和 macOS 的快速、本地优先剪贴板管理器。</p>
  <p>
    <img src="https://img.shields.io/badge/status-community%20maintained-4CAF50" alt="社区维护" />
    <a href="https://www.gnu.org/licenses/gpl-3.0"><img src="https://img.shields.io/badge/license-GPL--3.0-FF9800" alt="GPL-3.0 协议" /></a>
    <img src="https://img.shields.io/badge/platform-Windows%20%7C%20Linux%20%7C%20macOS-f44336" alt="Windows、Linux 和 macOS" />
    <img src="https://img.shields.io/badge/stack-WinUI%203%20(Windows)%20%7C%20Tauri%202%20(Linux%2FmacOS)%20%7C%20Rust-2196F3" alt="Windows 使用 WinUI 3，Linux 和 macOS 使用 Tauri 2，核心使用 Rust" />
  </p>
  <p>
    <a href="./README.md">English</a> | <a href="./README.zh-CN.md">简体中文</a>
  </p>
  <p>
    <a href="../../releases"><strong>下载</strong></a>
    · <a href="./CHANGELOG.md">更新日志</a>
    · <a href="./CONTRIBUTING.md">参与贡献</a>
  </p>
</div>

---

TieZ 将常用剪贴板内容保留在本地，不依赖托管服务存储历史记录。它能够采集常见剪贴板格式，提供快速检索，并覆盖整理、同步和粘贴等日常工作流。

## 核心能力

| 领域 | 功能 |
| :--- | :--- |
| **采集与粘贴** | 文本、富 HTML、图片、文件、剪贴板历史、快速粘贴和顺序粘贴 |
| **整理与检索** | 全文搜索、来源筛选、置顶记录、多色标签和可配置清理规则 |
| **同步与传输** | WebDAV 历史同步、MQTT 连接，以及局域网内容或文件传输 |
| **效率工具** | Emoji 表情库、外部编辑器回写、二维码识别和全局快捷键 |
| **隐私保护** | 本地优先存储，以及敏感内容预览自动脱敏 |
| **桌面体验** | 贴边收纳、亮色与暗色模式，以及四款视觉主题 |

## 主题预览

<div align="center">
  <table>
    <tr>
      <td align="center"><b>极简毛玻璃</b><br><img src="docs/images/毛玻璃.png" alt="极简毛玻璃主题" width="220" /></td>
      <td align="center"><b>笔记本</b><br><img src="docs/images/书.png" alt="笔记本主题" width="220" /></td>
      <td align="center"><b>便利贴</b><br><img src="docs/images/便利贴.png" alt="便利贴主题" width="220" /></td>
      <td align="center"><b>3D</b><br><img src="docs/images/3d.png" alt="3D 主题" width="220" /></td>
    </tr>
  </table>
</div>

## 下载安装

从 [Releases 页面](../../releases)下载最新版本，并选择对应平台的安装包。

| 平台 | 运行环境 | 安装包 |
| :--- | :--- | :--- |
| **Windows** | Windows 10 或 11，x64 | 已签名 MSIX `.msix`、App Installer `.appinstaller` |
| **Linux** | Ubuntu 22.04 或兼容的 x64 桌面环境 | DEB `.deb`、AppImage |
| **macOS** | macOS 11+，Apple Silicon 或 Intel | DMG `.dmg` |

### 平台说明

- **Windows 原生界面：** 正式发布的 Windows 版使用中文优先的 WinUI 3 原生主窗口，不再随主窗口分发 WebView2 运行时；App Installer 负责校验发布者签名并原位升级。
- **从旧版 Windows 安装迁移：** 请先导出 TieZ 备份并彻底退出旧版托盘进程，再优先安装 `.appinstaller`（也可安装 `.msix`）。确认原生版能够读取原有历史后，才卸载旧 NSIS/MSI 包。不要让两种程序同时访问同一数据目录；共享数据库互斥锁会拒绝第二个写入进程。
- **Linux 剪贴板采集：** 支持 X11/XWayland，以及实现 data-control 协议的 Wayland 桌面环境。
- **Linux 自动粘贴：** X11 使用 `xdotool`，Wayland 使用 `wtype`。如果两者均不可用，TieZ 会将目标内容留在剪贴板中，供用户手动粘贴。
- **macOS 权限：** 剪贴板历史和自动粘贴可能需要在系统设置中授予 Pasteboard 与辅助功能权限。
- **macOS 分发：** 当前构建尚未使用 Apple Developer 证书签名或公证，首次启动时可能需要手动通过 Gatekeeper 检查。
- **OCR：** 系统 OCR 基于 Windows Runtime，目前仅支持 Windows；二维码识别支持所有平台。
- **便携版本：** 当前暂未发布便携版 ZIP。

## 本地开发

开发正式 Windows 应用时，请安装 Node.js LTS、Rust、PowerShell 7，以及带“使用 C++ 的桌面开发”工作负载的 Visual Studio 2022，然后构建 WinUI 原生版本：

```powershell
npm install
npm run winui:build
npm run winui:test:release
npm run winui:package
```

发布就绪命令会启动 5 个互相隔离的原生进程，在首个进程中执行 100 次显示/关闭到托盘循环，并在允许发布 Windows 安装包前强制检查启动延迟和内存上限。

Linux/macOS 开发以及 Windows 源码级回退仍保留 Tauri 前端，并需要对应平台的 Tauri 2 前置依赖：

```bash
npm install
npm run tauri:dev
```

提交改动前，请运行主要验证命令：

```bash
npm test
npm run build
npm run test:rust
npm run verify:windows-release
```

贡献范围和 PR 要求请查看 [CONTRIBUTING.md](./CONTRIBUTING.md)。

## 项目状态

TieZ 当前作为社区维护分支持续推进，优先处理可复现构建、可靠发布、跨平台兼容和聚焦的缺陷修复。较大的功能提案建议先通过 Issue 讨论。

- 通过 [Issues](../../issues) 反馈缺陷或提交功能建议。
- 在 [CHANGELOG.md](./CHANGELOG.md) 中查看版本变更。
- 按照 [SECURITY.md](./SECURITY.md) 的说明反馈安全问题。
- 从新的 fork 发布二进制前，请重新检查旧版更新接口、签名凭据和分发基础设施。

## 开源协议

TieZ 使用 [GNU General Public License v3.0](./LICENSE) 发布。

<div align="center">
  <strong>如果 TieZ 对你有帮助，欢迎点一个 Star。</strong>
</div>
