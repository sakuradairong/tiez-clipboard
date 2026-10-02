# 表格复制粘贴：复现矩阵与来源归因审计

本文件是「表格复制粘贴问题」探索的第三块交付物，目标是让实验/灰度同学能**按步骤复现**问题，并按「来源 → 剪贴板格式 → TieZ 处理路径 → 观察点」把现象归因到 TieZ 侧、来源应用侧，或明确划给「待实测」。
本文件只做静态文档工作：全部结论来自对仓库源码的只读检索（`Get-Content` / `Select-String` / `rg` / `git log`），**没有在本机做过任何真机剪贴板实验**。

## 0. 修订信息与审阅边界

| 项 | 值 |
| :--- | :--- |
| 审计基线 | 仓库 `HEAD` = `2d03ed0`（tag `v0.3.12`，分支 `master`） |
| 审计日期 | 见本文件所在提交；行号随代码演进会漂移，引用前请以当前文件为准 |
| 审计方法 | 静态代码检索 + 单元测试用例阅读；**未运行任何构建、测试或 GUI** |
| 未实测声明的含义 | 凡标注「待实测」的条目，均为静态推断或对来源应用行为的先验假设，**不得**当作已验证结论对外发布 |

## 1. 全局前置条件（所有复现步骤共用）

1. Windows 10/11 x64，TieZ 版本与上表 `v0.3.12` 一致（`CONTRIBUTING.md:14` 明确当前发布验证只针对 Windows）。
2. **必须开启设置项「捕获带格式的文本」**：该开关默认值为 `false`（`src-tauri/src/database.rs:251` 的 `app.capture_rich_text` 默认 `'false'`），文案见 `src/locales.ts:72-73`，开关 UI 见 `src/features/settings/components/groups/ClipboardSettingsGroup.tsx:412-425`，写入命令见 `src-tauri/src/app/commands/settings_cmd.rs:426-437`，运行期读取见 `src-tauri/src/services/clipboard/mod.rs:794`。
   **若未开启，任何表格都会被按纯文本采集（`ClipboardData::Text`），矩阵中「富文本采集」整条链路不生效——这是最常见的「表格形状丢失」误报来源。**
3. 建议同时记录「捕获文件」（默认 `false`，`src-tauri/src/database.rs:247`）与「富文本快照预览」（默认 `true`，`src-tauri/src/database.rs:255`）的当前取值，二者会改变条目**预览**形态（不改变采集到的 HTML）。
4. 记录每个条目的四个字段以便归因：`content_type`、`content`（纯文本）、`html_content`（富文本 HTML）、`source_app` / `source_app_path`。
5. 采集来源判定依赖「复制瞬间的前台窗口」，见 `src-tauri/src/services/clipboard/mod.rs:1197-1198` 取得 `source_snapshot`；若复制时焦点不在来源应用（例如用宏、AutoHotkey、远程注入发送 Ctrl+C），来源判定会错，必须记录为失败样本而非缺陷。

## 2. 术语表

| 术语 | 含义 |
| :--- | :--- |
| CF_HTML | Windows 注册格式 `HTML Format`，头部为 `Version:` / `StartHTML:` / `EndHTML:` / `StartFragment:` / `EndFragment:`（写入见 `src-tauri/src/services/clipboard_ops.rs:915`；解析见 `src-tauri/src/services/clipboard/utils.rs:2732`） |
| CF_UNICODETEXT | Win32 标准文本格式，常量 `13`（`src-tauri/src/infrastructure/windows_api/win_clipboard.rs:11`） |
| CF_DIB / CF_DIBV5 | Win32 位图格式，常量 `8` / `17`（`src-tauri/src/infrastructure/windows_api/win_clipboard.rs:9-10`） |
| CF_HDROP | 文件列表格式，常量 `15`（`src-tauri/src/infrastructure/windows_api/win_clipboard.rs:393`） |
| 命名格式 | `EnumClipboardFormats` 枚举到的已注册格式（如 `Rich Text Format`、`Biff12`、`HTML Format`），读取见 `src-tauri/src/infrastructure/windows_api/win_clipboard.rs:576-629` |
| 富文本条目 | `content_type == "rich_text"`，同时携带 `content` 与 `html_content`（`src-tauri/src/services/clipboard/pipeline.rs:101-105`） |
| 内部标记 | TieZ 把图片回退与命名格式数据以 HTML 注释形式挂在 `html_content` 尾部：`<!--TIEZ_RICH_IMAGE:`（`src-tauri/src/services/clipboard/utils.rs:19`）、`<!--TIEZ_RICH_FORMATS:`（`src-tauri/src/services/clipboard/utils.rs:21`） |

