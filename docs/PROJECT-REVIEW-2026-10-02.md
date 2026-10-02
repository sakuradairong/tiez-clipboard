# TieZ 项目审查 — 2026-10-02

第一阶段审查以 `master` / `2d03ed0` 为基线，包含原有 7 个未提交文件的剪贴板/WebP 改动，确认 **6 项 P1、8 项 P2**。用户随后授权处理，本报告末尾记录实现和复核结果。以下问题描述、源码行号及初始测试数量保留修复前快照；原有 WebP 改动已保留。

P1 表示需要优先修复的安全、隐私或数据丢失问题；P2 表示有明确触发条件的功能、可靠性或隐私一致性缺陷。下文区分本地复现和源码确认，未把通过单元测试解释为完成原生桌面验证。

## 覆盖与验证

审查范围包括 React 状态与事件、搜索和分页、敏感内容显示、紧凑预览、Windows 剪贴板采集与粘贴、窗口与热键调用、SQLite 与迁移、标签与加密、OCR、备份恢复、WebDAV/云同步、MQTT、接力密钥与收据、文件传输、Tauri 命令注册/能力配置和 Windows 发布/卸载流程。未提交的实验目录不作为生产功能验收对象。

| 验证 | 结果 |
| --- | --- |
| `npm run build` | 通过；保留已有 `@tauri-apps/api/event` 混合静态/动态导入警告 |
| `npx vitest run` | 5 个测试文件、43 项测试通过 |
| `cargo test --manifest-path src-tauri/Cargo.toml` | Windows 后端 227 项测试通过；库/文档测试及仅 Linux 的集成测试在此主机上为 0 项 |
| Windows Rust 独立验证 | 确认命令参数解析、备份路径语义、空剪贴板哈希及 MQTT 退避溢出 |
| 前端真实 hook 的隔离验证 | 确认负 ID 分页停滞和 IME Enter 误粘贴；确认敏感标签大小写判断差异 |
| 原生交互/安装器 | 没有运行应用、改变用户剪贴板、注入真实窗口输入或执行卸载 |

构建和测试第一次受到沙箱子进程限制；获准运行后上述命令均通过。这些环境限制不是项目构建缺陷。

GitNexus 已用于查询关键符号的调用上下文。现存索引停留在 `3451a91`，概念查询提示 FTS 缺失；使用缓存 CLI 的 `context` 能查到关键调用链。尝试 `analyze --index-only --workers 4` 刷新时，实验目录的生成/第三方 C++ 头文件多次解析超时，刷新未完成。因此本报告没有把空图结果视为没有依赖，所有引用行号均来自当前源码；没有完成 PDG 污点分析。

## 需要优先修复的 P1

### 1. 打开 URL 时进入 Windows 命令解释器

位置：[content_handler.rs:179](../src-tauri/src/services/content_handler.rs)。

未配置专用 URL 程序时，用户点击 URL 条目的“打开”，经过 `open_content → handle_url_content → launch_default_handler` 执行 `cmd /C start "" content`。无空格且带 `&` 的 URL 参数会被拆成命令语法，普通查询参数也会被截断。Rust 的常规参数编码不能保护 `cmd.exe` 的特殊解析方式。

验证：使用项目锁定的 Rust 1.88 编译临时程序，将 `start` 替换为无副作用的 `echo`，保留相同参数结构。带 `&ver` 的参数被拆为两条命令，输出 URL 前半段后又输出 Windows 版本。没有打开实际 URL 或执行应用。

