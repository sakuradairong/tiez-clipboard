# Changelog

All notable changes to this fork will be documented in this file.

The format is inspired by [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Fixed

- WinUI 隐藏和退出改回可调用的原生按钮：窄侧栏只显示图标，宽侧栏显示文字、工具提示和可访问名称；退出进度写在独立文字上，不替换按钮内容。历史筛选和记录操作改为可收缩的行，避免默认窗口把筛选和采集动作挤出可视区域。设置和标签编辑改用主题表面，不再用固定灰底。工作区外观 35 项、布局 35 阶段、标签 74 阶段、卡片复制 39 阶段、缩略图 38 阶段、大正文 66 项和应用选择器 379 项通过，且未加载 WebView2。这是原生界面打磨，不代表迁移完成。

- 历史排序 smoke 从独立读取的全部样本元数据构造时间/次数、置顶及 ID 并列规则的期望，不再假设快速插入必有不同毫秒时间；保留 210 条全局分页、无重复、格式保留及只读断言，生产排序 SQL 未改。

- 原生紧凑预览区分显式按钮请求与普通悬停所有权，避免排队的旧行进入/退出通知取消新预览；保留隐私、版本、滚动、失焦和关闭取消，新增确定性旧通知回归及无正文状态诊断。

- 原生图标窗口验收按实际 ListViewItem/ScrollViewer 几何校正虚拟化滚动位置，避免目标图标仍在视口外时误判加载失败；保留可见性、确切行/图像源及资源上界断言，失败报告增加视口诊断。

- 修正同源远程图片测试 HTTP 服务器：Windows 接受的连接显式恢复阻塞模式，并在有界超时和请求大小限制下读完请求头再回复，避免忽略 `WouldBlock` 或分片请求后提前关闭连接。新增确定性分片请求回归，生产下载行为未改变。

- 修正焦点 smoke 夹具因临时根目录尾分隔符导致的误拒绝清理，避免异常覆盖环境不可用的退出码；清理继续要求专属 GUID 目录及非重解析点，新增 9 项不触碰焦点或剪贴板的规则检查。

- 原生托管图标响应为数组、null、标量或未定义值时统一保持 `invalid_response` 契约，不再泄漏 JSON 对象访问异常；新增 24 项解析检查及仅监控自有进程的 smoke 看门狗，未改变 Rust ABI。

- 原生缺失主快捷键配置时改用旧版数据库默认的 `Alt+C`，读取不写入；已保存的 `Win+V`、自定义组合或禁用空值保持不变，修正此前文档错误的默认值说明。

- 修正原生迁移清单漏掉带分号/嵌套泛型调用的问题，补动态 relay 命令、八类打开方式键及浏览器持久状态来源；增加提取回归测试，不读取用户存储值，也不将源码覆盖计数视作功能验收。

- 原生核心 ABI 测试夹具统一保护进程级单例，避免默认并行运行时互相触发 `already_open`；测试内部的多线程关闭、取消和锁释放断言保持不变，生产行为不变。

- 修正原生图标任务的文档与测试契约：未知字段/枚举仍在反序列化阶段报 `invalid_request`，已知类型/可用性配对错误以及超长或含控制字符的标识改由 `IconService::start` 在句柄解析后、目录与后端工作之前报 `icons_identity`；请求改为精确回显目录标识（含 handler/kind），并说明启动授权线性化于被观察到的目录代际、`poll` 在状态锁下观察取消后再检查当前代际，重叠的取消/刷新不得追溯撤销已返回结果，下一阶段 C# 需在界面采纳前独立检查自身取消与生命周期。主线程 join 关闭线程前新增有界 `is_finished` 观察，超时展开仍由夹具放行清理并明确失败而非挂起或误判成功；未改变生产行为。

- 原生应用图标非 PE 单测改用 `create_new` 在唯一自有临时路径创建夹具，同名既有路径不再被覆盖或删除，并断言再次创建失败后内容保持不变；同时澄清 COM 套间配平、取消证据与网络表述，未改变生产行为。

- 原生完整窗口回归改为等待确认框模板和按钮实际加载，避免提前查找尚未生成的按钮；保留原有超时和保存/删除断言，不改变应用对话框行为。

- 原生设置文本改用同步草稿状态更新，避免快速保存时按钮仍禁用，以及外部配置更新覆盖刚输入的草稿；恢复已保存文本会立即禁用无效保存。

### Added

- 原生新增/编辑对话框完成大正文链路验收：真实 TextBox、保存/取消按钮、未修改 CRLF、冲突草稿、敏感揭示、独立设置响应及退出前暂存取消；对话框关闭后清除大草稿控件。新增 66 项窗口专项，物理输入及完整视觉/环境验收仍待完成。

- C# 历史新增/编辑自动接入有界分块传输：后台分段编码保留 Unicode/换行，限制准入并释放每块请求门，取消清理暂存且不隐藏原生已接受结果；同步/异步关闭先排空传输再释放句柄。新增真实 DLL 专项，实际大编辑对话框验收仍待后续。

- 原生核心新增历史大内容分块 ABI：保留单次 64 KiB 请求上限，资源绑定核心与新增/编辑目标，以随机令牌、SHA-256、UTF-8 字节偏移、版本和敏感授权校验后一次性原子提交；保留负数会话 ID 和原始文本/HTML，取消、过期或退出释放有界暂存。C# 与编辑窗口接入、其他大内容协议仍待后续完成。

- 新增原生 Rust→WinUI 发布脚本及隔离资产审计：普通/portable 核心和 Debug/Release 分离，默认同时自包含 .NET 与 Windows App Runtime，验证真实 ABI、刚构建 DLL 哈希与源码收据，并排除 WebView2/旧壳运行资产。修复 unpackaged `dotnet publish` 漏应用 XBF/PRI 导致主窗口无法启动的问题。安装器、真正便携数据路径、升级回滚及干净系统验收仍未完成。

- 原生应用选择器显示真实 Windows 应用图标：绑定目录代际和完整行标识，仅可见容器持有原生位图，两个在途请求包含取消中的实际任务；刷新、筛选、回收、隐藏/最小化、重开与退出释放资源并拒绝迟到结果。单图标失败不影响选择，保留明确占位说明。新增真实窗口专项和完整回归，物理输入、混合 DPI、无障碍及无 WebView2 干净系统验收仍未完成。

- 托管 C# 新增有界应用图标客户端（`NativeCore.AppIcons.cs`）：`GetApplicationIconAsync(目录代际, 精确目录行, 边长, 取消令牌)` 绑定到某一已接受目录代际与该行的 `source_id`/`handler`/`kind`/`unavailable`（含仅推荐行），只发送这四个目录字段，不接受显示名、任意路径或调用方像素，返回只读、受请求边长与 64 KiB 限制的自有 RGBA 结果；无效边长/标识/代际在本地即报 `icons_edge`/`icons_identity`/`icons_unavailable`/`icons_catalog`，权威成员校验仍在 C ABI。准入在首次挂起前计数：最多 2 个并发加 8 个等待，第 11 个显式 `icons_busy`，取消与失败严格归还计数；后端 `icons_busy` 以联动令牌延迟最多重试 8 次，不空转也不无限重试。原生响应逐字段校验（非零 `job_id`、精确目录代际/边长/状态/回显标识、尺寸、恰为 `width*height*4` 且各值 0..255 的 JSON 数字数组），缺失或不符一律 `invalid_response`，不解码 base64、不伪造像素；内部等待不捕获调度器，原生接受后即使令牌已取消仍保留真实任务 ID 以便 `finally` 发送取消，成功返回前再次检查自身令牌与生命周期。同步与异步释放都会先排空已准入图标任务（排队/运行/取消中）再释放原生句柄，且不依赖所有者调度器或请求闸门，并明确区分托管排空与原生关闭 join Shell 工作线程。`--app-icons-only` 专项 102 项真实 DLL 检查（本机目录 394 行、259 候选；桌面 32/16/64/128 边长、6/6 打包应用、仅推荐行接受、真实过期代际与缺失标识拒绝、37 项畸形解析向量、严格准入上限、所有者校验、预取消与排队取消不创建原生任务、有界忙重试、同步/异步排空）与完整托管套件通过；构建零警告错误，窗口回归 16/379/30/139 通过且未加载 WebView2。WinUI 图标呈现、单图标失败界面、物理/DPI 与干净系统验收仍待迁移。

- 原生核心与 C ABI 新增可取消应用图标任务（`apps_icon_start`/`apps_icon_poll`/`apps_icon_cancel`）：请求只携带当前已接受目录快照中的精确标识（主列表或八个推荐组，`source_id`/`handler`/`kind`/`unavailable` 全等），不接受任意路径、显示名、类型或像素，标识在原生接受点重建自目录字段。边长限定 1–128，位图按请求边长与 64 KiB 上限在原生接受端校验（请求 32 却返回 64×64 即 `icons_bounds`；32×32/64×64 正常返回），最多保留 2 个任务槽、第三次 `icons_busy`，任务 ID 单调且不复用；取消、迟到忽略取消的结果、缓存读/新代际/失败刷新造成的旧代际结果都不会发布像素，调用方需轮询或取消以释放槽位。关闭时先向扫描与图标任务分别发出取消信号，再在注册表/状态锁之外 join。默认与便携核心测试各 137 项（含 15 项图标任务与 1 项目录信号/join）串行通过；C# 图标接口、WinUI 图标呈现、物理/DPI 与干净系统验收仍待迁移，本阶段未新增真实 DLL 图标客户端验收。

- 原生 Windows 平台新增无 Tauri 的应用图标位图后端：只接受共享目录中的桌面/打包应用标识，通过 Shell `IShellItemImageFactory`（ICONONLY，打包应用走 `shell:AppsFolder`）同步提取受限 RGBA 位图（边长 1–128，最多 64 KiB），正确翻转 BGRA 行序并还原预乘透明通道；拒绝不透明桌面 ID、UNC/相对路径、脚本与非 PE 可执行文件，区分取消/不可用/失败而不伪造占位图标，COM 套间、位图与 DC 全部按作用域释放且不改动调用方 STA/MTA。9 项单测、真实 STA/MTA 调用方套间及真实桌面/打包图标读取（本机 6 个打包应用全部成功，预热后 GDI 与进程句柄保持不变）通过，未启动应用、未接触剪贴板、未加载 WebView2。核心 ABI、C# 与 WinUI 虚拟化选择器接入仍待迁移，不代表界面图标迁移完成。

- 原生设置新增可搜索的应用程序选择器：八类记录打开方式可浏览共享核心应用目录，按路径/名称/不透明 SourceId 大小写不敏感搜索，保留推荐/默认标签、不可用原因（不可确认）与显式刷新；确认只更新草稿，显式保存仍走设置事务与版本检查。取消与外部版本冲突保留草稿，迟到结果及关闭/退出清理受保护。379 项自动化检查及 63/16/139/30 项回归通过，均未加载 WebView2；应用图标、旧脚本、广泛激活、外部回写、物理/IME/DPI/无障碍与干净 Windows 仍待验收。

- 原生应用目录接入共享核心、C ABI 和异步 C# 调用：单个受控扫描任务支持取消、轮询、内存缓存、显式刷新和任务版本检查，退出时取消并释放真实工作线程。默认/便携各 121 项核心测试、26 项真实 DLL 检查和 16 项真实窗口检查通过，覆盖只读状态、缓存、取消及响应式退出；可搜索的应用选择界面和图标仍待迁移。

- 新增原生 Windows 应用目录与关联推荐后端，通过 Shell COM 解析真实桌面程序路径和打包应用标识，替代 PowerShell 扫描方式；保留不可用项目原因，报告读取失败、未解析关联和数量上限。42 项平台单测及真实 STA/MTA 目录、八类推荐、COM 平衡和重复读取检查通过。此阶段尚未接入核心 ABI 和 WinUI 应用列表，不代表应用选择迁移完成。

- 原生独立设置新增八类记录打开应用的草稿、恢复默认、手动路径/打包标识及 Windows 原生 .exe 文件选择器；与普通设置共用事务和版本检查，保留未改动的旧配置，校验/数据库失败和冲突保留草稿。取消、重载、配置变化及窗口关闭后迟到的选择不会覆盖当前值。63 项真实窗口检查通过原生选择器取消、事务回滚和自有编辑器启动；已安装应用目录/推荐、脚本兼容、编辑回写及干净系统验收仍待完成。

- 原生历史工具栏、普通卡片和紧凑卡片新增真实打开操作，绑定核心记录 ID/版本，保留公开卡片的当前多选，敏感记录需先单独显示详情；查询、布局、导航、隐藏、回收和退出取消排队请求，已提交的真实结果不被取消覆盖。临时副本明确提示修改不会回写；编辑器已打开但计数失败单独显示警告，不误导重复启动。202 项窗口检查通过 9 次自有编辑器启动、原样文本/HTML/PNG/文件、敏感授权、7 类排队取消、真实冲突和计数 SQL 失败及退出清理；构建零警告错误且未加载 WebView2。应用选择、编辑回写、物理输入/DPI 和干净系统验收仍待完成。

- 原生外部打开成功后新增使用次数与历史变更通知：持久化记录仅事务更新次数/同步标记，临时记录仅更新内存，保留原始内容和采集时间。启动失败不再像旧版预先计数；目标在启动期间被修改、删除、提升或复用时拒绝错误记账。已启动但计数失败会单独报告，保留导出和清理所有权，不误报启动失败或自动重开。默认/便携各 110 项核心单测及各 5 次真实进程验证通过，覆盖真实 C ABI 的计数 SQL 部分失败；C# 计数/版本/通知回归通过且构建零警告错误。WinUI 打开入口、应用选择、编辑回写及干净系统验收仍待迁移。

- WinUI 新增共享核心的定时临时文件清理与异步退出进度/重试：隐藏到托盘后仍轮询，退出时关闭借用窗口和预览，系统关闭请求等待释放成功；文件占用失败时保留可见原生重试按钮和核心所有权。30 项真实窗口检查通过自有编辑器、原样导出、隐藏轮询、界面响应、重复退出、WM_CLOSE、文件占用失败及解锁后按钮重试，未加载 WebView2。此前 139 阶段完整窗口回归通过；物理输入、长期过期及干净系统验收仍待完成，打开入口、应用选择和编辑回写尚未迁移。

- 原生 C# 核心新增 IAsyncDisposable：异步等待两条请求队列并在线程池停止/释放原生服务，重复退出共用同一任务；导出被占用时保留句柄供重试，成功后重复异步/同步释放安全。真实默认 DLL 验证发起不阻塞、排队取消、重复任务、实际文件占用后的失败保留及解锁重试，既有托管回归通过且构建零警告错误。WinUI 退出进度/重试、系统关闭与自动清理交互仍待专项验收。

- 原生共享运行时接入打开服务、ID/版本/敏感授权 C ABI 与 C# 独立异步队列，慢速启动和清理不占用全局注册表锁；新增 8 项持久化打开器设置，保留旧值和无改动读取/回传。关闭失败保留停止后的核心及导出所有权，只允许清理/关闭重试，C# 保留句柄与 IsClosing 状态。默认/便携核心各 102 项、两种配置打开器各 8 项测试通过；真实默认 DLL 验证两次子编辑器启动、原样内容、取消及文件占用后的关闭重试，托管/WinUI 构建零警告错误，139 阶段完整窗口回归通过且未加载 WebView2。WinUI 打开/设置控件、清理轮询与退出重试界面、使用次数和编辑回写仍待迁移，接口明确报告后两项尚不可用。

- 原生 Rust 核心新增按权威 ID/版本/敏感授权协调打开的服务及受限临时导出所有权；启动前再次检查版本，接受后不持历史锁调用 Windows。支持唯一文件、协作式过期、关闭/Drop 清理，以及文件占用/命名空间替换时保留跟踪并重试，失败不丢失原错误或修改源内容。默认/便携核心各 100 项（含 12 项专项）通过，两种构建各验证 3 次真实核心到桌面探针启动与清理；UUID 固定为支持 Rust 1.88 的版本。此服务尚未接入 CoreRuntime/C ABI/C#/WinUI，打开器设置、使用次数和编辑回写仍待实现，不代表完整打开功能已上线。

- 原生 Windows 平台新增系统默认、桌面程序和打包应用启动后端，采用 ShellExecuteExW、明确程序名的 CreateProcessW 及原生 COM 激活，不依赖 PowerShell。真实测试发现本机 CreateProcessW 仍接受批处理，现加入原生 PE 校验并拒绝批处理及伪装脚本。平台 34 项单测、12 次预热后参数/句柄严格检查、隔离系统启动与清理通过，核心默认/便携各 88 项及 DLL 构建通过。冷启动句柄差异已定位为固定 ETW/Key 对象，非逐次进程/线程句柄增长。此阶段尚未接入核心/WinUI 打开入口，临时文件与回写、旧脚本打开器兼容、关联文档及打包应用成功激活仍待迁移验收。

- 原生内容库新增不依赖 Tauri 的外部打开准备层：保留文本空白/换行、原始 HTML 及支持的图片/动画编码，映射旧版 URL、首路径与默认应用回退规则；图片计划导出为实际格式后缀，保留原文件，并沿用现有读取/解码限制。8 项专项、内容库默认/便携各 138 项、核心各 88 项测试及 DLL 构建通过。此阶段不启动程序、不写临时文件、不改变历史；实际启动、临时文件生命周期、编辑回写、应用设置、版本/敏感保护接口与 WinUI 入口仍待迁移。

- 原生普通/紧凑历史卡片新增原格式与纯文本复制按钮，直接操作不替换多选；敏感记录需单选并在详情明确显示后才可复制。操作绑定历史版本，回收/过期、对话框、忙碌与隐藏状态禁用，查询/布局/导航/隐藏及退出取消排队请求；已接受的返回不会被误报为取消。工具栏复制同步接入版本保护与刷新提示。39 阶段卡片复制、139 阶段完整窗口、35 阶段布局和 38 阶段缩略图检查通过；复制成功反馈使用受控写入替身，并验证真实 DLL 的格式准备/过期拒绝路径；未执行成功的系统剪贴板写入。构建零警告/错误且未加载 WebView2，实际格式往返、物理输入/无障碍及干净系统仍待验收。

- 原生核心复制接口新增可选历史版本；复制/粘贴的格式准备结果在实际写入前再次校验版本，准备期间的编辑、敏感标签变更或删除会阻止旧内容写入，并保留回声标记与历史状态。默认/便携核心各 88 项、真实 DLL 管理层及 139 阶段窗口回归通过，C#/WinUI 构建通过且未加载 WebView2。私有 Windows 剪贴板站因 Win32 5 无法创建，实际新接口格式往返仍未验证；卡片复制控件见后续新增项，直接粘贴/打开仍待接入。

- 原生历史卡片和紧凑卡片新增图片缩略图：仅加载视口内非敏感记录，由 Rust 生成最长边 256 像素的静态预览，最多两个在途请求，保留原始图片与动画。滚动回收、查询/布局/隐私变化、隐藏/最小化及退出会取消并释放图片，过期响应不能填回旧卡片；敏感详情授权不会自动启用缩略图，损坏图片保留文字提示。38 阶段缩略图、139 阶段完整窗口及 35 阶段布局回归通过，构建零警告/错误且未加载 WebView2；物理视觉/无障碍、DPI、性能对比及无 Runtime 干净环境仍待验收。

- 原生历史新增列表/卡片切换，普通历史与标签记录各自记住布局；卡片自适应列宽并虚拟化，切换保留多选与浏览锚点，释放隐藏视图，重新遮罩敏感正文并取消旧预览。搜索、分页、排序、标签编辑和原生详情共用现有核心；35 阶段布局窗口及 139/74/29 阶段完整/标签目录/选择器回归通过，C#/WinUI 构建零警告/错误，未加载 WebView2。卡片内图片缩略图见后续新增项；快捷操作与旧浏览器偏好导入仍待迁移，物理输入/DPI/干净系统仍待验收。

- 原生核心新增普通历史与标签记录独立的列表/卡片布局偏好，分别默认列表与卡片；读取默认值不写入，与紧凑模式互不覆盖，保存复用版本检查事务。默认/便携各 85 项核心测试通过；布局控件见上方新增项；旧浏览器本地偏好仍待迁移。

- 历史标签编辑改为 WinUI 原生全局选择器：全目录搜索、分页/颜色/数量/敏感标记、跨页保留选择及显式新标签草稿；精确保留旧名称，未加入的输入阻止保存，冲突和读取失败保留草稿，显式重读后再保存。取消、隐藏及退出释放选择器资源；29 阶段原生选择器、74 阶段标签目录和最终构建的 139 阶段完整窗口检查通过，构建零警告/错误，未加载 WebView2。物理输入、视觉/无障碍、DPI 和干净环境验收仍待完成。

- 修正原生精确标签选择对其他会话记录中旧空白名称的识别；目录可见名称可完整选入其他记录，保持来源会话及原始拼写。默认/便携核心各 84 项测试通过。

- 原生核心新增版本校验的精确标签选择接口：读取仅返回标签元数据，保留旧目录名空白，同集合保存不改变顺序或会话 ID；敏感加解密、OCR 清理及会话转存复用原事务。默认/便携各 83 项测试与真实 DLL 检查通过；原生全局标签选择界面见上方新增项。

- 原生历史与标签内记录新增时间/使用频率排序及次数显示，普通历史保留默认置顶优先；主排序值相同时保留旧版置顶顺序。核心先合并持久化与会话元数据再分页，按实际处理数推进游标，复用原文/敏感内容保护且读取不保存设置。默认/便携核心各 79 项、真实 DLL 210 条记录检查、74 阶段原生窗口及最终 DLL 的 139 阶段完整回归通过，未加载 WebView2；真实粘贴后计数、多屏/输入法和无运行时环境仍待验收。

- 原生标签目录新增全量 Unicode 字面搜索、回车提交与清除；先过滤再排序分页，保留旧名称空白和颜色，百分号/下划线不作为通配符。已应用条件与待搜索输入独立保留，未保存标签草稿不会被覆盖，读取不保存设置。默认/便携核心各 75 项、真实 DLL、63 阶段原生标签窗口及 139 阶段完整回归通过，未加载 WebView2；标签内排序见后续新增项，卡片视图和完整运行验收仍待完成。

- 原生标签目录恢复按匹配历史数量降序排列，可切换为名称顺序；核心合并本地与会话数量后对完整目录排序，再执行分页，数量相同按原名称稳定排序。切换刷新已加载范围并尽量保留选择，不覆盖未保存草稿，不保存设置。默认/便携核心各 73 项、真实 DLL、52 阶段原生标签窗口及修正就绪检查后的 139 阶段完整回归通过，构建零警告/错误且未加载 WebView2；标签内时间/使用频率排序、目录搜索及列表/卡片视图仍待迁移。

- 原生标签页新增删除标签及其全部匹配历史的确认框，展示真实本地/会话数量，明确包含置顶、多标签和敏感记录；取消、隐藏或过期确认不会删除。核心事务覆盖全部匹配记录、同步墓碑和标签元数据，提交后更新会话与版本并保护共享附件；清理错误与已完成删除分别反馈。默认/便携核心各 71 项、真实 DLL、45 阶段标签窗口及 139 阶段完整窗口回归通过，未加载 WebView2；标签内时间/频率排序、同步服务和完整运行验收仍待完成。

- 原生标签页新增重命名与合并确认，保留名称草稿，过期确认拒绝保存；核心复用原仓储事务保留目标颜色、合并重复标签，并处理敏感加密/解密及 OCR 清理。会话负数 ID 保持，提交失败不改变会话或历史版本。默认/便携核心各 66 项串行测试、真实 DLL 与 29 阶段标签窗口检查通过，未加载 WebView2；标签删除见上方新增项，排序仍待迁移。

- WinUI 主导航新增原生标签页：分页与虚拟化目录、使用计数、创建、颜色选择器/文字编辑/恢复默认及精确名称历史筛选；切换页面和隐藏时保留草稿，外部更新明确提示冲突，失败不丢输入，退出清理标签资源。支持常用 HEX、RGB/RGBA 和命名颜色，无法预览的旧颜色原值保留；颜色变量映射、标签重命名和删除仍待迁移。16 阶段标签窗口检查与 139 阶段完整窗口回归通过，构建零警告/错误，未加载 WebView2；物理输入、混合 DPI 和无运行时环境仍待验收。

- 原生 Rust 核心与 C# 调用层新增全局标签目录、持久化/会话计数、创建及颜色接口；保留默认敏感标签、零使用标签和原名称/颜色字符串，分页与修改检查共享版本，失败提交回滚，重复操作不推进版本，读取不保存设置。默认/便携核心各 62 项串行测试及真实 DLL 检查通过；标签管理界面、重命名和会删除相关历史条目的标签删除流程仍待迁移。

- WinUI 全局和按应用清理规则新增逐条原生编辑：匹配、替换、说明、忽略动作及增删排序；保留原始注释、换行和未改动文本，恢复原值不制造格式变化，兼容文本仍可修正。危险的分隔符/注释开头和实际换行输入明确拒绝，修复单行控件提前吞掉换行的问题。构建零警告/错误，12 阶段独立规则窗口检查通过保存拒绝/修正、预览、重开和退出清理且未加载 WebView2；本轮全量窗口回归仍在紧凑表格预览复用检查失败，完整迁移与交互验收尚未完成。

- WinUI 全局与按应用清理规则新增样本测试预览，复用 Rust 捕获组/替换语义，明确显示忽略采集结果；样本、规则、匹配次数和输出受限，修改或关闭会取消旧请求并清理样本，不保存配置或操作剪贴板。默认/便携核心各 57 项串行测试、真实 DLL 和 128 阶段窗口检查通过，未加载 WebView2；应用匹配和其他策略组合的模拟仍待完成。

- WinUI 高级设置新增按应用清理策略表单，支持添加、删除、排序、应用匹配、启停、忽略/清理、类型和规则编辑；保留未知扩展字段、策略 ID、缺省字段及原换行，恢复原值不会留下格式变化草稿。兼容 JSON 编辑保留，错误配置明确提示并由核心整体拒绝保存；构建零警告/错误，119 阶段真实窗口检查通过，关闭/退出时清除编辑器缓存且未加载 WebView2；应用列表选择与结构化规则预览仍待迁移。

- WinUI 将 4 项隐私与清理规则移入独立高级设置窗口，普通设置保留 12 项；两个窗口共享核心、独立保留草稿，规则无效或版本冲突时拒绝保存，未改动规则保留原换行，有草稿时也跟随已接受主题。112 阶段真实窗口检查通过且未加载 WebView2；前一轮紧凑预览重开失败未在诊断轮复现，原因仍待确认，结构化应用策略编辑和完整设置迁移继续推进。

- WinUI 紧凑模式新增独立原生预览窗口、一秒悬停/命令入口、窗口复用和隐藏/隐私/切换时的清理，默认关闭。103 阶段真实窗口检查通过文本/图片/表格、敏感揭示、设置同步、关闭重开和退出清理，显示瞬间保持前台焦点，未加载 WebView2；物理鼠标和混合 DPI 验收仍待完成。

- 原生核心新增带历史版本和敏感揭示校验的预览条目读取接口，并支持现有 `app.compact_mode` 布尔配置，缺失时只读返回关闭状态；默认/便携核心各 53 项测试和真实 DLL 检查通过。紧凑预览窗口的交互验收独立推进。

- WinUI 新增无浏览器的富文本结构化预览，以原生文字、链接和网格控件显示标题、强调、列表、预格式化文本及合并单元格表格；可展开原始文本，明确提示样式简化、嵌入对象缺失和截断。保留原始 HTML/RTF/Office 载荷，限制解析与输出规模，敏感变化或切换会清除预览。内容测试默认/便携各 130 项、核心各 51 项、真实 DLL 与 83 阶段窗口检查通过；复杂 CSS、嵌入媒体和 RTF 专用显示仍待迁移。

- WinUI 历史详情新增受尺寸与内存限制的原生图片预览，支持透明度和缩放；敏感图片需显式揭示，切换、多选、搜索和外部隐私变化会清除旧图。GIF/WebP 动画仅显示带提示的静态首帧，原始内容保持不变；内容测试默认/便携各 123 项、核心各 49 项、真实 DLL 与 75 阶段窗口检查通过，未加载 WebView2。全分辨率查看与动画播放仍待迁移。

- 原生端新增清空普通历史：确认框展示全库范围内的本地/会话记录数量，保留置顶和带标签条目，预览过期时拒绝执行；删除失败或提交失败保留原状态，批量清空使用有界重同步通知。提交后尝试压缩数据库，压缩失败单独警告且明确删除已生效；47 项核心测试（默认/便携各一套）、真实 DLL 和 68 阶段窗口检查通过。

- 原生历史支持 Ctrl/Shift 多选与确认删除，每次最多 1000 条；混合会话/持久化记录作为一个事务和版本处理，确认期间历史变化会拒绝删除，提交失败保留记录和附件，仍有引用的共享附件会保留。多选时禁用单条操作，搜索变化立即清除旧选择；61 阶段真实窗口和 Rust/真实 DLL 检查通过。

- 原生端新增主窗口键盘快捷键设置、注册与禁用；保留 Win+V 默认值并报告系统占用，冲突或数据库提交失败保留原绑定和配置，提交后清理失败明确提示。操作队列保留连续切换的顺序；平台和真实 DLL 检查通过，WinUI 54 阶段窗口检查验证保存、通知唤起和禁用，鼠标中键和其他快捷键仍待迁移。

- 原生核心新增托盘图标、显示/设置/退出菜单和有界操作事件；WinUI 在托盘可用时关闭主窗口转为常驻，隐藏期间继续采集并可独立打开设置，明确退出时清理核心。真实 DLL 与 47 阶段窗口检查验证注册和生命周期；实际托盘点击、菜单交互及 Explorer 重启恢复仍待验收。

- WinUI 新增原始格式/纯文本粘贴入口，经 Rust 核心核验目标、按键状态及剪贴板序列后发送 Ctrl+V；部分输入失败只清理已接受的自有按键，并保留历史。无法核验新的外部窗口时使旧目标失效。共享按键状态规则的旧版回归和原生拒绝路径检查通过；目标程序实际接收、其他粘贴方式及粘贴后策略尚未验收。

- WinUI 主窗口新增输入目标状态和返回窗口操作；Rust 核心拥有前台/销毁事件钩子，以窗口、进程身份及代次校验拒绝过期目标，设置重开不重复启动跟踪。平台、DLL 和真实窗口生命周期检查通过；跨进程焦点实测因 Windows 拒绝测试窗口的初始激活而未完成，尚未发送粘贴按键。

- WinUI 新增按历史记录复制原始格式或纯文本的操作；Rust 核心校验敏感内容和载荷，复制回声绑定剪贴板序列，并在采集提交前复查序列。共享 Win32 富文本写入开始传递原先忽略的失败并清理未转移的内存。真实窗口验证失败提示及选择保留；独立窗口站的格式往返测试因 Win32 5 未通过运行门槛，实际成功复制和粘贴验收仍待完成。

- 原生内容层新增先准备后写入的文本、文件、富文本和图片载荷，复用旧版 CF_HTML 与动画格式策略；保留 Office 命名格式，静态回退图片追加 HTML，读取图片有明确范围与大小限制。复制/粘贴命令及实际目标程序验证尚未接入。

- WinUI 主窗口接入原生采集启动/暂停/状态，独立设置窗口支持采集、持久化、隐私、清理与数量策略；C# 提供类型化采集接口。真实窗口检查覆盖设置重开不重启服务、非法草稿保留、修正后保存和运行中退出；实际剪贴板格式往返及粘贴仍待验收。

- 原生核心新增独立采集服务与启动/停止/状态 ABI；共享格式读取和隐私清理流程，支持会话/持久化去重、负数 ID 转换与事务内数量限制，停止和关闭等待线程释放。附件清理延后至提交后执行，同时间戳按 ID 确定淘汰顺序；WinUI 接入仍在进行。

- 原生内容层新增多格式剪贴板读取及序列一致性保护，旧版与原生端共用 Office/WPS、动画图片、格式元数据探测和发现/清理/隐私转换阶段；采集线程接入仍在进行。

- 原生内容处理库同源复用清理规则、敏感匹配、指纹、富文本/格式元数据和受限图片读取；图片目录解析拆为宿主适配，原生端使用可恢复的线程内目录作用域，旧版保留原有目录行为。

- 原生平台库新增有界通知的 Win32 剪贴板监听器，支持启动确认、停止并等待线程释放；复用既有 Win32 格式代码，使用合成窗口通知验证生命周期，尚未接入正文采集。

- WinUI 原生主窗口新增虚拟化历史列表、搜索与筛选、原文详情、新增/编辑/置顶/标签及删除确认；敏感内容主动显示，旧请求和变更流不会回填过期正文，保存失败保留草稿。隔离数据的真实窗口检查覆盖分页和生命周期。

- 原生 C# 调用层新增有类型的异步历史接口，串行执行 DLL 请求并处理排队取消、在途保存结果及退出时的核心释放。

- 原生核心接入兼容历史数据库，新增分页摘要、全文读取、手动新增/编辑/删除/置顶/标签 C ABI、会话条目 ID 转换与有界变更流；敏感标签、加密和 OCR 清理在同一事务内提交。采集/粘贴仍在迁移。

- 原生重构新增完整能力迁移台账及源码清单核对工具，并让原有模型、数据库迁移、DPAPI 和仓库代码在独立无 Tauri 存储库中运行同源回归测试。

- Windows 原生迁移实验新增独立 Rust 设置核心与 C ABI，使用隔离数据库验证配置校验、原子保存和版本同步；现有 Tauri 应用保持原有行为。
- 新增 WinUI 3 原型主窗口、独立设置窗口与 C# 调用层，验证共享设置保存、主题和搜索框同步、窗口重开及原生核心释放；原型排除 WebView2 SDK 资产，纯原生输出可通过窗口功能检查。

## [0.3.13] - 2026-10-03

### Added

- 新增适合快捷剪贴板浮窗的「极简」主题，支持明暗和紧凑列表；外观设置增加强调色、圆角实时预览和本地个人预设，保存或应用预设时原子更新外观设置，不改变现有默认主题与复制粘贴行为。
- 全面打磨「极简」主题与主窗口视觉层级：重构深浅色调板（现代深色 Slate 调板与纯净浅色调板）、升级多阶阴影体系（`--shadow-xs` 至 `--shadow-lg`）、移除复古虚线分隔并替换为微光边框；优化条目选中态为左侧平滑内嵌色条，消除选中与悬停带来的列表跳动。
- 美化主窗口顶栏、搜索栏与分类筛选区：统一定制 26×26px 圆角操作按钮、规范搜索框内边距与一键清除（X）交互；新增「全部」分类胶囊并优化横向滚动条流式滚动，精细化紧凑模式内边距防止时间徽章与两行文本重叠。
- 重构外观自定义与个人预设面板：内置 6 种精选现代强调色圆点与 4 档圆角快捷药丸按钮；新增实时交互预览卡片与预设管理卡片，让主题与自定义效果一目了然。
- 完善浏览器预览运行时的环境守卫（`isTauriRuntime`），保证独立 Vite 预览时数据正常渲染且不阻塞 Vitest 单元测试运行。

### Fixed

- 优化重新打开窗口和返回主界面的过渡：搜索栏恢复时直接采用最终高度，滚动位置在界面绘制前统一复位；页面使用单层短淡入，移除设置页重复滑入，并尊重系统减少动态效果偏好。Windows 托盘左键仅在松开时打开窗口，避免一次点击重复触发复位。
- 修复中文输入法快捷键冲突：外观预设名称输入框（Enter）和主窗口搜索框（Escape）增加输入法组合状态保护（检查 `isComposing`、`nativeEvent.isComposing` 及 `keyCode === 229`），防止候选确认或取消时意外保存预设并清空输入框或清空查询；并在保存期间防止重复提交。
- 修复外观保存期间草稿被覆盖的问题：为外观调色板、快捷圆角、跟随主题重置按钮和预设操作统一切换保存期间（`busy`）禁用逻辑，保持颜色输入框、滑块与按钮行为一致，避免异步保存完成时旧状态覆盖正在调整的草稿。
- 恢复条目操作按钮的主题与自定义圆角：移除条目操作按钮固定的 `4px` 圆角，恢复使用 `--button-radius` token 与主题按钮 token（`--bg-button`、`--button-border`、`--button-shadow`），确保自定义 `0px`、`6px`、`24px` 圆角均能对操作按钮生效；极简专属透明效果收敛至 `minimal.css`，统一普通列表与紧凑模式的表现。
- 修复极简主题下已激活图标按钮（如窗口置顶、条目置顶）在悬停时背景变成浅灰底导致白色图标不可见的问题：在共享 `buttons.css` 与 `minimal.css` 中为 `.btn-icon.active:hover` 明确保留强调色悬停背景（`--accent-hover`）与对比色图标（`--button-active-filled-color`），确保深浅色模式下悬停均具备清晰对比度。
- 主窗口、高级设置与紧凑预览同步自定义主题参数、背景及透明度；修复樱花主题未使用自定义背景、纸质主题暗色选中状态对比不足，以及标题栏按钮和紧凑置顶标识绕过主题变量的问题。样式与设置实现影响所有平台，构建与行为验证以 Windows 为准。
- 修复动态 WebP 被转成静态 PNG 的问题：采集时按真实字节签名保留动画 WebP（含被标成 `.jpg`/`.png` 的缓存文件），粘贴时不再附带 PNG 剪贴板格式，并写入 `image/webp` 与 `.webp` 文件剪贴板，避免目标应用优先取第一帧。静态 WebP 仍走原来的 PNG 兼容路径。
- Windows 默认 URL 打开改用经过协议校验的原生处理程序，避免查询参数被命令解释器执行或截断。
- 备份恢复拒绝 Windows 根路径、盘符路径、备用数据流、设备名及重复路径，避免文件逃出恢复暂存目录。
- 富文本本地图片仅从受控目录读取，并检查实际图片格式、体积与解码限制；云同步仅读取管理附件和收藏表情。自动拷贝的接收图片保存管理副本，保留原下载文件和动画字节。
- 敏感标签统一按大小写不敏感规则判断；紧凑悬停预览遵守遮罩和揭示状态，隐私变化时取消并清除旧预览。
- 全局标签重命名在事务内执行加解密和 OCR 清理；OCR 完成后重查当前条目与敏感状态，加解密队列丢弃已过期任务。
- Windows 粘贴检查输入发送和目标窗口恢复结果，失败时保留历史与顺序粘贴队列，并清理临时输入状态。
- 历史首屏保留全部有界会话记录，数据库分页仅计算持久记录；输入法候选确认不再触发历史粘贴。
- Windows 文件剪贴板纳入采集哈希，避免连续复制不同文件列表被误判为重复；MQTT 长时间重连退避保持上限且不会溢出。
- Windows 卸载保留安装目录中的便携数据和其他用户文件；系统剪贴板设置仅在存在原值备份且当前值仍匹配 TieZ 写入值时恢复。

### Changed

- 忽略 WinUI 实验下载的依赖包、Rust 构建目录和生成代码，以及本地 `.mimosa/` 会话产物，避免误提交大文件阻塞 GitHub 推送。
- 更新应用图标为方案 A「Z-折角」v2（蓝渐变 + 白几何 Z）：Windows 安装器/磁贴、主窗口与文档用图同步替换；托盘改为透明底单色 Z，并按系统任务栏浅/深主题切换墨蓝 / 白色 glyph（含高分屏 `@2x` 墨蓝资源）。未覆盖未随包提供的 `ios/`、`android/` 移动端图标树。
- GitNexus 索引排除本地实验和诊断目录，避免生成或第三方源码阻塞项目调用图更新。
- 本轮备份、图片资源、标签和前端修复采用共用实现，会影响其他平台；构建与行为验证仍以 Windows 为准。

## [0.3.12] - 2026-09-17

### Fixed

- 修复 QQ 通过文件剪贴板提供 `.jpg` 后缀、实际为多帧 GIF 的缓存图片时变成静态 PNG 的问题：在 Windows 文件采集入口优先检查真实文件签名，无论“捕获文件”是否开启，都按 GIF 原始字节保存；非 GIF 和多文件复制保持原有行为。
- 修复 Windows 上 GIF 复制进历史后粘贴仍变成 PNG 的问题：粘贴 GIF 时不再附带 PNG 剪贴板格式（避免目标应用优先取静态 PNG）；从 CF_HTML 恢复的动画 GIF 改为按图片条目保存；关闭“捕获文件”时仍会从 CF_HDROP 保留单个 GIF。
- 修复 GIF 采集判定引入的回归：保留包含正文或多张图片的富文本，避免在 Office/WPS 分支提前读取位图，并复用本次采集已恢复的 GIF，防止重复下载失败后降级为静态图片。

## [0.3.11] - 2026-09-16

### Fixed

- 修复 Windows 从浏览器复制动态图片时 GIF 不显示、伪装为 `.jpg`/`image/jpeg` 的 GIF 被降级为静态图的问题；现在会从 CF_HTML 恢复原始动画，并按实际字节签名持久化附件格式。
- 修复 Windows 上使用自定义/便携数据目录时，已外置保存的图片在重启、切换分类或重新加载历史后因 Tauri 资源协议权限不足而消失的问题；运行时权限仅覆盖 `attachments` 与 `emoji_favorites` 图片目录。
- 修复富文本复制后粘贴会在相邻块元素之间重复插入空行的问题；优先保留来源提供的有效纯文本及其首尾空白，并为 HTML-only 内容按块边界生成纯文本。
- 修复位于用户目录外的自定义背景在重启后消失、必须调整透明度才看似恢复的问题；启动时重新授权已保存图片，保存时限制为受支持的实际图片文件，缺失/不可读时安全回退并反馈，主窗口与高级设置窗口同步更新。
- Startup race where the history list could appear empty after a reboot: the main window can load before Rust setup finishes managing the database state, causing the initial history fetch to fail with no retry. The frontend now retries failed history fetches with backoff, refreshes on the `app-ready` event emitted when backend setup completes, and polls the new `is_app_ready` command as a fallback once retries are exhausted.
- 修复升级后读取错误数据目录、旧历史看似丢失的问题。历史提交 972cb86 将应用标识从 `com.tiez.app` 改为 `com.tiez`，默认数据目录随之从 `%APPDATA%\com.tiez.app` 变为 `%APPDATA%\com.tiez`，旧目录中的历史记录不再被使用。现在启动时会按以下非破坏性规则兼容旧标识目录（`src-tauri/src/migration.rs`、`src-tauri/src/app/setup.rs`）：
  - 显式配置优先：启动先读取当前 `datapath.txt` 重定向（含无效内容）与可执行文件旁的 `data/` 便携目录再执行迁移，优先级保持便携 > 重定向 > 默认；存在任一显式配置时，所有数据迁移整体跳过，**绝不覆盖已有重定向**（重复启动同样安全）。
  - 旧标识目录自身的 `datapath.txt` 按旧版运行时语义解析：重定向有效时实际数据目录是重定向目标（即使 `com.tiez.app` 目录里已没有数据库），无效/为空时回落到旧目录本身；解析出的数据目录没有数据库则视为无数据。
  - 仅旧数据目录有数据库（新目录未初始化）：自动写入 `datapath.txt` 指向旧目录，**原地采用**——不复制、不移动、不删除任何文件，WAL/SHM、附件及数据库内的绝对路径全部保持一致。
  - 两边都有数据库：不按体积或修改时间静默覆盖、合并或删除；弹出“是/否”对话框让用户明确选择（“是”写入重定向采用旧数据，“否”保持当前目录），不选择则双方数据原样保留。
  - v0.2.8 的“贴汁”目录迁移移除了按文件大小替换数据库并递归删除旧目录的逻辑：默认目录不存在时仍整目录重命名；否则同样以原地重定向采用或保留双方数据。旧安装目录清理增加护栏：目录本身或其 `data/` 子目录（便携模式）含 `clipboard.db`、`datapath.txt` 或 `attachments` 时一律跳过；当前数据目录、显式重定向目标、便携目录及其任何祖先目录也不会被当作安装残留删除。**候选与受保护目录在比较前先解析为真实路径**（消除 8.3 短路径、`..`、`\\?\` 扩展前缀、目录联接等 Windows 路径别名），任何一方无法解析、候选为符号链接/联接时即跳过删除（fail closed），不回退到字符串比较；受保护目录**存在性无法判定**（权限不足、元数据错误等，与“明确不存在”严格区分）或运行中程序目录不可得/无法解析时整体放弃目录清理；运行中程序目录的祖先目录同样受保护。
  - **手动恢复指引**：若升级后历史“消失”，旧数据通常仍在 `%APPDATA%\com.tiez.app`。在本机 `%APPDATA%\com.tiez\datapath.txt` 写入旧数据目录绝对路径（单行、无引号）即可切回旧数据；也可在 设置 → 数据目录 中切换（该流程会完整迁移数据库、WAL/SHM、附件并重写库内绝对路径）。升级前请自行备份两个数据目录。

## [0.3.10] - 2026-09-12

### Added

- Collapsible pinned section in the clipboard history: a header with item count toggles the pinned block, the collapsed state persists across restarts (`app.pinned_collapsed`), and keyboard navigation skips hidden pinned items while collapsed.

### Fixed

- Keyboard navigation no longer selects or pastes hidden pinned entries when the pinned section is collapsed and no visible history entries remain.
- Turning off “Show Source App Icon” now hides the source app name in clipboard items and the compact preview header, not just the icon.

## [0.3.9] - 2026-08-18

### Added

- Sound effect preview button in settings so volume can be verified before copying.
- Setting to choose whether number quick paste stays active while the window is edge-docked/hidden.
- Per-item plain text paste action on text and rich-text clipboard entries.

### Changed

- Documented that active development and release validation currently target Windows only.

### Fixed

- Restored audible copy/paste sound effects by fixing the volume scale (`0~1` was incorrectly divided by 100).
- Rich-text paste no longer falls back to the preview image snapshot when HTML content is available.
- Duplicate-content merge no longer clears tags when updating an existing persisted entry.
- Silent startup and `--minimized` autostart now hide the main window instead of leaving it visible.
- Settings hotkey labels no longer force macOS symbols on Windows.
- Global hotkey re-sync no longer fails with `Win+V already registered` when the same shortcut is registered twice during sync.
- Pressing Backspace while recording a shortcut now clears it instead of trying to register `Backspace` as a global hotkey.

## [0.3.8] - 2026-07-27

### Added

- Added explicit desktop clipboard relay shortcuts for sending or fetching one encrypted text item through a separate WebDAV `relay/v1` namespace.
- Added native OS credential-store management for the relay shared key and an authenticated local delivery receipt ledger.

### Security

- Relay requires HTTPS, authenticates message metadata and acknowledgements, and exposes no plaintext content fingerprint or shared key through general settings.

### Fixed

- Restored the Windows-only “Use Win+V Shortcut” setting with persisted state, immediate hotkey switching, and rollback when applying the takeover fails.

## [0.3.7] - 2026-07-26

### Added

- Linux X11 clipboard round-trip integration coverage for text, HTML, images, and file lists
- Native macOS pasteboard revision monitoring and Linux XFixes event monitoring with a resilient polling fallback
- Wayland data-control clipboard support and explicit Linux automatic-paste capability detection
- Cross-platform release preflight builds for NSIS, MSI, DEB, AppImage, macOS app bundles, and DMGs
- Community-maintainer takeover notice in `README.md` and `README.zh-CN.md`
- `CONTRIBUTING.md` for contributor onboarding
- `SECURITY.md` for responsible disclosure guidance
- `.github/pull_request_template.md` for consistent pull requests
- `.github/CODEOWNERS` placeholder for future ownership assignment

### Changed

- Split Windows and non-Windows clipboard capture paths so Linux and macOS no longer depend on Windows API stubs
- Preserve text, rich HTML, image, and file-list clipboard snapshots during transient paste where supported
- Propagate macOS/Linux paste-injection failures before use counts, ordering, or delete-after-paste actions are applied
- Enabled four-platform release documentation and platform-specific local build commands
- Expanded settings, hotkey, appearance, clipboard, database migration, and cloud-sync behavior
- Simplified release workflow to remove a broken portable build/upload path
- Replaced legacy maintainer-facing README support links with fork-maintenance guidance
- Removed outdated issue-template contact links that pointed to legacy upstream infrastructure
- Switched inherited app links and hosted-service defaults to fork-safe configuration and environment variables
- Replaced generic fork placeholders with the maintainer fork URL `sakuradairong/tiez-clipboard`

### Fixed

- Linux automatic paste no longer invokes macOS `osascript`
- Linux and macOS image, file, and rich-text copy operations no longer report success without writing clipboard data
- Linux and macOS direct-text and emoji paste now stages the requested text and safely restores the previous clipboard after a successful paste
- Non-Windows clipboard monitoring no longer exits permanently after a single initialization failure
- Transient paste no longer overwrites a clipboard value changed by the user or target application during restoration