## 3. 来源 → 剪贴板格式 → TieZ 处理路径 → 观察点 矩阵

说明：下表「格式」列区分两类——**确定**（由 TieZ 代码显式读取/写入或由 Windows 平台契约保证）与**待实测**（来源应用实际提供的格式集合，必须用剪贴板查看器实测）。所有「TieZ 侧入口」均为仓库相对路径 + 行号。

| # | 来源 | 典型剪贴板格式 | TieZ 侧入口（文件:行号） | 观察点 / 失败判据（未实测者标「待实测」） |
| :-- | :--- | :--- | :--- | :--- |
| S1 | Microsoft Excel | CF_UNICODETEXT（Tab 分隔）、`HTML Format`、`Rich Text Format`、`Biff12`/`Biff8`/`Csv`/`Link` 等命名格式（后 4 类**待实测**枚举名） | 富文本探测 `mod.rs:411-499`；RTF 名存在性检查 `mod.rs:457-468`；表格源识别 `mod.rs:266-280`；命名格式保留 `mod.rs:359-387`；采集落地 `mod.rs:891-916` → `pipeline.rs:99-196` | 条目应为 `rich_text`，`html_content` 含 `<table`；`TIEZ_RICH_FORMATS` 中应出现 Excel 命名格式（**待实测**具体名单）；Tab 分隔纯文本应完整保留（`utils.rs:809-899`） |
| S2 | WPS 表格（`et.exe`） | 同上，另可能提供 WPS 私有格式（**待实测**） | 表格源识别 `mod.rs:277`（含 `et.exe`）；保留命名格式需要 `is_likely_spreadsheet_source` 为真 `mod.rs:352-356`；保守采集判定 `mod.rs:248-264`（`et.exe` **不属于** Writer，因此不触发保守路径） | 应保留命名格式；与 S1 的差异主要体现在命名格式名单与 HTML 头部噪声上（**待实测**） |
| S3 | WPS 文字（`wps.exe`） | CF_UNICODETEXT（常带 `MicrosoftInternetExplorer4 / DocumentNotSpecified / x.x 磅 Normal` 前缀）、`HTML Format`、`Rich Text Format` | 保守采集 `mod.rs:868-883`（**主动放弃命名格式与位图回退**）；RTF 探测被跳过 `mod.rs:457-467`；Office 元数据噪声清理 `utils.rs:579-629`、`utils.rs:696+` | 表格结构依赖 `HTML Format`；纯文本会带 Office 元数据噪声需被剥离（测试 `utils.rs:1706-1723`）；`html_content` 中**不会**出现 `TIEZ_RICH_FORMATS`（设计如此）；若来源 HTML 缺失或为 header 残缺形态，会退化（**待实测**） |
| S4 | Microsoft Word | CF_UNICODETEXT、`HTML Format`、`Rich Text Format`、`Object Descriptor` 等 | 富文本源识别 `mod.rs:230-243`（含 `winword`/`word`）；`Rich Text Format` 保留规则 `mod.rs:344-346`（**无条件为真**）；回写见 `clipboard_ops.rs:759-805` | Word 的 `Rich Text Format` 即使来源不是表格应用也会被保留；粘贴回 Word 时若只取 RTF，TieZ 自身**不生成 RTF**，只能靠命名格式原样回传（见 §6 A1 与 §6 B1） |
| S5 | 浏览器（Chrome / Edge / Firefox，含在线文档、飞书、企微网页版） | CF_UNICODETEXT、`HTML Format`、`PNG`/`image/png`、CF_DIB、`CF_HDROP`（图片拖拽类） | 富文本探测 `mod.rs:411-499`；图片优先判定 `mod.rs:95-111`；PNG/JPEG 快路径 `mod.rs:995-1053`；CF_DIB 慢回退 `mod.rs:1056+`；GIF 快路径 `mod.rs:798-818` | **网页表格**：`<table>` 是否原样保留（应保留）；**在线文档**：常见懒加载与 CSS 内联，`<img>` 存在时跳过位图抓取 `mod.rs:865-877`；图片型表格见 S8 |
| S6 | 钉钉（DingTalk） | CF_UNICODETEXT、`HTML Format`；表格多为应用内渲染（**待实测**是否提供 `HTML Format`） | 富文本探测 `mod.rs:411-499`（无钉钉专属分支，落入通用路径）；来源名会作为 `source_app` 进入去重键 `pipeline.rs:312-320` | 若钉钉只给纯文本，则 TieZ 侧无表格可还原，**归因在来源侧**；若给 HTML 但被 TieZ 判成噪声，则归因在 TieZ 侧（**待实测**：具体格式集合与 HTML 形态） |
| S7 | 记事本 / 终端（Notepad、Windows Terminal、cmd、PowerShell） | 仅 CF_UNICODETEXT / 纯文本 MIME | 采集路径 `mod.rs:1206-1210`（`normalize_clipboard_plain_text`）；类型判定 `pipeline.rs:100` 的 `detect_content_type` | 端到端应无表格可言；**关键用途是对照组**：同一份 3×3 数据在记事本中制表符应被原样保留为 `text` 条目，用于证明「表格形状丢失」发生在来源/HTML 环节而非文本环节 |
| S8 | 截图工具生成的表格图片（Win+Shift+S、Snipaste/PixPin、QQ/微信截图） | 仅图片：PNG/`image/png`、CF_DIB(V5)，部分工具附 `CF_HDROP` 临时文件；**不含** `HTML Format` | 截图来源识别 `mod.rs:163-185`；PNG 快路径 `mod.rs:995-1053`；CF_DIB 慢回退 `mod.rs:1056-1075`；CF_HDROP 单 GIF 归一 `mod.rs:124-161` | 入口即 `content_type == "image"`（`pipeline.rs:106`），**结构永远无法还原**，归因确定在来源侧；**待实测**：截图工具是否会顺带写入 `HTML Format` 或文件名格式导致误判为文件条目 |
| S9 | LibreOffice Calc / Writer（`soffice.exe`） | CF_UNICODETEXT、`HTML Format`、`Rich Text Format`、私有命名格式 | 源识别 `mod.rs:238`、`mod.rs:277`；`source_app_likely_formats_rich_text` 白名单 `utils.rs:942-953` | `soffice` 同时在富文本源与表格源白名单中，命名格式应被保留；**待实测**其 HTML 是否含 `StartFragment` 标记 |
| S10 | 纯文本编辑器/代码编辑器（VS Code、Sublime 等，作为对照） | CF_UNICODETEXT + 编辑器自身 HTML 高亮格式 | 故意不提升：`utils.rs:1062-1089` 仅在来源属白名单或纯文本带 HTML 信号时才把纯文本升级为富文本；测试 `utils.rs:1757-1767` | 复制 Markdown 表格源码时应保持 `text`/`code`，**不得**被误判成 HTML 表格；这是过度升级的回归哨兵 |