建议复用已有原生 `ShellExecuteW`/受控 opener 来打开经过 scheme 校验的 URL，移除 shell 解析。此判断与 [Rust Command 的 Windows 参数警告](https://doc.rust-lang.org/std/process/struct.Command.html#method.arg)及 [Microsoft cmd 特殊字符说明](https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/cmd)一致。

### 2. 备份恢复允许 Windows 路径逃出暂存目录

位置：[backup.rs:163](../src-tauri/src/services/backup.rs)、[backup.rs:223](../src-tauri/src/services/backup.rs)。

清单路径校验只拒绝 `is_absolute()` 和 `ParentDir`，解压随后直接 `destination.join(path)` 并 `File::create()`。Windows 的根相对路径和盘符相对路径均可通过校验，却使输出离开恢复暂存目录。合法数据库和匹配 SHA-256 不会消除这一风险；后续数据库检查失败也不能撤销先前的越界写入。

原生 Windows Rust 验证结果：`C:outside.txt` 的 `is_absolute=false`、无 `ParentDir`，拼接后仍是 `C:outside.txt`；`\outside.txt` 同样通过判断，拼接后成为 `C:\outside.txt`。验证只检查路径运算，没有写入这些位置。[Rust Path 文档](https://doc.rust-lang.org/std/path/struct.Path.html#method.is_absolute)也区分 Windows 的根路径和绝对路径。

建议仅允许普通相对路径组件，拒绝 `RootDir`、`Prefix`，限定备份允许恢复的顶层条目，并在创建文件前验证最终路径受暂存目录约束。

### 3. 富文本图片引用能读取并同步非图片本地文件

位置：[utils.rs:2688](../src-tauri/src/services/clipboard/utils.rs)、[cloud_sync.rs:574](../src-tauri/src/services/cloud_sync.rs)、[cloud_sync.rs:819](../src-tauri/src/services/cloud_sync.rs)。

采集到的 HTML 中，`<img src>` 指向的存在路径会被直接读取并复制到附件目录，没有实际图片格式验证或可信来源限制。同步富文本时，相关文件又被读取并转换成 data URL。由不可信 CF_HTML 指定的账户可读文件，即使是文本文件，只要内容非空且不超过 8 MiB，在富文本同步已启用时也可能上传到 WebDAV/云同步目标；用户无需主动选择该文件。

证据为采集流水线、附件化和同步转换的完整源码路径，尚未做真实剪贴板到服务器的交互复现。建议在采集和同步两处建立资源来源边界，并验证实际图片字节与大小；仅信任扩展名或 MIME 名称不足以保护本地文件读取。

### 4. 紧凑悬停预览绕过敏感遮罩

位置：[ClipboardItem.tsx:1025](../src/features/clipboard/components/ClipboardItem.tsx)、[ClipboardItem.tsx:1141](../src/features/clipboard/components/ClipboardItem.tsx)。

开启隐私保护和紧凑模式、关闭条目的 AI 选项菜单后，对非文件类型的已遮罩敏感条目悬停 1 秒，预览逻辑仍将原始 `content`、`preview`、`html_content` 发送到紧凑预览窗并直接显示。预览启用条件及发送路径都未检查 `isSensitiveHidden`，无需点击揭示按钮即可看到内容。

源码链路已确认，未运行原生预览窗。建议在发送内容前检查当前揭示状态，在预览窗保留隐私语义，并在条目或隐私状态变化时取消待显示请求、清除已有预览。

### 5. Windows 粘贴失败后仍可能删除记录或消费队列

位置：[clipboard_ops.rs:1261](../src-tauri/src/services/clipboard_ops.rs)、[clipboard_ops.rs:1128](../src-tauri/src/services/clipboard_ops.rs)、[clipboard_ops.rs:1617](../src-tauri/src/services/clipboard_ops.rs)。

Windows 的 `SendInput` 返回数量被忽略，`send_paste_keystroke` 最终无条件返回成功。普通权限 TieZ 向管理员权限编辑器粘贴时，输入可能被 UIPI 阻止；开启“粘贴后删除”时，上层仍执行删除。未固定、无标签的记录及符合清理条件的附件可能被删除；顺序粘贴也会消费队列条目。

调用与删除控制流已确认，未向真实管理员窗口发送输入。UIPI 限制和返回数量语义由 [Microsoft SendInput 文档](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput)明确规定。建议逐次验证发送数量，失败时返回错误并保留记录和队列；主动中止的粘贴路径也应返回失败。

### 6. 卸载脚本递归删除安装目录中的用户数据

位置：[uninstall.nsh:72](../src-tauri/nsis/uninstall.nsh)、[setup.rs:197](../src-tauri/src/app/setup.rs)。

卸载后启动的清理助手对整个 `$INSTDIR` 执行递归删除。应用明确支持可执行文件旁 `data/` 目录作为便携存储，也支持把数据路径重定向到安装目录内。这样的数据库、附件或用户放入目录的其他文件会被一并删除，脚本没有检查数据标记、保护目录或请求删除用户数据。

此结论来自卸载脚本与存储目录选择的交叉检查；未实际卸载。建议只删除安装器拥有的文件，对便携和重定向数据目录采取与现有迁移清理相同的保护机制。

## 有明确触发条件的 P2

### 7. 标签大小写与后端敏感性判断不一致

位置：[useClipboardItemRenderer.tsx:100](../src/shared/hooks/useClipboardItemRenderer.tsx)、[database.rs:36](../src-tauri/src/database.rs)。

添加 `Password`、`PASSWORD` 或 `Sensitive` 时，Rust 按敏感标签处理；隐私保护已开启且条目未被主动揭示时，前端精确 `includes` 判断却不遮罩，且缺少对应揭示按钮。真实前端判断表达式验证了这一差异。建议前端统一大小写不敏感的敏感标签判断，并覆盖主列表、条目操作及预览。

### 8. 全局标签重命名跳过敏感数据转换

位置：[tag_repo.rs:192](../src-tauri/src/infrastructure/repository/tag_repo.rs)、[history_cmd.rs:262](../src-tauri/src/app/commands/history_cmd.rs)。

UI 允许将普通标签重命名为敏感标签。该命令只更新标签关系和 JSON，没有像 `update_tags` 一样排队加密或删除明文 OCR 索引。历史读取的事后加密对齐不能保证立即覆盖全部批量条目，也不能清除这些 OCR 数据。建议复用敏感性转换逻辑，对受影响条目统一处理。

### 9. OCR 任务完成后使用过期敏感状态持久化

位置：[image_analysis.rs:266](../src-tauri/src/services/image_analysis.rs)、[image_analysis.rs:287](../src-tauri/src/services/image_analysis.rs)。

普通图片开始识别后，用户在识别期间将其标为敏感，标签命令会删除 OCR 缓存；正在运行的任务完成后却依据识别开始前的标签状态重新写入明文 OCR/二维码。源码可确定该竞争时序，未做桌面耗时识别复现。建议写入时在同一数据库锁/事务中重新检查条目、内容哈希和敏感标签。

### 10. 非持久历史刷新后无法翻过前 80 条

位置：[useHistoryFetch.ts:113](../src/shared/hooks/useHistoryFetch.ts)、[useHistoryFetch.ts:209](../src/shared/hooks/useHistoryFetch.ts)。

非持久模式积累超过 80 条负 ID 记录，删除记录或切换过滤条件刷新后，如果第一页 80 条全部为负 ID，当前只按正 ID 累加偏移的逻辑使结果保持 0。下一次加载重复第一页，重复偏移保护随后阻止加载。真实 hook 加模拟后端验证：200 条记录只显示 80 条，`offset=0`、`hasMore=true`，请求偏移为 `[0,0]`。建议明确后端混合会话/数据库分页契约，返回可推进的游标或分别维护偏移。

### 11. 输入法确认候选时误触发历史粘贴

位置：[useKeyboardNavigation.ts:170](../src/shared/hooks/useKeyboardNavigation.ts)。

搜索框进入键盘选择模式后，用中文输入法按 Enter 确认候选，handler 不检查 `isComposing`，会阻止默认行为；此时若仍有有效选中的历史记录，还会调用该记录的粘贴。对真实 handler 注入 `Enter + isComposing=true + keyCode=229` 的隔离验证确认了默认行为被阻止及粘贴调用。建议在处理快捷键前排除输入法组合阶段。

### 12. 快速连续复制不同文件被错误去重

位置：[clipboard/mod.rs:705](../src-tauri/src/services/clipboard/mod.rs)。

两秒预检只哈希文本和位图，尚未读取 `CF_HDROP` 文件列表。开启文件捕获时，资源管理器的纯文件复制得到相同的非零空哈希，两秒内第二次复制不同文件会提前返回，实际文件路径比较无法执行，造成历史漏项。Rust 1.88 验证两次空哈希均为 `15130871412783076140`，不同路径哈希则不同。建议预检纳入文件列表，或无可哈希格式时跳过内容去重。

### 13. MQTT 长时间失败后退避溢出

位置：[mqtt_sub.rs:431](../src-tauri/src/services/mqtt_sub.rs)。

退避公式先算无限制的 `5 * 2^(attempt-1)`，之后才 `.min(60)`。连续失败累积到第 63 次时 debug 运算 panic；release 从第 65 次开始退避为 0，连接立即拒绝时造成高频重试与日志增长。独立 Rust 程序在 debug/release 两种编译模式中确认此结果。按当前退避约一小时即可积累至该次数，连接超时会增加时间。建议先限制指数或使用饱和运算。

### 14. 卸载无条件修改用户的 Windows 剪贴板隐私设置

位置：[uninstall.nsh:23](../src-tauri/nsis/uninstall.nsh)、[uninstall.nsh:50](../src-tauri/nsis/uninstall.nsh)。

卸载脚本无条件启用 Windows 剪贴板历史和云剪贴板，并删除相关禁用策略；没有记录用户安装前的值或判断这些值是否由 TieZ 修改。用户主动关闭的功能和现有策略会被改变。源码确认，未执行注册表操作。建议恢复 TieZ 修改前保存的值，仅撤销由本应用拥有的设置。

## 依赖与发布检查

`npm audit` 报告 44 个受影响依赖条目：critical 1、high 21、moderate 19、low 3。`npm audit --omit=dev` 为 6 个 moderate 条目，来自 `react-select → @emotion → babel-plugin-macros → cosmiconfig → yaml` 链；底层公告是 [yaml 深度嵌套导致栈溢出](https://github.com/advisories/GHSA-48c2-rrv3-qjmp)。这不是 6 个独立运行时攻击入口。本次未确认这些工具依赖在打包应用中存在可到达的攻击路径，因此未按 npm 等级直接增加 P1 数量。

完整扫描中的 Vitest critical 条目有其服务端/UI 启用条件；当前执行的是 `vitest run`，不能据此声称已发布桌面应用存在对应远程代码执行入口。建议集中安排依赖更新及锁文件验证。扫描原始结果保存在本地诊断产物 `.mimosa/review-npm-audit-all.json` 和 `.mimosa/review-npm-audit-production.json`，不随代码提交。本机没有可用 `cargo-audit`，未完成 RustSec 公告扫描。

当前应用版本和 Windows release preflight、锁文件安装、测试步骤及签名配置已检查；没有发布、签名或启用新服务。安装器实际执行仍需在一次性 Windows 环境验证。Linux/macOS 没有进行原生运行或打包验收。

审查时工作树中的动画 WebP/GIF 改动通过全部现有测试，审查没有发现可确认的新增回归；QQ/WPS 原生延迟渲染及真实目标应用粘贴仍需专门的 Windows 交互验收。上述发现主要位于现有已提交实现，不能归因于本次工作树的 WebP 修改。

## 建议修复顺序与回归验收

1. 移除 URL shell 执行，收紧恢复路径与富文本本地资源读取边界。
2. 保护卸载用户数据，对粘贴失败停止删除/消费队列，补齐紧凑预览的敏感状态。
3. 统一标签敏感性转换与 OCR 写入检查，再修复分页、输入法、文件去重及 MQTT 退避。
4. 为上述触发条件增加窄回归测试，在一次性 Windows 环境验证管理员目标窗口、真实 CF_HDROP、紧凑预览和 NSIS 卸载。

初始审查的原生交互边界和依赖扫描限制仍需在发布验收中补齐。调用图在修复阶段已成功刷新，见下文。

## 修复与复核 — 2026-10-02

14 项发现均已有代码修复，并增加窄回归测试。交叉复核额外补齐了接收图片的受管副本、过期加解密任务、目标前台恢复失败及部分按键抬起失败。本节记录修复验证阶段结果；该阶段未提交或发布，也未运行应用、注入真实窗口输入或执行卸载。

| 编号 | 修复实现 | 回归覆盖 |
| --- | --- | --- |
| 1 | `content_handler.rs` 使用原生默认处理程序并校验链接协议 | 保留 `&` 查询参数，拒绝文件及可执行协议、控制字符 |
| 2 | `backup.rs` 统一校验恢复路径并在解压前复查 | 根路径、盘符、ADS、设备名、大小写别名及失败保留数据 |
| 3 | 新增 `local_image_resource.rs`，采集和同步检查规范路径、可信目录、体积、实际解码格式 | 非图片、截断签名、目录逃逸、超限图片、伪装扩展名及合法图片；接收下载保留原件并存管理副本 |
| 4 | 主列表与紧凑预览同步隐私语义，异步请求取消并清除所属预览 | 已遮罩条目、隐私/内容变化、卸载及过期异步显示 |
| 5 | `clipboard_ops.rs` 验证前台恢复和全部输入数量；仅清理本次尚未抬起的键；`paste_queue.rs` 成功后才消费 | 前台不匹配、部分按下/抬起、失败重试及队列替换；失败清除最近粘贴标记 |
| 6 | 卸载钩子移除整个安装目录的递归清理，保留标准安装器文件清理 | 钩子编译；生成的 Tauri 模板确认安装目录只做非递归 `RMDir` |
| 7 | 新增 `sensitiveTags.ts`，列表、条目按钮共用大小写不敏感规则 | `Password`、`PASSWORD`、`Sensitive`、中文敏感标签与普通标签 |
| 8 | `tag_repo.rs` 事务内重命名、加解密及清理 OCR，数据库成功后再更新会话并请求同步 | 敏感标签合并、解密、不可解密数据保留、失败回滚与会话一致性 |
| 9 | OCR 写入前在同一数据库锁内重查类型、哈希和标签 | 标为敏感、删除、更换内容及正常结果持久化 |
| 10 | 前端首屏请求全部有界会话记录，数据库偏移及下一页判定仅计正 ID | 全部负 ID、混合分页、会话转持久化；原生快捷粘贴保留原返回上限 |
| 11 | 组合输入及 `keyCode=229` 在键盘动作前退出 | 候选确认不阻止默认行为、不粘贴 |
| 12 | Windows 前置哈希纳入缓存的 `CF_HDROP` 路径，无可哈希格式时跳过去重 | 不同文件列表有不同哈希，空剪贴板不生成去重哈希 |
| 13 | MQTT 先限制退避指数 | 超过历史溢出次数及最大计数仍保持 60 秒上限 |
| 14 | 系统设置修改保存原值及最后应用值，卸载只恢复精确匹配的所有权快照；更新模式保留设置和自启动 | 缺失/不完整快照、值和类型变化、已删除值；PowerShell 所有权谓词 6 项验证 |

本地图片采集仅允许管理附件、收藏表情及临时缓存目录，云同步只允许前两个管理目录；路径先解析真实位置，图片格式按字节识别并验证解码，原始动画字节保留。非受管的旧图片引用不会自动读取上传。用户主动接收且启用自动拷贝的图片会生成管理副本，原下载文件和文件剪贴板路径保留。

旧版本未记录原值的系统设置无法可靠恢复：卸载保留现值，不推测安装前配置。用户或策略在 TieZ 修改后改变的值同样保留。升级卸载跳过系统设置恢复和自启动清理，避免消耗仍供新版本使用的快照。

### 最终自动验证

| 验证 | 结果 |
| --- | --- |
| `npm run build` | 通过，保留已有 event 混合导入警告 |
| `npx vitest run` | 8 文件、61 项通过，新增 18 项 |
| `cargo test --manifest-path src-tauri/Cargo.toml --quiet` | 整合远端后 Windows 后端 268 项通过；本轮新增 39 项，远端图标提交另含 2 项；保留原有未使用函数警告 |
| NSIS / PowerShell | 更新保护后的钩子、完整现有 Tauri 安装器模板均编译通过；6 项纯所有权判断通过；未执行注册表恢复或安装/卸载 |
| GitNexus | 提交前全量重建成功：3,955 节点、9,752 关系、300 流程；`detect_changes(scope=all)` 返回全部 285 符号、83 流程，`partial=false`、`truncated=false` |
| `git diff --check` | 通过 |

完整 NSIS 编译使用现有 release 二进制，仅用于检查安装器与新钩子的组合语法，不是修复后应用的发布构建。模板保留一个未使用 `Skip` 安装函数的编译警告。

GitNexus 的最终变更分析覆盖全部 34 个暂存文件，并包含原有 WebP 改动及新增源码和测试。整合远端后的增量刷新出现 FTS 索引一致性错误，随后全量重建成功；完整分析无部分或截断结果。分析风险为 CRITICAL，反映剪贴板、快捷键、备份和同步的调用范围较大，发布前仍需覆盖下述原生场景；未完成 PDG 污点分析。完整原始结果保存于本地诊断产物 `.mimosa/review-detect-changes.json`，不随代码提交。

真实 Windows 验收仍需覆盖：管理员目标窗口粘贴失败保留历史与队列、真实 IME 候选确认、紧凑预览隐私切换、连续文件复制、QQ/WPS 动画粘贴，以及一次性环境中的升级和卸载（包含便携数据、无备份旧设置及外部策略变化）。共用实现影响其他平台，本轮没有 Linux/macOS 原生验收。依赖升级与 RustSec 扫描仍是单独待办，锁文件及版本号未变。

## 提交整理与远端整合

本轮提交包含 34 个文件：修复源码、回归测试、CHANGELOG、审查报告和 GitNexus 排除配置，同时纳入审查前已有的动画 WebP 改动。本地 `.mimosa/`、`src-tauri/.mimosa/`、`experiments/` 和之前的 `TABLE-PASTE-AUDIT.md` 保留在工作区，不随本轮提交。

提交基于远端 `master` 的 [591025d](https://github.com/sakuradairong/tiez-clipboard/commit/591025d) 整合，保留方案 A 图标和按系统主题切换托盘的更新。CHANGELOG 的唯一内容冲突已合并保留双方条目；整合后重新通过前端构建、61 项前端测试、268 项 Windows Rust 测试和完整现有 NSIS 模板编译。未创建版本标签或发布构建。
