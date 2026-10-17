# Changelog

All notable changes to this fork will be documented in this file.

The format is inspired by [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Fixed

- 原生完整窗口回归改为等待确认框模板和按钮实际加载，避免提前查找尚未生成的按钮；保留原有超时和保存/删除断言，不改变应用对话框行为。

- 原生设置文本改用同步草稿状态更新，避免快速保存时按钮仍禁用，以及外部配置更新覆盖刚输入的草稿；恢复已保存文本会立即禁用无效保存。

### Added

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