## 4. 端到端处理链路（按代码顺序，供逐步打点）

1. 事件触发，读取来源窗口：`mod.rs:1196-1198`。
2. 会话级快速读区（文本/文件/图片/HTML）：`mod.rs:1201-1220`。
3. 750 ms 内容去重窗口：`mod.rs:1232-1242`。
4. 富文本探测（7 次退避：`0/40/80/140/220/360/560 ms`，`mod.rs:21`）：`probe_rich_text_payload`，`mod.rs:411-499`；HTML 解析入口 `utils.rs:2732`。
5. 纯文本兜底探测：`read_clipboard_text_fresh`，`mod.rs:200-205`。
6. 图片/富文本优先判定：`should_prefer_image_over_rich_text`，`mod.rs:95-111`（Office/WPS 源直接返回 `false`，即永远优先富文本，见测试 `mod.rs:1470-1484`）。
7. 位图回退抓取：`clipboard_image_fallback_data_url`（`<img>` 已存在或 WPS Writer 源则跳过，`mod.rs:865-877`）。
8. 命名格式保留：`capture_preserved_named_formats_from_clipboard`，`mod.rs:359-387`；上限 12 个 / 单格式 1.5 MB / 合计 4 MB（`mod.rs:22-24`），超限即静默丢弃（`win_clipboard.rs:609-615`）。
9. 内部标记挂载：`attach_rich_image_fallback`（`utils.rs:1170`）、`attach_rich_named_formats`（`utils.rs:1201`）。
10. 入库与去重：`pipeline.rs:99-196`（Discovery）→ `pipeline.rs:202-289`（Transformation，含 `embed_local_images`/外部化，`utils.rs:2560`）→ `pipeline.rs:294-475`（Validation）→ `pipeline.rs:495+`（Persistence）。
11. **回写（粘贴）**：`copy_to_clipboard`，`clipboard_ops.rs:252-341`；富文本分支 `clipboard_ops.rs:737-807`；CF_HTML 生成 `clipboard_ops.rs:862-935`；命名格式还原 `clipboard_ops.rs:759-760`、写回 `win_clipboard.rs:710-747`；文本 + HTML 写回 `win_clipboard.rs:631-707`。

## 5. 复现步骤清单（Windows 手动）

> 每个用例都要求：记录 `content_type` / `content` / `html_content` 存在性 / `source_app`，并截图条目预览 + 目标应用实际粘贴结果。**未标注「已实测」的用例默认均为「待实测」。**

| ID | 数据规模 / 特征 | 操作步骤（简洁版） | 预期结果 | 失败判据 |
| :-- | :--- | :--- | :--- | :--- |
| R01 | 3×3 小表，纯中文 | Excel 选区 Ctrl+C → 打开 TieZ（Alt+C）→ 单击粘贴纯文本 → 再右键「带格式粘贴」 | 条目为 `rich_text` 且预览呈表格；纯文本粘贴得到 3 行 3 列（Tab 分隔）；带格式粘贴到 Excel/Word 还原为 3×3 表格 | 预览为纯文本；或带格式粘贴得到 1 行/1 列 |
| R02 | 50×20 | 同 R01，只做采集 | `html_content` 为完整 `<table>`；预览截断不影响数据结构 | 预览行数异常（前端预览另有 3 行截断，见 §7 交叉引用） |
| R03 | 500×50 | 同 R01 | 采集成功，无卡顿性超时；命名格式若超 1.5 MB / 4 MB 上限会被丢弃但 HTML 仍在 | HTML 丢失；或整条记录未被采集 |
| R04 | 含合并单元格（3 行合并 + 3 列合并） | Excel 合并单元格后 Ctrl+C → 采集 → 带格式粘贴回 Excel | `rowspan`/`colspan` 在 `html_content` 中保留 | 合并被展平为独立单元格 |
| R05 | 含换行单元格（Alt+Enter 多行） | 单单元格内多行 → Ctrl+C → 采集 | 纯文本中单元格内换行与行分隔可区分；带格式粘贴回 Excel 不炸行 | 表格行数变多（单元格内换行被当作行分隔） |
| R06 | 单单元格超长（≥5000 字符） | 同 R05 | 采集不丢；预览被截断（`utils.rs:1310-1323` / `1328-1428`） | 采集本身丢内容 |
| R07 | 5000×50（超宽超长） | 同 R03 | 明确记录耗时；预期可能触发命名格式上限或探测退避耗尽 | 无任何错误提示但内容明显缺失（静默丢弃） |
| R08 | 含数字与日期、货币格式 | 同 R01、R05 | 纯文本保留格式如 `1,234.00`、`2026/09/17`；带格式粘贴保留 Excel 显示格式 | 显示值被转成序列号（如 `46252`）——若发生，需区分是来源 HTML 已丢失格式还是 TieZ 侧丢失 |
| R09 | WPS 表格 3×3 中文 | `et.exe` 中 Ctrl+C → 采集 → 带格式粘贴回 WPS 表格 | 与 R01 一致；命名格式保留分支生效（`mod.rs:352-356`） | 命名格式为空且 HTML 也异常 |
| R10 | WPS 文字内嵌表格 | `wps.exe` 中复制表格 → 采集 | 预期**不**保留命名格式（`mod.rs:868-883` 的保守采集），但 `<table>` HTML 必须在 | HTML 丢失 → 归因 TieZ 侧 |
| R11 | Word 7 列表格（带边框底纹） | Word 表格 Ctrl+C → 采集 → 粘贴回 Word | `Rich Text Format` 命名格式被保留（`mod.rs:344-346`） | 带格式粘贴后边框/底纹丢失 |
| R12 | Chrome 网页表格 10×6 | 网页 `<table>` 选区 Ctrl+C → 采集 | `html_content` 含完整 `<table>` 与内联样式 | 只得到纯文本 |
| R13 | 飞书 / 企微 / 钉钉文档在线表格 | 选区 Ctrl+C → 采集 | 见 S5/S6；先记录实际提供的格式集合 | 只得到纯文本 → 先判来源侧，再判 TieZ 侧（**必须附剪贴板查看器截图**） |
| R14 | 记事本 UTF-8 3×3 制表符文本 | 记事本 Ctrl+C → 采集 | 条目应为 `text`，Tab 与换行原样保留 | 被误判成 `rich_text` 或 Tab 被吞 |
| R15 | 终端 ANSI 输出 | Windows Terminal 选区复制 → 采集 | 条目为 `text`；ANSI 转义序列不做处理（预期内） | 出现乱码以外的结构丢失 |
| R16 | Win+Shift+S 截图表格 | 截取表格区域 → 采集 | 条目为 `image`（`pipeline.rs:106`） | 被误判为 `rich_text`/`file` |
| R17 | Snipaste / PixPin 截图表格 | 工具内 Ctrl+C → 采集 | 同 R16；若工具附 `CF_HDROP` 且关闭「捕获文件」，预期仍只采图片（`mod.rs:150-161`） | 得到 `file` 条目 |
| R18 | 全链路回归：TieZ 自身复制→粘贴 | 采集 R01 后，在 TieZ 中带格式粘贴到 Excel，再立刻 Ctrl+C 复制该区域 | 不应产生重复条目（回声抑制 `pipeline.rs:299-330`） | 每次粘贴都新增条目 |
| R19 | 剪贴板查看器对照 | 对 S1–S10 各装一个剪贴板查看器，逐个记录格式清单 | 形成矩阵第 3 列的事实依据，替换本文件中的「待实测」 | 缺少格式清单 → 归因不成立 |
| R20 | 来源焦点错位对照 | 用 AutoHotkey 在窗口未聚焦时发送 Ctrl+C | 明确记录 `source_app` 是否为预期来源 | 若错位，该样本作废（不进入缺陷判定） |

## 6. 分层结论

### A. 确定在 TieZ 侧

- **A1. TieZ 从不生成 `Rich Text Format` 剪贴板数据。** 回写只写 `CF_UNICODETEXT` 与 `CF_HTML`（`win_clipboard.rs:631-707`），RTF 仅作为「来源提供的命名格式」原样回传（`mod.rs:344-346`、`clipboard_ops.rs:796-804`）。因此「定位在 Word」且只接受 RTF 的目标应用会出现格式降级，与来源无关。
- **A2. 粘贴时的 CF_HTML 会被重新包裹/改写。** `generate_cf_html` 会重新计算偏移并可能整体包一层 `<body>`（`clipboard_ops.rs:871-913`），因此**粘贴出去的 HTML 与采集到的原始 HTML 不逐字节相同**；对比实验必须用「采集态 html_content」而不是「粘贴后 CF_HTML」。
- **A3. 命名格式存在硬上限与静默丢弃。** 数量 12 / 单格式 1.5 MB / 合计 4 MB（`mod.rs:22-24`），超限在枚举阶段直接 `continue`（`win_clipboard.rs:609-615`），无用户可见提示。
- **A4. 表格源识别是「子串匹配」而非白名单枚举。** `is_likely_spreadsheet_source` 使用 `contains("excel" | "et.exe" | "wps" | "spreadsheet" | "calc" | "numbers")`（`mod.rs:277`），路径或窗口标题含这些子串的非表格进程会被误判为表格源，从而改变命名格式保留策略。**待实测**：是否存在真实误判样本。
- **A5. WPS Writer 源被有意区别对待。** 保守采集会跳过位图回退并清空命名格式（`mod.rs:868-883`）；若 WPS Writer 的表格只靠 RTF 承载，迁移路径必然丢失（**待实测**该假设）。
- **A6. 预览与数据分离：预览截断不等于数据丢失。** 后端 `truncate_entry_for_ui`（`utils.rs:1305-1326`，文本 2000 字符）、`truncate_html_for_preview`（`utils.rs:1328-1428`，5000 字符 / 10 行）、前端 `sanitizeHTML` 再截断到 3 行（`src/shared/components/HtmlContent.tsx:103-123`）。**「预览少了几行」不得直接判为缺陷。**
- **A7. 前端在「疑似表格」时会改用图片快照预览。** `richTextLooksTabular` / `isSpreadsheetLikeSource` 命中后走 `preferGeneratedRichPreview` 与快照图（`src/features/clipboard/components/ClipboardItem.tsx:900-948`、正则 `ClipboardItem.tsx:55-57`），因此表格条目的「看起来像图片」是设计行为，不是采集失败。
- **A8. 默认关闭富文本采集。** `app.capture_rich_text` 默认 `false`（`database.rs:251`）；不开开关则表格结构 100% 不会入库。**这是「表格粘贴问题」最常见的一阶原因。**
- **A9. CF_HTML 头部文本识别与写出版本号不一致（静态推断，**待实测**影响面）。** 识别函数要求 `version:0.9` 或同时含 `starthtml:` 与 `startfragment:`（`utils.rs:978-982`），而写出的头部是 `Version:1.0`（`clipboard_ops.rs:915`）。仅当纯文本恰好是「带分行的 CF_HTML 头部」时才会有行为差异，实际影响未验证。

### B. 确定在来源应用侧

- **B1. 表头/结构只存在于位图的应用（截图类）永远无法还原结构。** 见 S8；TieZ 收到的是 `image`（`pipeline.rs:106`）。
- **B2. 只提供 CF_UNICODETEXT 的应用无法提供表格语义。** 若某来源（**待实测**：钉钉、部分 Web 文档）复制时只写纯文本，则 TieZ 侧最多得到 Tab/换行，无 `rowspan`/样式。
- **B3. 来源提供的 CF_HTML 头部畸形（缺 `<`、偏移错乱）属于来源问题**；TieZ 只做尽力修复（`utils.rs:672-694`、`utils.rs:2820-2869`；对应测试 `utils.rs:2052-2081`）。若实测中 HTML 恢复失败且原始字节确实畸形，归因来源侧。

### C. 必须实测才能区分（待实测）

| 编号 | 待区分的问题 | 判定实验 |
| :--- | :--- | :--- |
| C1 | 在线文档（飞书/企微/钉钉）复制时是否写 `HTML Format` | R13 + R19 |
| C2 | WPS Writer 表格是否仅靠 RTF 承载 | R10 + 剪贴板格式清单 |
| C3 | 大表（R07）丢失是命中了 1.5 MB/4 MB 上限还是探测退避耗尽 | R07 + 在 `mod.rs:359-387` 前后临时打点（灰色实验分支，勿并入主线） |
| C4 | `mod.rs:277` 的表格源子串匹配是否误判真实应用 | 收集失败样本的 `source_app` / `source_app_path` |
| C5 | `utils.rs:978-982` 的版本号差异是否造成真实退化 | 构造纯文本 = TieZ 自己 CF_HTML 头部的样本，观察 `content` |
| C6 | 是「采集时丢」还是「粘贴时丢」 | 同一条目分别做：单机预览观察 `html_content`、粘贴到 Word/Excel 观察结果，并对同一份 HTML 用第三方查看器写回做交叉验证 |

## 7. 已知历史修复的回归清单

来源：`CHANGELOG.md`（行号为本文件写作时的当前文件行）。

| 编号 | 历史条目（CHANGELOG 行） | 回归验证动作 | 通过判据 |
| :--- | :--- | :--- | :--- |
| G1 | 富文本相邻块元素重复插入空行（`CHANGELOG.md:27`） | 用例 R01/R05 采集后，观察 `content` 中的空行数；对照单测契约 `utils.rs:1523-1591`（尤其 `rich_text_extraction_table_cells_and_rows`，`utils.rs:1586-1591` 期望 `Alpha\nBeta\nGamma`） | 表格 `<td>` 之间只产生单个换行；显式空段落才保留一个空行 |
| G2 | 富文本粘贴不再回退到预览快照图（`CHANGELOG.md:64`） | 用例 R01 的「带格式粘贴」到 Word | 目标得到可编辑表格，而不是一张图片 |
| G3 | 动态 WebP 不被转成静态 PNG（`CHANGELOG.md:11`） | 复制含动态 WebP 的网页表格单元格 → 采集 → 粘贴到聊天工具 | 条目仍为动画；`should_attach_png_clipboard_format` 对动画返回 `false`（`clipboard_ops.rs:952-954`） |
| G4 | QQ 文件剪贴板里的 `.jpg` 实为多帧 GIF（`CHANGELOG.md:17`） | 在 QQ 中复制该图片 → 采集 | `single_gif_file_data_url` 按字节签名判定（`mod.rs:124-148`），条目为 `image` 且保留动画 |
| G5 | GIF 粘贴仍变 PNG（`CHANGELOG.md:18`） | 采集任意 GIF → 粘贴到画图/浏览器 | 不附带 PNG 剪贴板格式（`clipboard_ops.rs:985-988`、`win_clipboard.rs:750-1102`） |
| G6 | GIF 判定引入的回归：富文本被误降级（`CHANGELOG.md:19`） | 用例 R12：复制「含正文 + 多张图片」的网页表格 | 条目必须仍是 `rich_text`；`should_prefer_image_over_rich_text` 对富文本源返回 `false`（`mod.rs:103-105`），测试 `mod.rs:1470-1484` |
| G7 | 浏览器动态图从 CF_HTML 恢复（`CHANGELOG.md:25`） | 网页中复制动态图 → 采集 | `extract_animated_image_data_url_from_html`（测试 `utils.rs:1874-2013`）能还原动画 |
| G8 | WPS 纯文本噪声不再覆盖可渲染 HTML（`CHANGELOG.md` 相关行为，测试 `utils.rs:1706-1723`） | 用例 R09 采集后观察 `content` | `content` 不含 `MicrosoftInternetExplorer4 / DocumentNotSpecified / Normal 0` 前缀 |
| G9 | 表格 HTML 预览仍是合法表格（`utils.rs:1770-1782`） | 用例 R02 观察预览 | 截断结果 `starts_with("<table")` 且 `ends_with("</table>")` |
| G10 | 富文本命名格式往返不破坏 HTML（`utils.rs:1830-1871`） | 用例 R09/R11 采集 → 带格式粘贴 | 粘贴后 HTML 与采集态 HTML 等价，且命名格式被还原 |
| G11 | 富文本快照预览仍在（`CHANGELOG.md` 未列，但默认开启 `database.rs:255`） | 用例 R02 切换「富文本快照预览」开关前后对比 | 开关只改预览形态，不改 `html_content` |

## 8. 不确定性

以下结论均为**静态推断**，未做任何真机验证：

1. 矩阵第 3 列中标注「待实测」的来源格式集合（尤其 Excel 命名格式具体名单、钉钉/飞书/企微的 HTML 形态）来自对 Windows 剪贴板契约与 TieZ 代码路径的推断，未实测。
2. §6 中 A4（子串匹配误判）、A5（WPS Writer 表格仅靠 RTF）、A9（CF_HTML 版本号差异）三个结论只由代码阅读得出，影响面未量化。
3. §6 A3 的「静默丢弃」是读代码得到的推断（`win_clipboard.rs:609-615` 无日志、无返回值区分），未在真机确认用户完全无感知。
4. 复现步骤中的**预期结果**是依据代码契约书写，不是实测结果；任何一条在真机上不成立，都应先按 §6 C6 区分「采集时丢」与「粘贴时丢」，再更新本文件。
5. 本仓库现有集成测试只有 Linux 侧（`src-tauri/tests/linux_clipboard_integration.rs:1` 起 `#![cfg(target_os = "linux")]`，覆盖文本/HTML/图片/文件四类往返，`linux_clipboard_integration.rs:13-69`），**Windows 表格往返没有自动化覆盖**；本文件所列全部 R** 用例在补齐自动化前都是人工回归。
6. 行号引用基于 `HEAD = 2d03ed0`；任何一次入库改动都可能让行号漂移，引用前请重新核对。

## 9. 建议的实验/灰度落地方式

1. 先做 R19（格式清单采集），把矩阵第 3 列的「待实测」全部转成事实，再开始判定缺陷。
2. 再做 R01–R08（Excel 主链路），因为 Excel 是唯一同时提供 CF_UNICODETEXT + CF_HTML + RTF + 二进制命名格式的来源，最能把 TieZ 侧问题与来源侧问题分离。
3. R09–R11（WPS/Word）用于验证 §6 A1/A5 两条「确定在 TieZ 侧」的结论。
4. R12–R13（浏览器/在线文档）用于覆盖 §6 C1。
5. R16–R17（截图）作为「来源侧确定」的对照，出现结构丢失时不应计入 TieZ 缺陷。
