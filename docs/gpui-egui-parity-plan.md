# GPUI 前端 ↔ 最新 EGUI 的界面还原差距与计划表

> 对照基准：`origin/main` = `a2e87433`（2026-11-09，storytold/figcraft）。
> 本地：`GPUI-smooth` = `1fd20805`（已完成非 UI 同步）。
> 扫描范围：`crates/ui-gpui/src`（81 个非测试文件）对 `crates/ui-egui/src`（**127** 个非测试文件；2026-11-10 复查实测，原文写 130 有误）。
> **2026-11-10 复查修订**：补入 A19–A28（见 F 节复查说明），修正 A2/A18 参照路径、B5/B8 规模、B9 措辞，并新增 G 节「按文件实施清单」。

## 0. 本次同步与扫描结果

| 项 | 结果 |
|---|---|
| 上游领先量 | 16 个提交、81 个文件（merge-base `3635c230`） |
| 已同步 | 44 个文件：engine / format（.fig 缩放整条线）/ mcp / cli / docs / README / ROADMAP / packaging |
| 刻意不同步 | `crates/ui-egui/**`、`crates/ui-gpui/**`（上游根本没有此 crate）、`apps/figcraft/src/main.rs`（唯一改动是调 EGUI 新方法 `start_pending_update`）、根 `Cargo.toml`/`Cargo.lock`（本地独有 `write-fonts`/`usvg` 依赖） |
| 补回的 fork 独有扩展 | `format/export/layer.rs` 的 `NodeKind::Icon` 分支；`format/export.rs` 的 `warn_lossy_grids` |
| 编译验证 | 非 UI 四个 crate `cargo check` 通过；`cargo build -p figcraft-ui-gpui --example run` 通过 |
| `ui-egui` 状态 | **62 个错误，属本地既有破损**（`UiState` 里 `picker_popover`/`style_picker_popover` 各重复声明两次、`toggle_picker` 在两个文件各定义一次）。证据：HEAD 版本 `lib.rs` 里 `pub picker_popover` 已出现 2 次，`git diff HEAD -- crates/ui-egui crates/ui-gpui` 为空——与本次同步无关 |

---

## A. 完全缺失的界面（GPUI 没有任何对应 UI）

| # | 界面 | EGUI 参照 | GPUI 现状（证据） | 影响 | 优先级 | 规模 |
|---|---|---|---|---|---|---|
| A1 | **首页 Home**（Recents / Online / Templates / Getting started） | `home.rs` + `home/` | ✅ `doc.home` → `home` ui_request → `ExtraKind::Home`；`home_body` 读 `prefs.recent_files` / `getting_started_dismissed`，命令全走引擎（`file.openRecent` 等），复用面板 `pill`/`note` | Online 区与缩略图网格仍未做 | P1 | 大 |
| A2 | **画板模式 Boards**（笔/荧光笔/橡皮/便签/画板形状/连接线 + 画板画布） | `canvas/board_bar.rs`、`canvas/board_facilitate.rs`、`canvas/board_connect.rs`、`widgets/board_picker.rs`、`engine/src/boards`（**原文写的 `panels/boards/*` 目录不存在**） | 工具条无 `Marker/Highlighter/Eraser/Sticky/BoardShape/Connector` 任何 id（`toolbar.rs:60-82` 只有设计模式 6 组）；`board.timer` / `board.vote` / `addConnected` / `facilitate` / `BoardTimer` 全部 0 命中 | 整块白板功能不可用（计时器、投票、自动连线仍缺）；**2026-11-11 自检更正**：模板选择器已落地（M 节），此列原写「一起缺」是落后描述 | P1 | 大 |
| A3 | **评论 Comments**（评论工具之外的评论列表/气泡/回复） | `comments_*`、`canvas/comments.rs` | 只有工具按钮 `ToolId::Comment`（`toolbar.rs:80`），无评论面板/线程 UI | 能点工具，无处查看/回复评论 | P1 | 中 |
| A4 | **插件面板 Plugins**（浏览、安装、启用） | `dialogs/plugins_ui.rs`、`plugin_panel` | 只有主菜单 `"Plugins"` 字样（`main_menu.rs:37`），无浏览/管理面板 | 插件生态入口缺失 | P1 | 中 |
| A5 | **图标面板 Icons**（图标库插入） | `icons_panel.rs`（469 行） | ✅ 2026-11-11 落地 `crates/ui-gpui/src/icons_panel.rs`：搜索（名称 / 码位 / `U+` 前缀）、`New icon font`（`icon.font.create`）、字体条目（名称 + 重建 `icon.generateFont` + 删除 `icon.font.delete`）、图标行（按 outline 真实光栅化的预览、名称、`U+XXXX`、删除 `icon.delete`、点击复制字符到剪贴板）；空态两条。**2026-11-11 自检更正（R.4）**：入口 `ui.set {leftTab:"icons"}` → 左面板 tab 2，不再走 `ExtraKind::Icons` 居中浮层；原写「同一套整面板浮层约定」属错配残留，见 R / T 节 | 拖到画布插入文字层、行内改名 / 改码位、拖拽排序、导出字体（`icon.exportFont` + 保存对话框）未移植（egui 有）；重建按钮已按 egui 语义先逐图标 `icon.update` 再 `icon.generateFont` | P2 | 中 |
| A6 | **导出对话框 Export**（格式/倍数/预览/多选导出） | `dialogs/export.rs:64 export_dialog` | ✅ A19 通用壳 + `Field::Preview` 位图预览（`export_preview.rs` 调 `figcraft_render::render_node`） | 多选导出、逐格式参数、原生路径选择仍缺；预览为 0.25 倍位图，不随格式实时切换 | P0 | 中 |
| A7 | **首选项 Preferences 对话框** | `dialogs.rs:211 show`（Preferences 分支） | ✅ 补上 Language（`pref.language`）、Scroll wheel（`pref.scrollWheelZoom`）、`Reset widths`（`view.panelWidths` 空参数 = 引擎自带的重置语义） | 语言框只列 6 个常用项，其余 BCP 47 代码引擎仍支持但 UI 未列 | P0 | 中 |
| A8 | **批量重命名 Batch rename** | `dialogs/batch_rename.rs` | 搜 `"Rename layers"` 0 命中 | 多选改名只能逐个 | P2 | 小 |
| A9 | **分支 UI Branch** | `dialogs/branch_ui.rs` | 搜 `branch` 只命中控制通道/文本编辑，无 UI | 分支管理不可用 | P2 | 中 |
| A10 | **色彩配置 Color profile** | `dialogs/color_profile.rs` | 仅 `panels/right/sections.rs:1823` 注释提到 Page 设置 | 色域 / 配置文件不可设 | P2 | 小 |
| A11 | **Credits / 贡献者** | `dialogs/credits.rs` | 0 处引用 | 关于页缺失 | P3 | 小 |
| A12 | **导入报告 Import report** | `dialogs/import_report.rs` | 0 处引用 | 导入结果/警告不可见 | P2 | 小 |
| A13 | **缺失字体提示 Missing fonts** | `dialogs/missing_fonts.rs` | **2026-11-11 自检更正**：已落地 `dialogs.rs::Kind::MissingFonts`（`dialog.missingFonts` → Spec + 形状测试），原「0 处引用」是**判断失误** | ~~打开缺字体文件无提示/替换入口~~ 已可做替换（仍是居中对话框） | P2 | 小 |
| A14 | **更新对话框 Updates** | `dialogs/updates_ui.rs` + craft-update | 0 处引用 | 无法检查/安装更新 | P1 | 中 |
| A15 | **取色器 Eyedropper / Sampled color** | `dialogs/sampled_color.rs` | 只有光标名命中（`canvas/cursors.rs`），无取样 UI | 屏幕取色不可用 | P2 | 小 |
| A16 | **多窗口**（File ▸ New window / Open in new window / 标签拖出成窗口） | `windows.rs` | `open_window` 是弹层 `popovers::WindowState`，非 OS 多窗口 | 只能单窗口 | P2 | 大 |
| A17 | **菜单项取值对话框**（Create link / Offset path / Move to page 等） | `dialogs/menu_input.rs` | 无对应实现 | 这些命令没有输入入口 | P2 | 小 |
| A18 | **标签关闭未保存询问**（Don't Save / Cancel / Save；Alt+close 直接丢弃） | `windows.rs:392,422,542`（引擎请求 `dialog.unsaved`，**原文写的 `app.rs` #2032 不是对话框所在处**） | ✅ 已落地：`tabs_view.rs:388,392` 的 `force: e.modifiers.alt`、`lib.rs:1400` 消费 `dialog.unsaved`、`dialogs.rs:285-295` 三问 | Save 后不会自动再关闭；拖出成窗口（A16）仍缺（见 I 节） | P1 | 小 |
| **A19** | **通用对话框框架**（数据驱动：标题 / 说明 / 字段 / 按钮 / 确认取消 / Escape / 遮罩） | `dialogs.rs`（579 行，`renderers()` 统一 About·Preferences·Shortcuts·Find·Rename·Variables·Export） | **2026-11-11 自检更正**：已落地 `dialogs.rs`（`Kind::ALL` 16 种 + `Spec` + `parity_contract` 双向比对）；原两处是 2026-11-10 扫描快照，现已过时 | **A6/A7/A8/A10/A11/A12/A13/A15/A17/A18 十个对话框的共同前置**；不做就只能每个各造一套壳 | **P0** | 中 |
| **A20** | **查找替换 Find and replace** | `dialogs.rs:368-404`（`edit.findReplace`） | **2026-11-11 自检更正**：已落地 `Kind::Find`（`edit.findReplace` / `dialog.find` → Spec + Find / Replace all 按钮 + 3 条测试），原「全 0 命中」是**判断失误** | ~~无法批量查找替换~~ 已可用（仍是居中对话框，非 EGUI 内联） | P1 | 小 |
| **A21** | **无障碍 a11y**（AccessKit 树、F6/Shift+F6 区域切换、Tab 焦点环、方向键导航） | `a11y.rs`（1071 行，`focus.region` 请求） | `AccessKit` / `accesskit` / `focus_ring` / `Key::F6` 全 0；`tabs_view.rs:13` 自认「a11y（AccessKit，P4）」 | 键盘/读屏用户不可用 | P2 | 大 |
| **A22** | **双向文字 + 界面缩放**（UAX#9 名称、阿拉伯连字、括号镜像；`pref.uiScale`） | `bidi.rs`（127 行）、`ui_scale.rs`（117 行） | `bidi` / `UAX` / `ui_scale` / `pixels_per_point` 全 0；`tabs_view.rs:134` 自认没有 `bidi::paint_name` | RTL 语言图层名错乱；界面大小不可调（A7 只以「等设置」带过） | P2 | 中 |
| **A23** | **平台服务层**（原生文件对话框 / 剪贴板 / `OPEN_EXTENSIONS` / 拖放进度卡片 / IME） | `services.rs`、`open.rs`、`pending_open.rs`、`app/ime.rs`、`app/clipboard.rs` | `pick_open` / `OPEN_EXTENSIONS` / `pending_open` 全 0；`context_menu.rs:44` 自认「clipboard and file dialogs — which this front-end does not have yet」 | 打开/另存/复制粘贴走不通原生路径 | P2 | 中 |
| **A24** | **字段通用行为**（数字框 `+10/*2/50%` 相对运算、字体预览、实时文本提示、`menus/select_layer.rs`） | `widgets/field_math.rs`、`widgets/font_preview.rs`、`canvas/live_text.rs`、`menus/select_layer.rs` | `field_math` / `font_preview` / `editing_hint` / `select_layer` 全 0 | 数值/字体/文本编辑体验缺失（属通用行为，宜一次补齐给所有字段复用） | P3 | 中 |
| **A25** | **插件 widget 悬浮栏** | `canvas/widget_bar.rs`（113 行，`widget.menu`） | `widget.menu` / `widget_bar` 0 命中；A4 只提了插件面板 | 画布上的插件小工具入口缺失 | P2 | 小 |
| **A26** | **原型画布连线**（noodles、「+」拖拽手柄、overlay ghost） | `canvas/proto_canvas.rs`（341 行）+ `canvas/proto_canvas/overlay_ghost.rs` | `proto_canvas` / `noodle` / `overlay_ghost` 全 0；只有右栏属性 `panels/right/proto.rs` | 原型交互只能在属性面板配，画布上连不了线 | P2 | 中 |
| **A27** | **画板协作小工具**（计时器 / 投票 / 自动连接 / 模板与表格行列选择器） | `canvas/board_facilitate.rs`（340 行）、`canvas/board_connect.rs`（255 行）、`widgets/board_picker.rs`（163 行） | `board.timer` / `board.vote` / `addConnected` / `connected_place` / `BoardTimer` 全 0 | A2 只点了工具条，实际还差整块画板交互 | P2 | 中 |
| **A28** | **Live 协作前端整块**（光标 / 聊天 / spotlight / 共享 / 加入 / 断连标记） | `live.rs`（1571 行）+ `live/agent.rs`（101 行） | `find crates/ui-gpui/src -iname '*live*' -o -iname '*collab*'` = **0 文件**；`tabs_view.rs:13` 自认「live/断连标记（P0-2）」未做 | 原 B5 写成「26 个文件规模、需逐项比对」，实际 GPUI 完全没有；应归到「完全缺失」 | P1 | 大 |

---

## B. 已还原但不到位（有 UI，功能/文案不齐）

| # | 位置 | EGUI 做法 | GPUI 现状（证据） | 差距 | 优先级 |
|---|---|---|---|---|---|
| B1 | ~~右栏第三个 tab~~ | `"Developer Mode"`（#2135 商标改名，`right_tab_label()` + 命名回归测试） | 改名与命名回归测试已落地；**2026-11-10 用户要求从右栏 tabs 行去掉该按钮**，现只剩 Design / Prototype。`naming.rs` 的常量/函数保留给 a11y 与其它入口，Developer Mode 面板仍可从底部工具栏 开发 或 Shift+D 进入 | 命名合规完成；可见 tab 按用户要求移除；底部模式切换第 4 格 `Inspect`→`开发`（修掉退役名），动画/开发模式下右栏 tabs 行改为显示「动画」/「开发」文本（见下方进度） | ✅ |
| B2 | ~~Actions 调色板（Cmd+K）~~ | 三个 tab `All / Assets / Plugins & widgets`、`Recents`、前缀优先排序、最多 60 行（`menus/palette.rs:90,162,170`） | 2026-11-10 落地 `palette.rs`：三 tab + Recents（读引擎 `prefs.recent_actions`）+ 按顶层菜单分组 + 搜索时前缀优先/60 行上限 + Assets/Plugins/widgets 真实数据 | 键盘上下键选中、Enter 运行高亮行、实时过滤（冻结的 `TextField` 只有 commit 回调）未移植；见 H 节 18:4x | ✅ |
| B3 | 工具条 | 设计模式 6 组 + **画板模式 8 组**（bmove/marker/sticky/bshape/connector/text/section…） | 设计模式 6 组齐全；画板组全无（`toolbar.rs:17-20` 明写未移植） | 见 A2 / A27 | P1 |
| B4 | 标签条 | 拖拽重排、关闭询问、拖出成窗口、Alt+close 直接丢弃 | 拖拽重排与关闭询问均已接；**拖出成窗口仍缺**：引擎有 `window.new` 请求，但 GPUI 需重构 `Session` 所有权才能共享会话（见 J 节） | 见 A16 与 J 节 | 🔄 |
| B5 | ~~协作 Live~~ | ~~59 个文件规模~~ → 实为 `live.rs` + `live/agent.rs` 共 1672 行 | ~~26 个文件规模~~ → **0 个文件**，整块缺失 | **已移到 A28**；原文「需逐项比对」严重低估 | P1（见 A28） |
| B6 | 变量面板 | Full / Floating + 表格列、重命名、导出 | `variables_full` 有（`lib.rs:585`），`WindowKind::VariablesTable` 有 | 列行为/多选/导出细节待逐项核 | P2 |
| B7 | 属性面板（右栏 sections） | `panels/right/{effects,inspect,menu,paints,stroke}.rs` 分模块 | 集中在 `panels/right/sections.rs`（+inspect.rs/proto.rs/component.rs） | 需按 section 逐条比对；`fill_rule.rs`（442 行）只有 Non-zero/Even-odd 菜单（GPUI `sections.rs:669-670`），`reverseLoop` 与填充区域蓝/橙可视化、点击反转环仍缺 | P2 |
| B8 | 演示 Present | `present.rs` + **10 个非测试子模块**（actions/chrome/device/draw/input/layout/navigate/player/scroll/transition，约 2731 行） | `present.rs:231` + `preview.rs:257` ≈ 488 行，子模块全无对应 | **约 1/6**；多页/链接/响应式/退出的细节不只是「待核」，是未移植 | P2 |
| B9 | **Hide UI / Minimize UI** | `view.toggleUi`（Cmd+\\）设 `ui_hidden`；`view.minimize`（Shift+Cmd+\\）设 `ui_minimized`，EGUI 据此收侧栏并保留宽度 | engine 两个命令与快捷键都在（`engine/src/cmd/view.rs:333,339`、`keymap.rs:164,228`），✅ 2026-11-10 已接：`lib.rs::ui_flags()`(2231-2233) 读两个状态，`dock()`(2239) 给有效宽度且**不改记住的 `left_w`/`right_w`**，render 条件渲染侧栏/标签条/工具栏/时间轴/分隔条命中区(4219-4222、5847、6674、6682) | 无 | ✅ |
| B10 | 画布光标/工具态 | 每工具+修饰键的光标、拖拽阈值、双击进文本 | `canvas/cursors.rs` 存在 | 需逐工具核对 | P3 |

---

## C. 上游最近 16 个提交里尚未跟进的 UI 相关变更

| # | 上游变更 | 说明 | GPUI 现状 | 优先级 |
|---|---|---|---|---|
| C1 | #2135 / #2165 **Dev Mode → Developer Mode** | 商标原因改名，新增 `right_tab_label()` 与 `naming_tests.rs`；locales 里法/葡/荷的错误译名一并修正。**注意本地 `ui-egui/src/app.rs:352` 仍是 `tr("Dev Mode")`**（该 crate 刻意不同步），`right_tab_label` / `naming_tests.rs` 本地不存在，需以 `docs/feature-names.md` 为准 | ✅ GPUI 侧同前；**全仓 UI 文案已清**：`ui-egui` 的 `app.rs` / `canvas.rs` / `menus.rs` 三处 `tr("Dev Mode…")` 改为 `tr("Developer Mode…")`，并同步 `devmode_tail_tests.rs` 断言。ui-gpui 里的 "Dev Mode" 只剩注释 / RETIRED 测试表 / 搜索别名（标识符，合规） | ✅ |
| C2 | craft-update `c5a1cbf` → `1b34eba`：更新器上报 installing/ready/installed/failed，**退出时安装** | `start_pending_update()` 由 `FigApp` 提供，`main.rs` 退出时调用 | 无更新器 UI、无退出安装 | P1（依赖 A14） |
| C3 | `docs/feature-names.md` 新增 | 命名规范（不再用 Figma-class 比较措辞）；Developer Mode 一节 | 已同步文档，UI 文案尚未按规范对齐 | P2 |
| C4 | MCP display names 测试 | 非 UI（已同步） | 已同步 | — |
| C5 | `.fig` 实例缩放（#2091/#2097/#2100） | engine/format 层 | **已随本次同步落地** | 完成 |

---

## D. 建议实施顺序

> **2026-11-10 19:0x 逐项核查见 I 节，19:1x 补全结果见 J 节**：下面写的是「建议」；第 1 批除 B4 拖出成窗口外均已落地，A1 Home 已接上。

### 第 1 批（合规 + 通用底座，1–2 天）
0. **A19**：通用对话框框架（一次做成数据驱动，十个对话框共用一套壳 + 一套控件 + 一套 Escape/遮罩行为）。**必须排在最前**，否则 A6/A7/A18 各自造轮子。
1. **B1 / C1**：右栏第三 tab 改名 `Developer Mode`，补 `right_tab_label()` 与命名回归测试，清掉全仓 UI 文案里的 `"Dev Mode"` 残留。
2. **B9**：把 `ui_hidden` / `ui_minimized` 接到 render 层（两个快捷键已经绑好，只差界面响应）。
3. **A6**：导出对话框（格式 / 倍数 / 预览），复用 A19 的壳。
4. **A7**：首选项对话框（主题 / 语言 / 滚轮缩放 / 面板宽度重置），复用 A19 的壳。
5. **A18 / B4**：标签关闭询问（Don't Save / Cancel / Save）+ Alt+close，复用 A19 的壳。

### 第 2 批（主导航与内容完整性，3–5 天）
6. **A1**：首页 Home（Recents 卡片 + Templates + Getting started）。
7. **B2**：调色板三 tab + Recents + 前缀排序。
8. **A3**：评论面板（列表 / 气泡 / 回复）。
9. **A4 / A25**：插件面板 + 画布 widget 栏。
10. **A20**：查找替换（A19 的字段模型直接复用）。

### 第 3 批（专业功能，5–8 天）
11. **A2 / B3 / A27**：画板模式整套（工具组 + 画板画布 + 顶栏 + 计时器/投票/连线/模板选择器）。
12. **A5**：图标面板。
13. **A8 / A12 / A13**：批量重命名、导入报告、缺失字体提示（全部走 A19）。
14. **A15 / A10 / A17 / A11**：取色器、色彩配置、菜单取值、Credits（全部走 A19）。
15. **A26**：原型画布连线。

### 第 4 批（长尾，按需）
16. **A9 / A21 / A22 / A23 / A24**：分支 UI、a11y、bidi/ui_scale、平台服务、字段通用行为。
17. **A14 / C2**：更新器 UI 与退出安装（需先升 craft-update，会牵动 ui-egui 编译）。
18. **A16 / A28**：多窗口、Live 协作前端。
19. **B6–B10**：变量 / 属性面板 / Present 子模块 / 光标的逐项精修。

---

## E. 风险与前置条件

| 事项 | 说明 |
|---|---|
| `ui-egui` 本地破损 | 62 个错误（重复字段/重复函数）挡着 `cargo check --workspace`。若要跟 C1/C2 的 EGUI 侧改动，需先修这份重复块（删除后出现的那一份 `picker_popover`/`style_picker_popover` 与 `toggle_picker`） |
| craft-update 升版 | 升到 `1b34eba` 会让 `ui-egui/updates_ui.rs` 用旧 API 编译失败，必须同时接受 EGUI 侧的 API 适配（`Status`、`manual_window_open` 的 cfg 收窄、`result_window_shows`） |
| 根 `Cargo.toml` / `Cargo.lock` | 本地独有依赖（`write-fonts`/`usvg`）与上游不同，**不能整体覆盖**，每次同步都要手工合 |
| 命名合规 | `"Dev Mode"` 属 Figma 商标，UI 文案、i18n 目录、帮助文本都要改；命令 id `view.devMode` 保持不动 |
| **通用控件冻结** | `crates/ui-gpui/src/widgets/**` 均已 FROZEN，任何行为调整都要先与用户确认。新界面**只能调用** `PanelSection`/`field_row`/`Popover`/`TextField`/`number_field`/`Dropdown`/`IconButton`/`MenuAim` 等既有通用件，不得手搓行布局、不得改通用件 |
| 同一文件串行改 | 一个文件的多处改动必须串行编辑并改完复查盘上内容，避免并行写互相覆盖 |

---

## F. 2026-11-10 复查说明（遗漏是怎么查出来的）

方法：取两个 crate 的全部非测试 `.rs` 清单（EGUI 127 / GPUI 81），按目录逐组比对，并用关键词 grep 反查 GPUI 是否已有等价实现（不只看文件名），再核对文档里每条「现状证据」的实际行号。

**已有、不算遗漏**（复查反证「漏掉大块面板」不成立，GPUI 把它们合并在几个大文件里）：
`panels/{shortcuts_panel,versions,dev_compare,assets_panel}.rs` → `panels/extra.rs` 的 `ExtraKind::{Shortcuts,Versions,DevCompare,Assets}`；`typography.rs` / `scale_panel.rs` / `selection_colors.rs` / `export_panel.rs` / `section_panel.rs` → `panels.rs`；`vector_panel.rs` → `sections.rs:1292` + `data.rs:963`；`widgets/{colorpicker,style_picker}.rs` → `popovers.rs`；`shortcuts.rs` / `app/ime.rs` → `lib.rs`；`menus/tree.rs` → `main_menu.rs` + `widgets/menu_aim.rs`。

**新补入的遗漏**：A19–A28（见 A 表），其中最重要的是 **A19 通用对话框框架**——原文把十个对话框各列为独立条目，却没写它们共同的根因；以及 **A28 Live**——原文 B5 误判为「已还原但不到位」，实际 GPUI 0 个文件。

**证据修正**：
1. 文件数 130 → 127（非测试文件实测）。
2. A2 参照 `panels/boards/*` 目录不存在，真实文件在 `canvas/board_*.rs` + `widgets/board_picker.rs`。
3. A18 参照应为 `windows.rs:392,422,542`，不是 `app.rs` #2032。
4. B5「26 个文件」→ 0 个；B8「`present.rs` 存在，细节待核」→ 约 1/6 规模。
5. B9「全仓不读」→「`crates/ui-gpui/src` 内 0 处读」（engine / ui-egui 侧确有命中）。
6. C1 本地 `ui-egui` 仍是旧文案 `tr("Dev Mode")`，`right_tab_label()` / `naming_tests.rs` 本地不存在，实施时以 `docs/feature-names.md` 为准，不要照抄本地 EGUI。
7. 现状证据复查：B3 仍成立（`toolbar.rs:17-20` 画板组未移植）；B1 与 A18 已在 2026-11-10 实施完成——B1 的右栏第 3 tab 已移除、底部切换器第 4 格由退役名 `Inspect` 改为 `开发`（原 `:243` 的 `"Inspect"` 已不存在），A18 的标签关闭三问已接通用对话框，原 `tabs_view.rs:11-13`「无对话框/a11y/live」已不成立。

---

## G. 按文件实施清单

> 原则：**先通用、后具体**；每个界面只写「数据 + 复用通用件」，不新增专属控件/行为。
> 状态：⬜ 待办 / 🔄 进行中 / ✅ 已落地（含编译或测试验证）。

| 顺序 | GPUI 目标文件 | 对应条目 | 复用的通用控件 / 行为 | 状态 |
|---|---|---|---|---|
| 0 | `docs/gpui-egui-parity-plan.md` | 复查修订 | — | ✅ |
| 1 | `crates/ui-gpui/src/naming.rs`（新） | B1 / C1 | 纯函数 + 命名回归测试（无 UI 依赖） | ✅ |
| 2 | `crates/ui-gpui/src/panels/right/sections.rs` | B1 / C1 | `tab(...)` 既有 tab 控件，只换 label 来源 | ✅ |
| 3 | `crates/ui-gpui/src/dialogs.rs`（新） | A19（A6/A7/A8/A10/A11/A12/A13/A15/A17/A18/A20 共用的壳） | `Popover` 外壳、`field_row`/`field_row_full` 统一网格、`text_field`/`number_field`、`CmdQueue`、通用 Escape/遮罩行为 | ✅ |
| 4 | `crates/ui-gpui/src/lib.rs` | A19 挂接 / B9 | `sync_ui_requests` + 渲染分层 + `DockLayout`；`ui_hidden`/`ui_minimized` 只改布局值，不动几何公式 | ✅ |
| 5 | `crates/ui-gpui/src/main_menu.rs` | A6 / A7 / A11 菜单入口 | **无需改此文件**：菜单命令统一在 `lib.rs::click_menu` 拦截，仍是既有主菜单行模型 | ✅ |
| 6 | `crates/ui-gpui/src/tabs_view.rs` | A18 / B4 | 关闭去掉写死的 `force:true`，只有 Alt+close 才直接丢弃；引擎 `dialog.unsaved` → 通用三问（Don't Save / Cancel / Save），弹窗前 `doc.switch` 到目标标签 | ✅ |
| 7 | `crates/ui-gpui/src/palette.rs`（新）+ `menu_card.rs` + `lib.rs` | B2 | 既有 `menu_card::rows` / `Palette` 弹层；`Item.key` 让运行时 action key 复用同一套 `OnPick` | ✅ |
| 8 | `crates/ui-gpui/src/panels/extra.rs`（`ExtraKind::Home` + `home_body`） | A1 | 既有整面板 chrome + `pill`/`note`；数据与命令全来自引擎 | ✅ |
| 9 | `crates/ui-gpui/src/comments.rs`（新） | A3 | 通用浮层 + 输入框系列 | ✅（L 节；回复 / 编辑 / 表情未接） |
| 10 | `crates/ui-gpui/src/plugins.rs`（新） | A4 / A25 | 通用浮层 + `widget_bar` 复用同一行模型 | 🔄（A25 ✅；A4 只剩网络 Browse） |
| 11 | `crates/ui-gpui/src/toolbar.rs` + `board_bar.rs` / `board_picker.rs` | A2 / B3 / A27 | `GROUPS` 既有工具组模型（只加数据，不改控件） | 🔄（工具组 + 模板选择器已落地见 M 节；计时器 / 投票 / 连线未核） |
| 12 | `crates/ui-gpui/src/present/*`（新） | B8 | `present.rs` 既有播放状态机 | ⬜ |
| 13 | `crates/ui-gpui/src/a11y.rs`（新）/ `bidi.rs` / `ui_scale.rs` | A21 / A22 | 通用焦点环 / 名称渲染钩子 | ⬜ |
| 14 | `crates/ui-gpui/src/services.rs`（新） | A23 | 平台能力接口（保持 UI 可替换） | ⬜ |
| 15 | `crates/ui-gpui/src/widgets/field_math.rs`（新，需解冻确认） | A24 | 通用数字字段（**冻结件，改前必须先问用户**） | ⬜ |
| 16 | `crates/ui-gpui/src/panels.rs` | 右栏 48 pt 顶栏（头像 / Present / Share）恢复 | 既有 `right::topbar::top_bar` 通用顶栏，只补一行调用，与 Motion 分支同序 | ✅ |
| 17 | `crates/ui-gpui/src/panels/right/sections.rs` | 右栏 tabs 去掉第三个「开发模式」按钮 | 既有 `tab(...)`，只减少一项数据；面板功能仍由底部工具栏 开发 / Shift+D 进入 | ✅ |
| 18 | `crates/ui-gpui/src/icons_panel.rs`（新）+ `panels/extra.rs`（`ExtraKind::Icons`）+ `lib.rs`（`ui.set {leftTab}`） | A5 | `search_field` / `IconButton` / `CmdQueue` / `note` / `pill`；预览走 `figcraft_render::render_path` + `screen_buffer` 同路 | ✅ |
| 19 | `crates/ui-gpui/src/panels.rs` + `lib.rs`（rail 调用点） | 左栏 rail 三项高亮互斥 | 抽纯函数 `rail_active(...)` + 4 条穷举回归测试；`left_rail` 签名收敛为 `(left_tab, variables_open, variables_full)`，未新增控件 | ✅（S 节） |
| 20 | `crates/ui-gpui/src/layer_tree.rs` | 图层树节点多时选中高亮错位 + 24pt 规格 | 逐行 `relative()+absolute()` 绘制；**2026-11-11 18:5x 追加**：色块/图标高 24、图标格 24×24、块间距 8、每层缩进 24、每行 `flex_none()` 防压缩；`selection_runs` 已删（不再拼连块） | ✅（S / T 节；虚拟化未做） |
| 21 | `crates/ui-gpui/src/theme.rs` | 父集/子集选中色不是深浅差异 | `selected_row_child` 改由同一品牌色派生（白 0.93 / 黑 0.90），不再写死浅绿 | ✅（T 节） |
| 22 | `crates/ui-gpui/src/dialogs.rs` + `lib.rs` + `parity_contract.rs` | A8 批量重命名 1:1 | 照 `ui-egui/dialogs/batch_rename.rs` 逐项抄；预览走引擎 `BatchRename::new/apply`；`z_sorted` 注入 ids/names；契约 `A8` 双向核对 | ✅（Y 节） |

---

## H. 2026-11-10 第 1 批实施结果（按文件）

| 文件 | 落地内容 |
|---|---|
| `docs/gpui-egui-parity-plan.md` | 复查修订：补 A19–A28、修正 7 处证据、新增 F 复查说明与 G 按文件清单 |
| `crates/ui-gpui/src/naming.rs`（新） | `right_tab_label()` + `RIGHT_TAB_LABELS` + 4 条命名回归测试；命令 id 与调色板搜索别名保持不变 |
| `crates/ui-gpui/src/panels/right/sections.rs` | 第三个 tab 从 `"Inspect"` 改为 `crate::naming::right_tab_label(2)`（常量后随用户口径改为 `"开发"`）；2026-11-10 用户要求从右栏 tabs 行移除该按钮，现只剩 Design / Prototype |
| `crates/ui-gpui/src/dialogs.rs`（新） | A19 通用框架：`Kind` / `Field` / `Action` / `Spec` 数据模型 + 15 种 `spec()` + `ui_action()` / `menu_action()` 映射 + `DialogState`（retained 输入框）；外壳用 `widgets::popover::Popover`，遮罩用 `popup_backdrop`，行一律走 `field_row`/`field_row_full`，按钮统一推 `CmdQueue`。**没有修改任何 `widgets/**` 冻结件** |
| `crates/ui-gpui/src/lib.rs` | `mod naming` / `mod dialogs`；`CanvasView::dialog` 状态；`sync_ui_requests` 消费 `dialog.*`；`click_menu` 拦截带 "…" 的命令；Escape / X / 遮罩统一关闭；render 末尾挂对话框层。B9：`ui_flags()` 读 `ds().view.ui_hidden/ui_minimized`，`dock()` 统一给出有效宽度（**不改记住的 `left_w`/`right_w`**），`local()` 与 render 共用同一份 Dock 值，隐藏/收起时条件渲染侧栏、标签条、工具栏、时间轴与分隔条命中区 |
| `crates/ui-gpui/Cargo.toml` | 新增**可选** `test-support` feature（`gpui/test-support` + 可选 `usvg`），只供手工跑单元测试；`[dev-dependencies]` 保持只有 `tiny-skia`。**同时修正 17:15 引入的性能回归**：先前把 `gpui/test-support` 放进 dev-dependencies，而 examples 也使用 dev 依赖，Cargo 把 `test-support → leak-detection → backtrace` 统一进了 exe 的 GPUI 构建，拖动时整窗非常卡 |

**验证**：
- `cargo check -p figcraft-ui-gpui --lib`：通过。
- `rustfmt --check`（`dialogs.rs` / `naming.rs`）：通过。
- `cargo test -p figcraft-ui-gpui --lib --features test-support`：**202 通过 / 3 失败**；新增 9 条（`naming` 4 + `dialogs` 5）全部通过。不带该 feature 时，既有 `#[gpui::test]` / `TestAppContext` / 图标 SVG 测试仍会让 lib-test 目标编译失败（34 个既有错误，属仓库原有状态）。
- 3 条失败用 **HEAD 基线单独复验**（备份 → `git show HEAD` 还原 `lib.rs`/`sections.rs` → 跑全套 → `cp` 恢复 + `md5sum` 核对）：基线 **193 通过 / 同样这 3 条失败**，确认为既有问题，与本次改动无关：`main_menu::aim_integration_tests::slanting_toward_the_submenu_keeps_file_open`、`panels::extra::tests::shortcuts_come_from_the_engine_keymap`（Shift+D 不在 KEYS 表）、`panel_resize_tests::divider_hit_area_is_four_px_centred_on_the_hairline`。

**性能回归复验（17:23）**：修复后 `cargo build -v` 显示 exe 链接的是 default-only 的 `libgpui-99dcec04e3ffb5d9.rlib`；`cargo tree -e features -i gpui` 再也搜不到 `test-support` / `leak-detection` / `backtrace`；exe 体积 78,098,944 → 76,569,600 字节（少了 backtrace/泄漏检测代码）。已重新编译并用资源管理器托管启动，窗口标题 `FigCraft`、Responding=True（PID 22288）。**注意**：默认 `cargo build --example run` 与 `cargo check --lib` 都不带该 feature，界面性能回到上一个版本的配置。

**诚实的边界**（没有假装接好）：
- A6 导出、A7 首选项、A20 查找替换、A10 色彩配置、A8/A17 重命名、A12 导入报告、A23 另存、A9 分支已通过通用壳接到**真实引擎命令**（`file.export` / `view.panelWidths` / `edit.findReplace` / `file.colorProfile` / `object.rename` / `file.saveAs` / `branch.create`）；但字段仍是通用版，不等于 EGUI 的完整交互（预览、多选导出、逐字体替换等）。
- A18 首轮只接了引擎 `dialog.unsaved` 请求，`tabs_view.rs` 的 Alt+close / 关闭三问当时未接；已在 17:2x「G 表第 6 行落地」补齐（见下），此处保留原记录仅为时间线。
- A13 缺失字体、A15 取样色、A14 更新、A11 About、A4 插件目前是**带说明的提示窗口**，因为引擎没有对应命令或依赖 C2 升版；按钮只有 Close，不提交不存在的命令。
- A1 Home / A2 Boards / A3 Comments / A16 多窗口 / A21 a11y / A22 bidi·ui_scale / A24 字段运算 / A28 Live / B8 Present 子模块仍未开始（B2 调色板已于 18:4x 落地，见下）。

### 17:25–17:33 按现场反馈追加调整

| 事项 | 处理 |
|---|---|
| 提交 GitHub | `e9de7cb4`（第一批）+ `e9640ac1`（Cargo.lock）已推送 `fork/GPUI-smooth`。git 的 `http.proxy` 写死在失效端口 65311，实际监听的是 54834；临时 `-c http.proxy=… -c http.schannelCheckRevoke=false` 后推送成功 |
| 用户头像 / Present / Share 栏「会丢失」 | 根因：`panels.rs::right_panel` 的 `wrap()` 只画 `view_bar` + body，48 pt 顶栏**只在 `lib.rs` 的 Motion 分支**画过。已在 `wrap()` 补上 `right::topbar::top_bar(f, data)`，顺序与 Motion 分支一致；两条分支互斥，不会重复画 |
| 「下面这一栏不要开发模式按钮」 / 底部切换器第 4 格名称 | 用户确认为**右栏 tabs 行的第三个标签**，已移除只留 Design / Prototype；`naming.rs` 的 `right_tab_label` / 常量 / 命名回归测试保留给 a11y 与其它入口，Developer Mode 面板本身没删（底部工具栏 开发、Shift+D 仍可进入）。同一轮把底部模式切换器第 4 格从退役名 `Inspect` 改为 `开发`，并在动画/开发模式下把右栏 tabs 行的 Design/Prototype 换成「动画」/「开发」文本 |
| 验证 | `cargo check -p figcraft-ui-gpui --lib` 通过；重编前先关闭占用 `run.exe` 的旧窗口（否则 LNK1104），新 exe 17:32 生成，17:33 启动窗口 `FigCraft`（PID 4204，Responding=True） |

### G 表第 6 行落地（A18 标签关闭三问）

- **根因**：`tabs_view.rs` 的 X / 中键关闭一直写死 `file.close {force:true}`，把引擎的脏文档检查整个绕过去，所以三问永远弹不出来。
- **修法**：改为 `force: e.modifiers.alt`（Alt+close 才直接丢弃）；普通关闭由引擎对脏文档发 `dialog.unsaved`，已接好的通用对话框弹 Don't Save / Cancel / Save。`sync_ui_requests` 在弹窗前执行 `doc.switch {index}`，避免 Save 存到别的活动文档；Save 按钮改为不带 index 的 `file.save`（引擎不接受 index）。
- **诚实的边界**：Save 只保存，不会自动再执行一次关闭（这样即使保存失败也不会丢改动）；保存后再点 X 即可关闭。EGUI 的「保存后自动关闭 / Save as」流程仍未移植。
- **验证**：`cargo check -p figcraft-ui-gpui --lib` 通过；`cargo test --lib --features test-support -- naming dialogs` 9 条全绿；重编 exe（17:36）并重启窗口（PID 3500，Responding=True）。

### 18:1x 模式切换器改名 + 右栏 tabs 文本化（用户 2026-11-10 现场反馈）

- **底部模式切换器第 4 格名称不对**：原写死 `"Inspect"`，而 `naming.rs` 的 `RETIRED` 列表把 `Inspect` 列为退役名（违反 #2135 商标合规）。已改为 `"开发"`，与用户口径一致；`active` 高亮逻辑不变（仍由 `right_tab==2` 判定）。`naming.rs` 的 `DEVELOPER_MODE` 常量同步改为 `"开发"`，命名回归测试相应更新（仍 4 条全绿）。
- **动画 / 开发模式里不显示 Design / Prototype 按钮**：`view_bar` 新增 `motion` 形参；`motion==true` 时整行显示「动画」、`f.right_tab()==2`（开发模式）时显示「开发」，其余模式才显示可点的 Design / Prototype 两个标签。动画分支由 `lib.rs` 的 Motion 右栏头部传 `true`，普通分支传 `self.motion`；`right_panel` 同步新增 `motion` 形参。
- **点 Design 回不去 / 点不了 Draw（开发模式下）**：根因是 `on_intent` 的 `Mode(0)`/`Mode(1)` 分支只关 Motion、切工具，却**没把右栏标签切回 Design**（`set_right_tab(0)` 缺失），于是 `right_tab` 停在 2，右栏一直停在 Inspect 面板、底部高亮也永远卡在开发格。已对齐 EGUI `pick_mode`：Draw/Design 两个分支都补 `self.fields.set_right_tab(0)`。
- **验证**：`cargo check -p figcraft-ui-gpui --lib` 通过；`cargo test --lib --features test-support -- naming` 4 条全绿；重编 exe 后启动窗口（Responding=True）。

### 18:4x G 表第 7 行落地（B2 Actions 调色板三 tab）

- **新模块 `crates/ui-gpui/src/palette.rs`**（数据层，不碰任何 `widgets/**` 冻结件）：`Tab::{All,Assets,Plugins}`、`Section` / `Row`、`sections()` / `items()`，逐项对齐 `ui-egui/src/menus/palette.rs`。
  - `All` = Recents（读引擎 `Session.prefs.recent_actions`）+ 按顶层菜单分组的命令；`Assets` = `library::local_components` / `local_styles`；`Plugins & widgets` = `prefs.plugins` 的命令与 widget；Plugins 菜单项并入 Plugins 分组。
  - 搜索时把 `all_commands()`（排除 `cmd::needs_params`）并入候选、去重同名标签、**以 query 开头的标签优先 + 短标签优先**、截断 60 行；空查询保持目录顺序且不截断。`catalog::SEARCH_ALIASES`（"Dev Mode"）参与匹配，保住 `naming.rs` 的命名回归口径。
  - 分组标题用 `menu_card::Item` 的 disabled 行呈现，不新增控件。
- **`menu_card.rs` 最小扩展**：`Item` 增 `key: SharedString`（默认 = `id`）与 `key()` 构造器，`OnPick` 由 `&'static str` 改 `&str` —— 让 `component:<node>` / `style:<id>` / `plugin:<id>/<cmd>` / `widget:<id>` 这些运行时 key 复用同一套菜单行与点击回调，不必 `Box::leak`。原右键菜单 / 工具栏飞出的闭包只改参数类型。
- **`lib.rs`**：`mod palette` 与 `palette_tab` 视图状态；`palette_at_pointer` 统一收口所有打开路径并重置为 All；新增 `pick_palette()` 按 key 前缀转真实引擎命令（`plugin.run` / `widget.insert` / `instance.place` / `style.apply`），并写回 Recents（newest first，上限 `MAX_RECENT_ACTIONS` = 5）；render 在借用 `&mut self.popup` **之前**快照 sections，弹层新增三 tab 行与空搜索提示。
- **诚实的边界**：① 搜索框仍只在 **Enter 提交**时更新（冻结的 `widgets::TextField` 只有 `set_on_commit`，要实时过滤必须改冻结件，动手前先问用户）；② 打开面板保留上次搜索词（否则清空模型会留下旧文本），EGUI 是清空；③ 上下键移动高亮 / Enter 运行高亮行未移植；④ 引擎没注册的命令仍是 disabled 显示（EGUI 直接隐藏）；⑤ 插件命令走 `plugin.run`、widget 走 `widget.insert`，没有 EGUI 的插件面板 UI（A4 仍缺）。
- **验证**：`cargo check -p figcraft-ui-gpui --lib`、`cargo check --example run` 通过；`rustfmt --check`（新模块）通过；`cargo test --lib --features test-support -- palette` **8/8**；全套 **210 通过 / 3 失败**，3 条与 HEAD 基线同名同因（`main_menu aim` / `extra Shift+D` / `panel_resize hit area`），无回归。
- **exe（18:56 用户确认后重编）**：先关掉占用 `run.exe` 的旧窗口 PID 27484，`cargo build -p figcraft-ui-gpui --example run` 通过（76,654,080 字节 / 18:56:55），经资源管理器托管启动新窗口 PID 15124（`FigCraft` / Responding=True）。构建有一条 `error deleting lock file for incremental compilation session directory … 拒绝访问` 警告，属旧进程残留的 incremental 锁文件，不影响产物（Finished dev profile）。**改动仍未提交**。

---

## I. 2026-11-10 19:0x 完成情况核查（第 1 批 6 项 + 第 2 批 A1）

> 用户要求逐项核对 D 节「建议实施顺序」第 1 批六项与第 2 批第 1 项（A1）的**实际落地**，而不是照抄计划。
> 方式：直接读当前 `crates/ui-gpui/src` 源码、`engine/catalog` 与 `ui-egui` 对照，每条给出文件:行号证据，明确「完成 / 部分完成 / 未开始」。

| 条目 | 计划要求 | 实际状态 | 证据（文件:行） | 未完成 / 诚实边界 |
|---|---|---|---|---|
| **A19** 通用对话框框架 | 十个对话框共用一套壳 + 一套控件 + 一套 Escape/遮罩，数据驱动 | ✅ **完成** | `dialogs.rs`（15 种 `Kind`；A6/A7/A20/Plugins/Branch/SaveAs 已逐个深化）、`Field::{Text,Number,Choice,Note}`、`Spec`/`Button`/`DialogState`、`ui_action()`/`menu_action()`、`render()`(452)、`popup_backdrop`(543)；壳用 `widgets::popover::Popover`、行用 `field_row`/`field_row_full`、按钮统一推 `CmdQueue`。`lib.rs`：状态(608)、`sync_ui_requests` 消费全部 `dialog.*`(1400)、带「…」菜单拦截(3317)、Escape(5289)/X/遮罩统一、render 挂层(6632)。**5 条单测**，**未改任何 `widgets/**` 冻结件** | A13/A15/A14/A11/A4 因引擎无对应命令，仍是「带说明的提示窗口」（按钮只有 Close），不等于 EGUI 的完整交互 |
| **B1 / C1** 命名合规 | 右栏第三 tab 改名、补 `right_tab_label()` 与命名回归测试、清掉全仓 UI 文案 `"Dev Mode"` 残留 | ✅ **完成** | `naming.rs`（`"开发"` + 4 条回归）；`ui-egui` 的 `app.rs:352` / `canvas.rs:238` / `menus.rs:56` 三处真实 UI 文案已改为 `Developer Mode…`，`devmode_tail_tests.rs:322` 断言同步 | 无（注释、RETIRED 测试表、`SEARCH_ALIASES` 里的旧名是标识符/测试，按规范保留） |
| **B9** Hide UI / Minimize UI | 把 `ui_hidden` / `ui_minimized` 接到 render 层 | ✅ **完成** | `lib.rs::ui_flags()`(2231-2233) 读 `ds().view.ui_hidden/ui_minimized`；`dock()`(2239) 给有效宽度且**不改记住的 `left_w`/`right_w`**；render 条件渲染侧栏 / 标签条 / 工具栏 / 时间轴 / 分隔条命中区(4219-4222、5847、6674、6682) | 无 |
| **A6** 导出对话框 | 格式 / 倍数 / **预览**，复用 A19 壳 | ✅ **完成** | `dialogs.rs::Kind::Export` 的 Format / Scale / File name + `Field::Preview`；`export_preview.rs` 用 `figcraft_render::render_node` + `to_rgba8` + 项目既有 `wrap_rgba`，显示走 `img(ImageSource::Render)` 并放进 `field_row_full` | 多选导出、逐格式参数、原生路径选择仍缺；预览固定 160×100 显示框、0.25 倍栅格，不随格式实时切换 |
| **A7** 首选项对话框 | 主题 / **语言** / **滚轮缩放** / **面板宽度重置**，复用 A19 壳 | ✅ **完成** | `dialogs.rs::Kind::Preferences` 新增 Language（`pref.language` 带每选项参数）、Scroll wheel（`pref.scrollWheelZoom`）、`Reset widths` 按钮（`Action::Run` → `view.panelWidths` 空参数，引擎自带的重置语义）；全部复用 `Field::Choice` chips + `field_row` | 语言只列 6 个常用项（引擎支持更多 BCP 47 代码）；首选项无实时预览 |
| **A18 / B4** 标签关闭三问 | Don't Save / Cancel / Save + Alt+close + 拖拽重排 + 拖出成窗口 | ✅ **A18 完成** / 🔄 **B4 部分** | A18 同前（`force: alt`、`dialog.unsaved`、`doc.switch`、三问）；拖拽重排 `tabs_view.rs:155-187` 已有 | **拖出成窗口仍缺**：引擎确有 `window.new` 请求（`cmd/view.rs:365`），但 GPUI 要开 OS 多窗口须重构 `Session` 所有权以共享会话，属于「自己弄」，按本轮约束不做（见 J 节） |
| **A1** 首页 Home（第 2 批第 1 项） | Recents 卡片 + Templates + Getting started | ✅ **完成** | 独立 `home/mod.rs`（不再塞在 extra.rs）：`doc.home` → `ExtraKind::Home` → `home::body`；Recents 网格/列表 + 名称/编辑时间排序读 `prefs.recent_files` / `getting_started_dismissed`，按钮推 `file.openRecent` / `file.new` / `file.newBoard` / `file.newFromTemplate` / `help.gettingStarted`，行用面板既有 `pill` / `note` | Online（服务器文件）区、缩略图卡片网格、`home_sort`/`home_order` 排序 UI 未做；Recents 为文本行 |

<!-- PARITY_CONTRACT_STATUS — 由 crates/ui-gpui/src/parity_contract.rs 自动核对；
     状态只能随代码一起改，测试会双向检查「代码做到没 / 文档标没标」。
A19=done
B1C1=done
B9=done
B2=done
A6=done
A7=done
A18B4=partial
A1=done
A2B3A27=done
A8=done
-->

**结论（19:1x 补全后）**：核查出的缺口已补 —— A6 预览、A7 三项、C1 全仓文案、A1 Home 全部落地；第 1 批 6 项现为 **5 项完成 + 1 项部分（B4 拖出成窗口）**。B4 是**有意不做**：引擎只有 `window.new` 请求，GPUI 侧缺 OS 多窗口的会话共享，按「不自己弄」的约束留待 A16。所有状态已回填 A/B/C/G 表，并由 J 节的契约测试自动守护。

**（下一步已移至 K 节末尾）**

---

## J. 防回归机制（2026-11-10 19:1x 起生效）

> 用户要求：「写机制，确保这个问题以后不会再犯」。问题 = 计划表里的状态是**人手写**的，
> 代码落后了表格也不会失效（A6/A7 被当成完成、A1 一行没写却排在「下一步」）。

**机制**：`crates/ui-gpui/src/parity_contract.rs`（`#[cfg(test)] mod`，随 `cargo test` 跑）。

1. **唯一事实源** `CONTRACTS`：每个条目声明「完成」所依赖的**能力 token**（字段 key、引擎命令 id、通用件名、ui_request 分派结果、按钮标签）。
2. **能力只从代码派生** `capabilities()`：遍历 `dialogs::Kind::ALL` 的真实 `spec()` 取所有字段 / `Field::Preview` / 选项命令 / 按钮；`include_str!` 扫描 `dialogs.rs` / `lib.rs` / `palette.rs` / `naming.rs` 与 ui-egui 的 UI 文案；再查 `panels::extra::ui_action("home" | "window.new", …)` 的分派结果。**没有任何手写的「已完成」清单**。
3. **文档状态机器可读**：I 节末尾的 `<!-- PARITY_CONTRACT_STATUS … -->` 块。测试**双向**比对：代码没做到却标 `done`、代码做到了却还标 `partial`，都会失败并指名缺哪个 token。
4. **不许自己造命令**：`every_dialog_command_is_a_real_engine_command` 断言每个对话框按钮 / 选项推的 id 都能被 `figcraft_engine::find_command` 找到 —— 落实「尽可能使用引擎底下已有的能力，而不是自己弄」。
5. **通用控件**：新字段一律走既有 `field_row` / `field_row_full` / `choice_chips` / `pill` / `note`，预览只是把项目已有的 `img(ImageSource::Render(..))` 放进一个 `field_row_full`；**没有修改任何 `widgets/**` 冻结件**，也没有新增专属控件。

**负向验证**（2026-11-10）：把 `A6=done` 临时改成 `partial` 后跑测试，得到
`A6: code satisfied=true (missing []), document says 'partial'. Update the code AND the status block together`，随后恢复文档。守卫确实会拦住过时状态，不是空跑。

**维护规则**：新增/删减能力 → 先改 `CONTRACTS`，再改代码，最后同步状态块与 I 节表格；三者不一致时
`cargo test -p figcraft-ui-gpui --lib --features test-support -- parity_contract` 会失败。**不要只改文档来让测试变绿。**

**仍诚实保留的缺口**：`A18B4=partial` 依赖 `detach_window`。引擎确实有 `window.new` 请求（`cmd/view.rs:365`），但 GPUI 侧接 OS 多窗口要重构 `Session` 所有权以共享会话，属于「自己弄」，本轮按用户约束不做；契约保留这个 token，真正接上才会自动翻成 `done`。

---

## K. 全项目 GPUI 自查（2026-11-10 19:4x）

> 触发：用户指出「里面的这些控件实际上下拉菜单我都是有的，但是我不知道你为什么不去还原。
> 还有窗口本体也是有可以拖动的通用控件的，你也不用」，并明令
> **「该还原的部分就该 1:1 还原，照着原来代码的布局直接抄」「写进项目记忆，严禁这么干」**。
> 方法：以 `crates/ui-egui/src` 为唯一参照逐文件比对，并盘点 `widgets/` 下已有却没被调用的
> 通用件。规则已固化到 `.workbuddy-ai/memory/MEMORY.md`（铁律，跨会话生效）与 J 节契约测试。

**符号**：✅ 本轮已修 / 🔄 本轮修了主结构但仍有细节 / ⬜ 待办。

| # | GPUI 位置 | EGUI 参照 | 差距（原控件 → 现/原替代件） | 状态 |
|---|---|---|---|---|
| 1 | `panels/extra.rs` `home_body` | `home.rs:145-232,313-370,418-454`；`home/home_recents.rs` | 全屏 240pt sidebar + Recents/Online/Templates tab + `action_card` 260×72 + 6 tips 徽标卡 + 搜索/排序 → 原先只有 `note()` 段 + `pill()` 列表 | 🔄 已改为全屏 + 侧栏 + 卡片 + tips；Online 区/模板缩略图/排序分页仍缺 |
| 2 | `panels/extra.rs::render` 居中 760×520 卡 | `home.rs:145` `CentralPanel`；`shortcuts_panel.rs:105` 底部锚定 | 各自定位/可拖动的独立窗口 → 统一写死的居中遮罩卡 | 🔄 Home 已占满窗口；其余四块仍用居中卡 |
| 3 | `dialogs.rs` A19 整套数据网格 | `dialogs.rs:86-97,219-365`；`dialogs/export.rs:90-151` | About 三 tab / Preferences 完整表单 / Export 分区 preset 行 → 15 种压成 message+fields+buttons | 🔄 补了 `Field::Section` / `Field::Action`，Export 按 8 preset 行重排；About tab、Preferences checkbox 等仍缺 |
| 4 | `dialogs.rs` `popover_window(Open::At(at))` | `dialogs.rs:91` `floating()`「the user can move」 | 可移动对话框 → 固定居中不可拖 | ✅ 挂 `on_drag_start`/`on_drag_move` + `popover::clamp` + 3pt 死区，接 `CanvasView` 拖动状态机 |
| 5 | `dialogs.rs` `choice_chips` | `dialogs.rs:343-352,436-475,506-545` | `widgets::dropdown` + `menu_popup` 下拉 → 一排 chips | ✅ 全改为真实 `widgets::dropdown()` + `CanvasView::open_menu` |
| 6 | `dialogs.rs` `Field::Note` | `dialogs.rs:219-228,233-365` | tab 分组 / 分区 / 表单 → 纯文本行 | 🔄 引入 `Field::Section` 分区；About/Preferences 完整表单仍缺 |
| 7 | `dialogs.rs` Export 硬编码 format 5 / scale 4 | `dialogs/export.rs:79-88,113-143` | 8 preset + 图层导出 + PDF print options → 编造的「格式/倍数下拉」 | ✅ 改回 caption + 8 preset 行 + PDF options + whole-file 行 |
| 8 | `panels/extra.rs:309-317` 分组「伪下拉」 | `shortcuts_panel.rs:108` `tab_strip` | 真 tab 条 → 点一下换一组的循环切换 | ⬜ |
| 9 | `panels/extra.rs:717-737` 版本/模式循环下拉 | `dev_compare.rs:491-492` | 带图标的 `dropdown` + `menu_popup` → 循环切换 | ⬜ |
| 10 | `panels/extra.rs:303-368` Shortcuts 单列 `take(120)` | `shortcuts_panel.rs:15,110,126-142` | 3 列 keycap 网格 / 双语搜索 / 滚动 → 单列 120 行封顶 | ⬜ |
| 11 | `popovers.rs:970-1019` `Setting::Options` chips | `layout_panel.rs` / `right/stroke.rs` | `dropdown` + `menu_popup` → chips | ⬜ |
| 12 | `panels/extra.rs:935-947` Assets 用 `pill()` 切换 | `assets_panel.rs:301-312,421-449,1085` | 真 tab + 搜索 + 两列缩略图 + 文件夹折叠 → pill + 基础网格 | ⬜ |
| 13 | `popovers.rs:380-381` 变量编辑器半张表 | `dialogs/variables_panel.rs::table()` | 左侧集合/分组 sidebar + 顶部搜索 + 编辑拖拽 → 只有右表 | ⬜ |
| 14 | `panels/extra.rs:531-599` 版本行内联 5 图标 | `versions.rs:460,514` `menu_popup(&more)` | "more" 下拉菜单 → 一排内联小按钮 | ⬜ |
| 15 | `lib.rs` palette 仅 tab + 搜索 + 点击 | `menus/palette.rs:57-61,113-141,215-241` | Up/Down 高亮 + Enter 运行 + 分区头 + 字母 tile → 无键盘导航 | ⬜ |
| 16 | `panels/right/{devmode,inspect,proto,topbar}.rs` 各自 `note()` | 各 section 的空态 | 多为空态占位，不是替代主布局 | ⬜ 低优先 |

**本轮已落地（提交见 git log）**
- `dialogs.rs`：`Field::Choice` → 真实 `widgets::dropdown` + `open_menu`（`MenuRow.checked` 显示当前项）；
  新增 `Field::Section`（caption + divider）与 `Field::Action`（点击即推引擎命令并关闭），
  Export 按 `dialogs/export.rs` 抄成 8 preset + PDF options + whole file，Preferences 按
  `dialogs.rs` 抄成 General / Interface 分区 + 下拉字段。
- 对话框外壳：通用 `Popover` 的 `on_drag_start` / `on_drag_move` + `popover::clamp` + 3pt 死区，
  并扩展 `CanvasView::begin_popup_drag` / `move_popup_drag` / `end_popup_drag` / `popup_dragging`
  让窗口级拖拽路由也覆盖对话框（指针移出卡片仍连续）。
- `panels/extra.rs`：Home 占满窗口、240pt 侧栏（wordmark + `search_field` + 3 个 section tab）、
  主区 `action_card` 三连卡、Recents 文件行、6 条 Getting started tips（原文抄自 `home.rs::TIPS`）。
- 契约加固：A19 的 `requires` 新增 `dropdown(` / `on_drag_start` / `on_drag_move`，A1 新增
  `action_card` / `home_section` / `home-screen` —— 退回 chips / note+pill / 固定窗口会直接测试失败。

**剩余待办按优先级**：#8/#9/#10/#11 的「真下拉 / 真 tab / 三列网格」→ #12/#13/#14 的整面板还原
→ #15 调色板键盘导航 → #2 其余四块整面板的可拖动外壳 → #16 空态文案。**每一项都必须先读
EGUI 原文件再动手，禁止再出现简化替代**；完成后同步 `CONTRACTS` / 代码 / 状态块。

**下一步（19:4x 修订）**：先按上表把 `panels/extra.rs` 的四块整面板与 `popovers.rs` 的
`Setting::Options` 换成真实 `dropdown` / `tab_strip` / 网格；B4 拖出成窗口（A16）需要共享
`Session` 的多窗口架构，单独排期。涉及 `widgets/**` 冻结件的改动（A24 字段运算、调色板实时过滤）
必须先与用户确认。

---

## L. 2026-11-10 21:5x 第 1–2 批功能核查（用户要求）

> 用户要求：确认第 1 批（A19 / B1C1 / B9 / A6 / A7 / A18·B4）与第 2 批（A1 / B2 / A3 / A4·A25 / A20）**功能正常运行、模块不遗漏、不重复造轮子、能用通用控件的地方都用通用控件、能用引擎能力的地方都用引擎本身**。

**审计方法**
1. `cargo test -p figcraft-ui-gpui --lib --features test-support` = **213 通过 / 3 失败**（基线 3 条：main_menu aim / extra Shift+D / panel_resize hit area），无新增回归。
2. `parity_contract` **3/3**：A19 / B1C1 / B9 / B2 / A18B4 / A1 / A6 / A7 的能力 token 与本文档状态块双向一致。
3. **命令审计**：`crates/ui-gpui/src` 里 46 个 `Cmd { id: "…" }` 字面量 id **全部**能在 `crates/engine/src` 找到定义（**0 个自造**）；对话框命令另由 `every_dialog_command_is_a_real_engine_command` 断言。
4. 逐项读源码确认模块存在、使用 `crate::widgets` 通用件、推真实引擎命令。

**逐项结果**

| 项 | 状态 | 模块 | 通用控件 | 引擎能力 | 诚实边界 |
|---|---|---|---|---|---|
| **A19** 对话框框架 | ✅ | `dialogs.rs`（15 `Kind`；A6/A7/A20/Plugins/Branch/SaveAs 已逐个深化） | `Popover`/`field_row`/`field_row_full`/`Dropdown`/`Checkbox` | 全部命令 `find_command` 可达 | notice 型（MissingFonts/ImportReport/SampledColor/Updates）引擎无命令，保持提示 |
| **B1 / C1** 命名 | ✅ | `naming.rs` | — | — | — |
| **B9** 隐藏 / 最小化 | ✅ | `lib.rs::ui_flags`/`dock` | — | `view.toggleUi`/`view.minimize` | — |
| **A6** 导出 | ✅ | `dialogs.rs::Kind::Export` + `export_preview.rs` | `field_row`/`Field::Preview`(`img`) | `file.export` / `file.exportLayers` | 多选导出 / 原生路径 / 逐格式参数未做 |
| **A7** 首选项 | ✅ | `dialogs.rs::Kind::Preferences` | `field_row`/`Dropdown`/`Checkbox` | `pref.*` / `view.theme*` | Accent / 图形后端引擎无命令，未造假控件 |
| **A18 / B4** 标签关闭 | A18 ✅ / B4 🔄 | `tabs_view.rs` + `dialogs.rs` | — | `file.close`/`file.save`/`dialog.unsaved` | B4 拖出成窗口需 `Session` 跨窗口共享重构 |
| **A1** 首页 | ✅ | `home/mod.rs`（独立模块） | `PanelSection`/`icon`/`Dropdown`/`canvas` 无 | `file.openRecent`/`file.new*`/`help.gettingStarted` | Online 在线区、缩略图卡片未做 |
| **B2** 调色板 | ✅ | `palette.rs` + `menu_card` | `menu_card`/`TextField` | `pick_palette` 把 key 转真实命令 | 搜索仍 Enter 提交（冻结 `TextField` 只有 commit） |
| **A3** 评论 | ✅ | `comments.rs` | `Dropdown`/`div` 行 | `comment.open`/`comment.markRead` | 回复 / 编辑 / 表情（thread 弹层）未接 |
| **A4 / A25** 插件 + widget 栏 | A4 🔄 / A25 ✅ | `dialogs.rs::Kind::Plugins` + `widget_bar.rs` | `field_row`/`Dropdown` | `plugin.remove`/`plugin.import`/`widget.menu` | A4 的 Browse（网络插件目录）是异步后台任务，未接 |
| **A20** 查找替换 | ✅ | `dialogs.rs::Kind::Find` | `field_row` | `edit.findReplace`（Find 不带 `replace`，修掉删匹配的 bug） | — |

**结论**：第 1–2 批 11 项中 **8 项完整、3 项有诚实边界**（B4 拖出成窗口、A4 网络 Browse、A3 thread 弹层）；模块齐全、无自造命令、通用控件与引擎能力均按 EGUI 用法接入，没有另起炉灶。

**本轮之后新增（第 3 批起步）**：画板工具条 `board_bar.rs`（颜色 / 形状网格 / 字号 / 连线，全部 `canvas()`+`paint_path` 自绘，命令 `board.setColor`/`board.setStyle`），尚未进入上面的第 1–2 批统计。

---

## M. 2026-11-10 22:3x 第 3 批 ②：画板工具栏工具组 + 模板选择器

> 用户原话：「② 工具栏画板工具组 + 模板选择器」，并要求照 `ui-egui` 结构抄，不许自己发明简化件。

### 落地内容

| 文件 | 改了什么 | 对照的 EGUI 源 |
|---|---|---|
| `toolbar.rs` | `Group` 补齐 `name`（caret 的 tooltip / a11y 名）与 `options`（`Options::{None,Colors,Shapes,Stamps}`）；`has_flyout = tools.len()>1 \|\| options != None` | `ui-egui/src/toolbar.rs:37-59` |
| `toolbar.rs` | `BOARD_GROUPS` 九组（bmove / ink / sticky / bshape / connector / text / section / stamp / comment），名字与 options 逐条照抄 | `ui-egui/src/toolbar.rs:98-113` |
| `toolbar.rs` | `groups(board)` / `group(key)` / `width(groups, board)`；画板尾段换成 **模板选择器 + Design 面板按钮** 两格（设计文件仍是 4 格模式切换器） | `main_toolbar` 的 `if board` 分支 + `main_width` 的 `tail` |
| `toolbar.rs` | `tool_last` 由「按下标」改为「按组 key」（`Vec<(&'static str, ToolId)>`），与 egui 的 `tb_last` 每 key 一条一致 —— 画板 9 组与设计 6 组不再互串记忆 | `ui-egui/src/toolbar.rs:117` `remembered()` |
| `board_bar.rs` | 新增 `Choice` / `Mark` / `choices(options, board_tool)` / `color_label` / `choice_mark` / `outline_marks`（印章 SVG 路径 → GPUI `PathBuilder`） | `ui-egui/src/canvas/board_bar.rs:34-92`、`icons/draw.rs:1145` |
| `board_picker.rs`（新文件） | 模板选择器：3 列卡片网格，每张卡渲染该模板的真实预览图（`board_templates::preview` + `render_area` + `screen_image`），末位是「表格」卡；`command_for(i)` → `board.template` / `board.table` | `ui-egui/src/widgets/board_picker.rs` |
| `lib.rs` | `board_panel` 状态 + 画板右栏默认收起（`dock` / `show_right`）；`Intent::Templates` / `Intent::DesignPanel`；`Popup::Options` / `Popup::Templates` 两个弹层；`pick_board_option` / `pick_template` | `app/frame.rs:right_panel_shown`、`app/commands.rs:133` |

### 与 EGUI 的**一处有意偏差**（已在代码注释里写明）

`ui-egui` 的 `main_width` 画板尾段只算 `CELL`（Design 面板一格），因为它的 `board_picker::button` **写好了却从没被挂到工具栏上**（全仓 `grep board_picker::button` 只有定义、无调用；`board_picker_tests.rs` 仍按「模板按钮在 Design 面板左边」断言，该测试目标本身编译不过）。GPUI 侧按用户要求把选择器挂上了，所以尾段是 `2*CELL + GAP`。这是「把原本没接上的件接上」，不是改算术。

### 验证

- `cargo test -p figcraft-ui-gpui --lib --features test-support` = **217 通过 / 3 失败**（仍是基线三条：main_menu aim / extra Shift+D / panel_resize hit area），新增 `toolbar::tests::a_board_swaps_in_its_own_groups`、`board_picker::tests::*` 两条、`parity_contract::tests::every_board_toolbar_command_is_a_real_engine_command`。
- 新增契约 **A2B3A27**（token：`BOARD_GROUPS` / `board_panel` / `board_templates` / `choices(` / `command_for` / `board.setColor` / `board.setStyle`），文档状态块 `A2B3A27=done` 与代码双向核对通过。退回「手写 pill / chips / 假选项」会直接失败。
- 起 exe 实测（`file.newBoard` 后截图）：药丸自左至右 **Move▾ Marker▾ Sticky▾ Shape▾ Connector Text Section Stamp▾ Comment │ ⚡ ｜ 模板 面板**，共 9 组 + Actions + 两格尾段；画板右栏默认不显示。

### 诚实边界（未做）

1. 模板选择器的**卡片缩略图**只在第一次打开时同步渲染 5 份画板（与 EGUI 一样缓存，EGUI 存 `ctx.data`，这里存 `CanvasView::tpl_pics`），首开会有一次卡顿。
2. 画板的 `Options` 弹出层目前**只有横向一排格子**（与 `options_flyout` 同构），没有 EGUI 的 `a11y::own_focus` 键盘循环。
3. 帮助「?」按钮已在画布右下（既有实现），本轮未动。
4. 画板下 Motion 时间轴仍可显示（EGUI 同样没有 board 守卫，属上游既有行为，非本次引入）。

---

## N. 2026-11-10 22:5x 现场反馈修复（右栏标题行 / 动画默认关 / 画板隐藏侧栏 / 首页崩溃 / 竖条）

> 用户现场指出四件事：① 右栏第 2 段（放组件名称那一行）做成了上下结构；② 打开任何东西都应默认
> 关掉动画；③ 画板模式下把左右栏、动画、左侧竖条都关掉；④ 首页 bug 很多。

### 修了什么

| # | 问题 | 根因 | 修法 |
|---|---|---|---|
| 1 | 右栏类型名出现**两次**、还多一条区块标题 | `sections::header` 返回 `PanelSection::new(类型名)` + 下面再一行同名 `field_row` | 改成**一行 40 pt**（`field_row_h(…, 40.0)`）：左 `[类型名 ▾]`（Frame/Group 才可下拉）+ Frame 的「交换方向」独立图标，右三个快捷动作。`egui right.rs::header` 本来就没有区块标题 |
| 2 | 右栏快捷动作只剩**一个**图标 | `field_row` 的 trailing 按 `SLOT * trailing.len()` 占位，却把 `icon_row([3 个])` 当成**一个**元素传进去 → 列宽只有 24 pt，后两个被裁 | 三个 `IconButton` **逐个**传进 trailing |
| 3 | 打开文件时动画时间线自动展开 | `CanvasView::new` 里 `motion: true` | 改 `false`；并在 `run_command` 里对 `file.open*` / `file.new*` / `file.close` / `file.saveAs` 调 `set_motion_open(false)`（egui `FigApp::new` 是 `motion_open: false`） |
| 4 | 画板下左右栏 / 动画栏 / 左侧竖条仍占位 | `dock()` / `show_sidebars` / `motion_bar_h()` 都没有 board 判断 | 画板：`left = 0`、`show_sidebars = false`、`motion_bar_h() = 0`；右栏仍由 `board_panel`（工具栏最右按钮 / `view.design`）控制。实测 `canvasRect` 宽度 == `vw`（画布铺满） |
| 5 | **首页点「Grid / Edited」直接 panic 退出** | `*slot.borrow_mut() = !*slot.borrow()` 是一条表达式：左边 `RefMut` 还活着时右边又 `borrow()` → `RefCell already borrowed`，`panic in a function that cannot unwind` 直接杀进程 | 先读再写。同型 3 处全修（`home::cycle_bool` / `home::cycle_uint` / `board_bar` 的形状菜单开关） |
| 6 | 首页右上角浮着一个 ✕ | 浮层外壳的关闭按钮被 Home 分支一起继承了 | 删掉：egui 的 Home 是 `CentralPanel`，没有任何关闭控件（离开靠标签栏） |
| 7 | 首页三张动作卡溢出窗口右边 | 卡片行是 `flex_row` 不换行（260 pt × 3 > 主区宽） | `flex_wrap()`（egui `horizontal_wrapped`）；Templates 段同样 |
| 8 | 首页段落顺序与 egui 不同 | GPUI 是 卡片 → Recent files → Getting started | 改成 egui `recents()` 的顺序：卡片 → Getting started → Recent files |
| 9 | 左侧竖条是三个**黑点**、还没有文字 | `Icon::Variable / Panel / Menu` 都落在占位分支画一个圆点；`Icon::Assets / Logo / Variables` 根本不存在 | 按 `ui-egui/src/icons/draw.rs` 补 `Logo` / `Assets` / `Variables` 三个变体并实现 `Variable`(六边形+点) / `Menu`(三横) / `LinkOn` / `LinkOff`；竖条重写成 egui 结构：Logo + 16 pt 分隔带 + **File / Assets / Variables**（56 pt 项，32×32 图标格 + 9 pt 文字） |

### 验证

- `cargo test -p figcraft-ui-gpui --lib --features test-support` = **217 通过 / 3 失败**（仍是那三条基线）。
- exe 实测：右栏 `[Frame ▾] ↻ … ◆ ⧉ ▦` 单行；左侧竖条 Logo + File/Assets/Variables 带文字；首页三卡换行、
  无 ✕、顺序为 卡片 → Getting started → Recent files；画板下 `canvasRect` 宽度等于视口宽度（侧栏全让位）。
- 崩溃路径：点 Grid 后进程存活（此前必崩）。

### 诚实边界（本轮没做）

1. **左栏仍缺 Pages 段和文件头**（文件名 / Drafts / 收起按钮）。egui `layers::left_panel` 是三段结构，
   GPUI 只有「Layers 标题 + 树」。这是已知差距，不在本轮范围（用户澄清说的是右栏）。
2. 首页 **Templates 段**仍是三张动作卡，没有 egui 的模板卡片网格 + 缩略图；**Online 段**是占位说明。
3. 首页搜索框只在 **Enter** 提交（冻结的 `TextField` 只有 `set_on_commit`），不是边打边过滤。
4. 「Join with a code」行未做（依赖 live join 组件）。
5. 右栏 header 的 Frame 菜单里没有 egui 的 **frame 预设列表**（需要把选区 id 带进 `node.set`）。

---

## O. 2026-11-10 23:0x 本地化根因 + 左栏三段 + 首页模板/在线 + 文件导入

### 本地化：三个真根因

1. **系统语言从来没被读取过**。`CanvasView::new` 里 `figcraft_i18n::set(&prefs.language, None)` ——
   第二个参数是「系统语言列表」，一直传 `None`，于是 `"system"` 永远回退英文。现在
   `system_language()` 直接问系统：Windows 用 `GetUserPreferredUILanguages(MUI_LANGUAGE_NAME)`
   （等价 `ui-egui` 经 `apps/figcraft` 走的 `GlobalizationPreferences::Languages()`），
   macOS 读 `AppleLanguages`，其他 unix 读 `LANGUAGE` / `LC_ALL` / `LANG`，结果用 `OnceLock` 缓存。
2. **主菜单完全没翻译**。`TOP_MENUS` 是 `const`（只能放 `tr_noop`），但**画的时候也没过 `tr`**；
   命令行走的是 `m.label` 原文，子菜单走 `sub` 原文。现在分别改成 `tr(l)` /
   `figcraft_i18n::menu_item(id, english)` / `tr(&sub)` —— 与 `ui-egui/src/menus.rs` 同一句。
   右栏 Design/Prototype 标签和选区标题行的类型名同样补上了 `tr`。
3. **目录本身缺键**。`zh-Hans.toml` 比 `en.toml` 少 **251** 个键；其他语种少 40–245 个。
   已把 **全部 24 个语种**补齐（es-419 是 `es` 的 overlay，只列差异，无需补）。
   校验：逐语种与 `en.toml` 求差集（0 缺失）、按节查重复键（0 重复）、
   `cargo clean -p figcraft-i18n` 后重建（TOML 重新解析通过）。

### 功能

| 项 | 做了什么 |
|---|---|
| **边打边过滤** | 冻结件 `TextField` 新增 `set_on_change`（平台输入 / 输入法 / 退格 / 粘贴都会触发；`set_on_commit` 只在 Enter 与失焦）。首页搜索、资源搜索、快捷键搜索、Pages 搜索全部改用它 |
| **左栏三段** | `layer_tree::render_left_panel` 按 `layers::left_panel` 补齐：64 pt 文件头（可改名 `file.rename` + `Drafts` + 收起按钮）、Pages 段（折叠 / 搜索 / 加页 / 页行 / 分隔页）、Layers 段。新增 `pages_open` / `page_search` / `page_filter` / `left_collapsed` 状态 |
| **模板卡片网格** | 首页 Templates 段换成 248×234 卡片网格，缩略图用 `Template::build` + `render_area` + `screen_image` 真渲染（缓存在 `ExtraState::template_thumbs`），点击 `file.newFromTemplate` |
| **在线段** | 列出 `prefs.online_files`（名称 + 服务器）与空态文案；引擎没有 `online.*` 命令，所以只做只读列表，不自造命令 |
| **文件导入** | `run_command` 拦截 `file.open` / `file.import` / `file.placeImage`，用原生 `rfd` 选择器取路径再执行引擎命令（扩展名表照抄 `app/files.rs::import_dialog`） |

### 诚实边界

- Pages 段还没有**拖拽排序**、**就地改名**、**把图层拖到某页**（egui `pages()` 有，`list_drag` 通用件 GPUI 侧已有但未接）。
- 在线段没有「刷新 / 打开 / 移除」动作（缺引擎命令）。
- 模板卡片没有按类别（`Kind`）过滤的分段控件，也没有独立的模板搜索框（复用侧栏搜索）。

---

## P. 2026-11-10 23:3x 导入 / 拖入按原版补齐

用户：「导入的能力严重缺失，去看原版是怎么导入、怎么处理拖入的，引擎底层已经很完备了」。

**原来缺什么**：只在 `run_command` 里拦了三条命令弹一个 `rfd` 选择器；**完全没有拖入**；
文档拖进来会被 `file.import` 当成「追加页面」而不是「开成新文件」；不认落点；错误只在
`FC_DEBUG_CMDS` 下打到 stderr。

**现在（照 `ui-egui/src/open.rs` + `canvas/input.rs::import_dropped` + `home.rs::dropped`）**：

| 件 | 内容 |
|---|---|
| `open.rs`（新） | `OPEN_EXTENSIONS` / `IMPORT_EXTENSIONS` / `IMAGE_EXTENSIONS`，`opens()` / `is_document()` / `is_image()` / `file_name()`；带 4 组单元测试（文档 / 图片 / SVG / 拒绝） |
| `CanvasView::drop_files` | 文档 → `file.open`（**开成新文件**，首页与画布都一样）；图片与 SVG → `file.import` 带**落点页面坐标** `{path, x, y}`（引擎自己分流：SVG 变框架、图片放图片）；首页只收 `opens()` 的类型，其余弹 "Drop a .figcraft, …" |
| 拖放目标 | **整个窗口**（root 上的 `.on_drop::<ExternalPaths>`），不只是画布；落点取 `window.mouse_position()`（GPUI 在 Submit 时已更新） |
| 拖入高亮 | 悬停时整窗强调色描边 + 中间「松手以打开 / 松手以导入」（`has_active_drag() && layer_tree.drag.is_none()` 判定外部拖拽；本工程只有图层树用 `on_drag`） |
| 错误反馈 | 新增 `toast`（egui `app.toast`）：底部中间圆角卡片，4 秒自动消失；`execute_reporting` 把引擎原话弹出来 |
| 原生选择器 | 三个命令共用 `open.rs` 的扩展名表（一个筛选器列全部扩展名），失败也走 toast |

**验证**：`file.import {"path", "x", "y"}` 对一张真实 PNG 返回放置好的图片节点（`{id, image, width, height}`）；
拖入高亮层实测在一次真实拖拽中出现并在松手后消失；`open.rs` 4 条测试通过。

**仍未做**：`pending_open` 那种「读取中」卡片（桌面有路径是同步读，不需要）、
从资源面板把素材拖到画布（GPUI 的资源面板还没有内容）、浏览器端的 `data` + `name` 形式。

## Q. 2026-11-11 08:2x 第 3 批 ②：A5 图标面板

### 落地内容

| 件 | 内容 | 对照的 EGUI 源 |
|---|---|---|
| `icons_panel.rs`（新） | `listed()`（名称 / 码位 / `U+` 前缀过滤，可测纯函数）、`Act` / `command()`（每个按钮 → 真实引擎命令）、`body()` 渲染搜索 + `New icon font` + 字体条目 + 图标行 | `ui-egui/src/icons_panel.rs` |
| 图标行 | 预览由 `figcraft_render::render_path` 从图标**存储的 outline** 光栅化（未编译字体也能显示），`screen_image` 转 GPUI 图；名称、`U+XXXX`、✕（`icon.delete`）、整行点击把字符写进剪贴板 | 同上 `icon_row` / `build_thumb` |
| 字体条目 | 名称 + 重建 + 删除；重建**先逐图标 `icon.update` 再 `icon.generateFont`**（否则编译的是旧轮廓），与 egui `Act::Generate` 同语义 | 同上 `font_header` / `Act::Generate` |
| `ExtraKind::Icons` | ~~沿用整面板浮层约定~~ **2026-11-11 自检更正（R.4）**：已改为左面板 `left_tab==2`；`ExtraKind::Icons` 变体与 `extra.rs::render` 分支是**未清理的死路径残留**，见 T.2 | `panels/layers.rs:200` 的 `LeftTab::Icons` 分支 |
| `ui.set {leftTab}` | `icons` → 打开 Icons、`assets` → Assets、其它 → 回到图层面板；`set` 语义幂等（不是 toggle） | `ui-egui/src/control.rs:1128-1133` |

### 验证

- `cargo test -p figcraft-ui-gpui --lib --features test-support`：**220 通过 / 3 失败**（失败仍是文档记录的 3 条基线：main_menu aim / extra Shift+D / panel_resize hit area）；新增 2 条（搜索过滤、命令真实性）全部通过。
- `cargo build -p figcraft-ui-gpui --example run`：通过。
- **真实窗口实测**：`ui.set {leftTab:"icons"}` 打开面板（标题 / 搜索 / `New icon font` / 空态可见）；
  点击 `New icon font` 后引擎返回 `{"fonts":[{"id":"iconfont-1","name":"Icons","glyphs":[]}]}`，
  面板同时出现字体条目与「No icons in this font yet.」——按钮 → 引擎 → 文档 → 重绘整条链路成立。

### 诚实边界（未做）

- 拖到画布插入文字层（egui 的 `IconDrag` + 画布落点）：GPUI 目前没有「外部 payload → 画布放置文本」这条路。
- 行内改名 / 改码位、拖拽排序：`text_field_inline` 在冻结件里，逐行内联编辑需要先确认能否复用。
- 导出字体（`icon.exportFont` + 保存对话框）。
- 字体折叠（egui 折叠状态里的 `collapsed` 集合）——本版所有字体默认展开。

---

## R. 2026-11-11 17:0x 规划错配审计：被误做成「弹窗/浮层」的内嵌交互（下次整改）

> **问题来源**：用户实测发现，画布上双击画框顶部的名字本应**就地改名**，结果 GPUI 弹出了一个对话框；
> 资源列表本应在**左栏**，结果 GPUI 也弹成了浮层。全盘核对 EGUI 原版后发现这不是个例——
> 规划文件（及 `extra.rs` 的模块注释）把一批「画布内联编辑 / 左栏 tab / 占满窗口」的交互
> 统一归成了「居中浮层 `ExtraKind`」或「对话框 `dialogs::Kind`」，与 EGUI 真实形态不符。
>
> **铁律回放**（项目记忆 `PARITY_CONTRACT` 第 6 条）：*不要把整屏页面塞进 760×520 居中弹层，
> Home 应占满窗口：240pt 侧栏 + 主区*。这条同样约束 Assets / Icons / Shortcuts / Versions 这类
> 本就属于常驻面板的交互。
> **更正（2026-11-11 17:1x 读码确认）**：`Home` 已占满窗口（`extra.rs::render` 的 `absolute` 全屏分支），
> R3 实为已修复项，不占用下次整改名额；真正未做的是 R1/R2/R4/R5/R6/R7/R8/R9。

### R.0 根因

- `crates/ui-gpui/src/panels/extra.rs` 第 9–11 行注释**明知** EGUI 把 Assets 放左栏 tab、Versions 放独立面板、
  Shortcuts 放窗口底部，却仍「统一成一个可关闭的居中浮层」，并把它当作既定设计写进了规划（A1/A5、Q 节）。
- `crates/ui-gpui/src/dialogs.rs:302` 把 `canvas.renameLabel`（画框标签改名）、`layers.rename`（图层面板改名）、
  `board.editText` 全部路由到 `Kind::Rename`（居中对话框）。这些在 EGUI 里都是**贴着控件/画布的内联编辑框**，不是弹窗。
- 左栏 rail 的 `rail-assets` 按钮（`panels.rs:1663`）`on_click` 为 `None`——点击无反应；Assets/Icons 只能靠
  `ui.set {leftTab}` / `panel.assets` 打开，而打开的仍是 `ExtraKind` 浮层，不是左面板 tab。
  **2026-11-11 自检更正**：前半已由 R.4 修复（按钮接上点击、入口改 `left_tab`）；但 `ExtraKind::Assets/Icons`
  变体与 `extra.rs::render` 的 760×520 卡片分支**仍未删除**，是死路径残留（T.2）。

### R.1 错配清单（每条都是下次整改的硬要求）

| # | 交互 | EGUI 原版形态（证据） | GPUI 现状（错误） | 整改要求 |
|---|---|---|---|---|
| R1 | **资源面板 Assets** | 左栏 tab：`LeftTab::Assets` → `assets_panel::show` 渲染进**左面板**（同 Pages/Layers 同一栏，面板宽） | `ExtraKind::Assets` 居中浮层（`extra.rs` + `ui.set{leftTab:"assets"}`） | ⚠️ **入口已改，变体未清**（T.2）：见 R.4，Assets 进左栏 tab；但 `ExtraKind::Assets` 变体 + `render` 卡片分支仍在 |
| R2 | **图标面板 Icons** | 左栏 tab：`LeftTab::Icons` → `icons_panel::show` 渲染进**左面板** | `ExtraKind::Icons` 居中浮层 | ⚠️ **入口已改，变体未清**（T.2）：同 R1 机制进左面板 tab 2；但 `ExtraKind::Icons` 变体仍在 |
| R3 | **首页 Home** | 替换**整个工作区**（占满窗口，240pt 侧栏 + 主区），不是居中小窗 | ~~`ExtraKind::Home` 居中浮层~~ **已修复**：`panels/extra.rs::render` 对 `Home` 走 `div().absolute().top(TITLE_H).left(0).right(0).bottom(0)` 占满窗口（注释明写「1:1 with egui `home.rs` CentralPanel，不是 760×520 居中卡片」） | ✅ 无需整改（视觉已占满窗口，与 EGUI 一致） |
| R4 | **画框标签改名** `canvas.renameLabel` | `canvas/frame_labels.rs::rename_field`：**在画布标签的屏幕坐标上**盖一个内联编辑框，Enter/失焦改名、Esc 保留 | `dialogs.rs:302` → `Kind::Rename` 居中对话框 | 改回画布内联编辑：双击标签 → 在标签处画 `TextField`，走 `object.rename`；不要弹窗 |
| R5 | **图层面板改名** `layers.rename` | 在 Layers 列表行内**就地**改（egui `layers_rename_tests.rs`） | `dialogs.rs:302` → `Kind::Rename` 居中对话框 | 改回列表行内联编辑（GPUI 左面板已用 `TextField` 做文件头改名，图层行同理） |
| R6 | **看板文字编辑** `board.editText` | 贴着画布的内联文字编辑 | `dialogs.rs:302` → `Kind::Rename` 居中对话框 | 改回内联编辑（依赖 A2/A27 画板模式先落地） |
| R7 | **快捷键面板 Shortcuts** | 窗口**底部**常驻条（`panels/layers.rs` 底部），非模态 | `ExtraKind::Shortcuts` 居中浮层（`extra.rs` 注释承认此差异） | 移到窗口底部 dock（或至少承认差异并登记为偏差，不要伪装成弹窗） |
| R8 | **版本历史 Versions** | 独立**面板**（左面板宽，`versions::panel`），非居中卡片 | `ExtraKind::Versions` 居中浮层 | 改成独立面板（左栏宽或右侧抽屉），不要居中卡片 |
| R9 | **Dev 对比 DevCompare** | 双图并排/叠加对比（占满工作区） | `ExtraKind::DevCompare` 居中浮层 | 占满主区做对比视图，不要居中卡片 |

### R.2 连带的规划文件错误（需同步修订）

- **A19 模块注释**（第 2 节 A 表 A19 行、以及 `dialogs.rs` 注释）把 `Shortcuts` 和 `Rename` 列为「通用对话框框架」的
  renderer。按 R4/R5/R7，这二者本不是对话框——应从 A19 的对话框清单里**移除**，改为各自的内嵌形态。
- **A1 / Q 节**把 `ExtraKind::{Home,Assets,Icons}` 当作「整面板浮层约定」正面记录。按 R1/R2/R3，这是错配，
  下次整改应把 Assets/Icons 改为左面板 tab、Home 改为占满窗口，并把这条「约定」改为「临时过渡」。
- `extra.rs` 第 9–11 行「与 egui 的差异」应改为明确标注 **TODO（错配，待 R1–R9 整改）**，不再当作既定设计。

### R.3 整改验收（下次做时对照）

1. 双击画布画框顶部标签 → 标签处出现内联输入框，回车改名、Esc 取消；**不**弹出任何窗口。
2. 点击左栏 rail 的 Assets / Icons → 左面板内容切到对应面板（与 Pages/Layers 共用同一栏与宽度），而非浮层。
3. 打开首页 → 占满窗口（含 240pt 侧栏 + 主区），非居中卡片。
4. 图层面板里改名 → 行内编辑，非对话框。
5. `dialogs.rs:302` 不再把 `canvas.renameLabel` / `layers.rename` / `board.editText` 路由到 `Kind::Rename`；
   `Kind::Rename` 仅保留给真正的「批量重命名」等本就是对话框的入口。

### R.4 2026-11-11 已实施：R1 / R2（Assets / Icons 进左栏 tab）

把资源与图标从 `ExtraKind` 居中浮层改为左面板的常驻 tab，与 EGUI `LeftTab` 一致。改动全部复用
冻结件与已有 `assets_body` / `icons_panel::body`，未触碰 `widgets/**` 通用件。

| 文件 | 改动 |
|---|---|
| `lib.rs` | `CanvasView` 新增 `left_tab: u8`（0=File / 1=Assets / 2=Icons）；`ui.set {leftTab}` 改为设 `left_tab` 并清 `extra`；`UiAction::LeftTab` 分支设 `left_tab`；新增 `on_assets` 闭包（点 rail 的 Assets 在 tab 0/1 间切换并清浮层）；`on_file` 顺带把 `left_tab` 归 0 |
| `panels/extra.rs` | `UiAction` 新增 `LeftTab(u8)`；`panel.assets` / `panel.libraries` 由 `Toggle(ExtraKind::Assets)` 改为 `LeftTab(1)`；`assets_body` 提为 `pub(crate)`；同步改了 `ui_request_dispatch_covers_every_panel` 的两条断言 |
| `panels.rs` | `left_rail` 增加 `on_assets` / `assets_active` 参数；`rail-assets` 按钮接上点击与高亮（此前 `on_click` 为 `None`，点了没反应）。**2026-11-11 18:4x 追加**：三项高亮收敛到纯函数 `rail_active(left_tab, variables_open, variables_full)`，见 S 节 |
| `layer_tree.rs` | `render_left_panel` 按 `left_tab` 分流：1 → `assets_body`、2 → `icons_panel::body`，标题用 `SectionHeader`（与 Files tab 同款） |

**验证**：`cargo check -p figcraft-ui-gpui --lib` 与 `--example run --features gpu` 均通过；
全套 `247 通过 / 3 失败`（3 条均为记录里的基线：main_menu aim / extra Shift+D / panel_resize hit area），
无新增回归；`ui_request_dispatch_covers_every_panel` 中 `panel.assets → LeftTab(1)` 断言通过。

**诚实边界（本轮没做）**：
- 左栏 rail 仍只有 Assets 按钮（无独立 Icons 按钮）；Icons 经 `ui.set {leftTab:"icons"}` 进左面板 tab 2。
  EGUI 的 rail 有 Icons 按钮——若要 1:1，需在 `left_rail` 再加一颗 `rail-icons` 按钮（R2 的余量）。
- Assets/Icons tab 下没有 EGUI 那颗「收起面板」按钮（`file_header` 里的 Panels 图标），目前靠拖动手柄收起。
- R4（画框标签内联改名）、R5（图层行内改名）、R6（看板文字）、R7（Shortcuts 窗口底部）、
  R8（Versions 独立面板）、R9（DevCompare 占满主区）仍按原计划留待下次整改。

---

## S. 2026-11-11 18:4x 左栏现场反馈：rail 单选 + 图层树滚动错位

用户现场反馈：① 修好左边文件栏；② 从文件栏切到组件库（Assets）时「单选状态切换不了，变成两个都选」，且**注意别跟变量冲突**；③ 「文件下面的那个**树**」= 左栏 Layers 图层树 / 组件树，「项目一多就乱」（用户明确纠正过不是 Pages 列表）。

### S.1 遗漏检查（用户要求「检查是否有遗漏」）

- 全仓 `overflow_y_scroll` 逐处对照：`home/mod.rs`、`layer_tree.rs::pages-list`、`lib.rs:7632`、`motion/inspector.rs`、`panels/extra.rs`（extra-body / compare-changes）、`panels/right/variables.rs`（var-collections / var-groups）、`panels.rs`（right-body）、`popovers.rs`、`widgets/menu_popup.rs`、`virtual_list.rs` —— 都是普通 flex 列表 + 滚动，**只有 `layer_tree.rs::render_layer_tree` 把选中高亮做成容器级 `absolute` 浮层**，是唯一的同类错位隐患。
- `panels/extra.rs::pill` 是本地函数（第 459 行），Assets 的 Components / Styles 由同一个 `assets_tab: Rc<RefCell<bool>>` 互斥派生；`panels/right/variables.rs` 既不用 `pill` 也不引用 `assets_tab`。本轮改动没有碰这两处，**不会与变量面板冲突**。
- R.4 表格中 `left_rail 增加 on_assets / assets_active 参数` 的描述随本轮签名收敛而过期，已就地补注。

### S.2 ①② rail 三项高亮互斥（用户报的「两个都选」）

| | 根因 | 修法 |
|---|---|---|
| 两个都选 | `left_rail` 里 File 高亮是 `!variables_tab`（只要不是变量全屏就亮），Assets 是 `left_tab==1`；点 Assets 后 `left_tab=1` 而 File 仍亮 → 两项同亮 | 新增纯函数 `panels::rail_active(left_tab, variables_open, variables_full) -> (file, assets, variables)`，三项**严格互斥**；`left_rail` 签名收敛为 `(left_tab, variables_open, variables_full)`，`lib.rs` 调用点不再各算一份 |
| 别跟变量冲突 | `on_variables` 不重置 `left_tab`；变量全屏时若 `left_tab==1`，Assets 与 Variables 会同亮 | 变量全屏（`variables_open && variables_full`）优先级最高，此时 File / Assets 一律熄灭；浮动小窗（`variables_full=false`）仍高亮 File，符合 2026-11-08 既定规则；`variables_full` 作为会话记忆，关闭后不会泄漏高亮 |

`panels.rs` 新增 `rail_active_tests` 4 条回归测试（穷举 `left_tab 0..=4 × open × full` 全部组合断言至多一个高亮、`切到 Assets 时 File 灭`、`浮动小窗仍高亮 File`、`全屏变量压过 left_tab 且关闭后不泄漏`），把规则锁死，防止再犯。

### S.3 ③ 图层树节点多就乱

| | 根因 | 修法 |
|---|---|---|
| 滚动后选中块与行错位 | `render_layer_tree` 把整个 run / band 用**容器级** `absolute().top(i*ROW_H)` 浮层盖在 `overflow_y_scroll` 容器上；行数少不滚时看不出，节点一多需要滚动，浮层与行位移不一致 → 视觉上「树乱了」 | 选中高亮改为**逐行**绘制：整树的 run 边界与 band 实色预算成 `(run 顶, run 底, band 色)` 下发给 `layer_row`，画在每行自己的 `relative()+absolute()` 高亮块里，跟着行一起滚动 |
| 连续选中仍要读成一整块 | 逐行画若每行都内缩 4pt 会变成一堆断开的小圆角块 | `pad_top`/`pad_bottom` 只在 run 首/末行取 4pt、run 内相邻行取 0，配合只在 run 上下两端加圆角 —— 1:1 保留 EGUI `layers.rs` 的 run 浅底 + band 实色 + 接缝直角 + 两端圆角 |

未新增任何控件 / 命令，改的都是行内 `relative()+absolute()` 这一既有通用行为；`AnyElement` 仍由 `layer_overlay` 使用，无遗留死代码。

> **2026-11-11 18:5x 用户口径更正（覆盖上表第二行，详见 T.3）**：不再把连续选中拼成一整块。
> 每个色块独立 24 高、上下各留 4 → 相邻块间距恒为 8；`selection_runs` 随之删除。
> 这是**有意的产品口径偏差**（EGUI 是 run 连块 + 16 缩进），不是回归。

### S.4 验证

- `cargo check -p figcraft-ui-gpui --lib`：RC=0。
- `cargo test -p figcraft-ui-gpui --lib --features test-support`：**247 通过 / 3 失败 / 3 忽略**；3 条失败均为记录在案的基线（main_menu aim、extra Shift+D、panel_resize hit area），**无新增回归**。
- 新增 `rail_active_tests` 4 条全绿。
- 重编 `run.exe` 并用 explorer 托管启动新窗口供现场验证。

### S.5 诚实边界（本轮没做）

- **图层树仍是全量渲染，没有接入 `panels/right/virtual_list.rs` 虚拟化**。EGUI `layers.rs::layers` 用 `show_viewport` 只画视口内的行（第 919–956 行），本轮只修了「高亮错位」，没有做虚拟化；节点极多时的性能差距仍在。`collect_layer_rows` 也没有 EGUI 的 5000 行上限。
- Pages 段仍缺拖拽排序、就地改名、图层拖到某页（O 节已记）。
- 左栏 rail 仍无独立 Icons 按钮、Assets/Icons tab 下没有「收起面板」按钮（R.4 余量）。
- R4–R9 仍按原计划留待下次整改。

**当前下一步（2026-11-11 自检更正）**：G 表第 7 行 / B2 三 tab 已 ✅（H 节 18:4x），原写「继续第 7 行」是落后描述；
实际剩余是 B2 的**键盘上下键导航 + Enter 运行 + 实时过滤**，以及 T.2 的弹窗化残留清理。
若要彻底对齐 EGUI 的「节点极多也不卡」，把 `render_layer_tree` 接到现成的 `v_virtual_list`。
涉及 `widgets/**` 冻结件的改动（A24）必须先与用户确认。

---

## T. 2026-11-11 18:5x 差异表全面自检 + 图层树 24pt 规格

用户要求：「对这个差异表格进行一下自检，看看是不是有落后的，是不是还有要更新的，之前没有检查到的。
或者判断失误的，**之前有把任何面板都当成弹窗来做的重大失误**」，并明确「不要死板执行，按照最好的方法来，
只要数值对就行」。

### T.1 自检方法

不是只看表格，而是**逐条回源码核对**：`grep` 命令 id / `Kind` 变体 / `UiAction` 分支 / `render()` 形态，
再与表格的「现状」列对照。共核对 A 表 28 项中的可疑项、B/C 表状态、G 表 21 行、R 节 9 条、Q 节、
`parity_contract.rs`、以及所有 `overflow_y_scroll` 容器。

### T.2 结论一：**「把面板当弹窗做」的重大失误没清干净**（最重要）

R 节（17:0x 审计）把 R1/R2 标成「✅ 已实施」，但回源码核对发现**只改了入口，没删变体**：

| 残留 | 证据 | 结论 |
|---|---|---|
| `ExtraKind::Assets` / `ExtraKind::Icons` | `panels/extra.rs:266-267` 仍在 `render()` 分支里；`lib.rs:1704` 仍判断这两个变体；测试 `extra.rs:1325,1327` 仍构造它们 | 入口已改走 `left_tab`，但**死路径未删**，容易再次被误当作「浮层约定」。R.1 表 R1/R2 已改为 ⚠️ |
| **R7 快捷键 Shortcuts** | `ui_action:106` → `Toggle(ExtraKind::Shortcuts)` → `extra.rs:292-337` **760×520 居中卡片** | 仍是弹窗，EGUI 是窗口底部常驻条 |
| **R8 版本历史 Versions** | `ui_action:107` → `Toggle(ExtraKind::Versions)` → 同一张居中卡片 | 仍是弹窗，EGUI 是独立面板 |
| **R9 Dev 对比** | `ui_action:109` → `Compare` → `ExtraKind::DevCompare` → 同一张居中卡片 | 仍是弹窗，EGUI 占满工作区 |
| **R4/R5/R6 内联改名** | `dialogs.rs:302` 仍把 `layers.rename` / `canvas.renameLabel` / `board.editText` 路由到 `Kind::Rename` 居中对话框 | 仍是弹窗，EGUI 是控件处内联编辑 |
| R3 首页 Home | `extra.rs:270-287` `absolute()` 占满窗口 | ✅ 确已修复，无误判 |

⇒ **9 条错配里，真正闭环的只有 R3；R1/R2 半闭环（入口改了、变体没删）；R4–R9 六条完全没动。**
这是本次自检发现的最大遗漏，不能再用「已实施」概括。

### T.3 结论二：判断失误（表格写「没有」，源码里已落地）

| 行 | 表格旧说法 | 源码实际 | 处理 |
|---|---|---|---|
| A13 Missing fonts | 「0 处引用」 | `dialogs.rs::Kind::MissingFonts`（`dialog.missingFonts` → Spec + 形状测试 + 3 处断言） | 已就地更正为已落地 |
| A20 Find and replace | 「`edit.findReplace` / `find_replace` 全 0 命中」 | `Kind::Find`，`dialog.find` / `edit.findReplace` → Spec + Find / Replace all 两按钮 + 3 条测试 | 已就地更正 |
| A19 通用对话框框架 | 「`tabs_view.rs:11` 自认还没有对话框框架」 | `dialogs.rs` 已落地，`Kind::ALL` 16 种，且被 `parity_contract` 双向比对 | 标注为 2026-11-10 扫描快照、已过时 |
| A2 画板 | 「模板选择器也一起缺」 | 模板选择器 M 节已落地（`board_picker.rs`） | 已就地更正 |
| A5 / Q 节 Icons | 「与 Assets 同一套整面板浮层约定」 | R.4 已改为左面板 tab 2 | 已就地更正，并标出变体残留 |
| S.5 下一步 | 「继续 G 表第 7 行（调色板三 tab）」 | G 表第 7 行 / B2 三 tab 已 ✅（H 节 18:4x） | 改为实际剩余：键盘导航 / Enter / 实时过滤 |

**未误报的**：A28 Live 协作。源码里 `canvas::Live` 是「拖动时当帧重抓的实时图层补丁」，
不是协作光标 / 聊天 / spotlight；表格写「GPUI 完全没有 live 协作」仍成立，不能因 grep 到 `live` 就翻案。

### T.4 结论三：之前没检查到的

- **防回归机制与规划表脱节**：`parity_contract.rs` 的 `Kind::ALL`（16 种）早已覆盖 Find / MissingFonts，
  但 A 表仍写「0 命中」——说明 `CONTRACTS` 只锁「代码有没有」，锁不住「表格有没有更新」。
  后续应把 A 表的「现状」列也纳入双向比对，或至少每次落地对话框后同步 A 表。
- **状态标记未收敛**：全文 15 个 ⬜ / 0 个 🔄 / 51 个 ✅，但 G 表 12–15 行的 ⬜ 里，
  部分（A21 a11y、A23 services、A24 field_math）确实没做，属真实未完成，不算遗漏。
- `extra.rs` 第 9–11 行模块注释仍把 Assets 写成「与 egui 的差异 / 居中浮层」，R.2 早已要求标注为 TODO，
  **该注释至今未改**——这是规划错配的源头，下次整改应一并处理。

### T.5 本轮代码：图层树 24pt 规格（用户口径，有意偏离 EGUI 的 32/16）

用户明确数值，**只要数值对、结构取最直白的做法**，不强行 1:1 复刻 EGUI 的 run 连块：

| 项 | 数值 | 落地 |
|---|---|---|
| 图标 / 单个选中色块高度 | **24** | `ICON_BOX = 24`；所有图标格 24×24（前置 chevron、节点类型、后置 eye / lock） |
| **相邻单独色块间距** | **8** | `ROW_H = 32`（步进）、`BLOCK_PAD = 4` 上下各留 → 块高 24、块间距 8 |
| 每层子集缩进 | **24** | `LAYER_INDENT = 24`（EGUI 是 16，按用户口径覆盖） |
| 项多被压缩 | — | 每行加 `.flex_none()` 钉死 32 步进，超出由 `overflow_y_scroll` 滚动，不再被 flex 压扁 |
| 父子选中不是深浅差异 | — | `theme.rs` 的 `selected_row_child` 由硬编码浅绿改为**同一品牌色派生**（亮 0.93 / 暗 0.90），与 `selected_row`（0.86 / 0.85）构成深浅；父集取 band 实色、子集取浅色 |
| 连续选中拼块 | — | 按用户「每个单独色块间距 8」，**不再拼成一整块**；`selection_runs()` 随之删除，每块独立圆角 |

改动全部复用既有通用行为（行内 `relative()+absolute()`、既有 `marks` / `run_color` / `Icon` / `paint_icon`），
未新增控件、未碰 `widgets/**` 冻结件、未新增命令。

### T.6 验证

- `cargo check -p figcraft-ui-gpui --lib`：RC=0；两个改动文件 CRLF 完整（`layer_tree.rs` 859、`theme.rs` 290，无裸 LF）。
- `cargo test -p figcraft-ui-gpui --lib --features test-support`：**251 通过 / 3 失败 / 3 忽略**；
  3 条失败仍是记录在案的基线（main_menu aim / extra Shift+D / panel_resize hit area），**无新增回归**（比上轮多 4 条通过）。
- 改动文件：`crates/ui-gpui/src/layer_tree.rs`、`crates/ui-gpui/src/theme.rs`、`docs/gpui-egui-parity-plan.md`。

### T.7 诚实边界（本轮没做）

- **T.2 的六条弹窗化残留一条都没改**：本轮只做「自检 + 记录」，没有动 R4–R9 的实现；
  它们是下一次整改的硬要求，不能因为写进了表格就当完成。
- 图层树仍未接入 `v_virtual_list` 虚拟化；节点极多时的性能差距仍在。
- `ExtraKind::Assets/Icons` 死路径未删（涉及 `extra.rs` / `lib.rs` / 测试，需单独一轮并核对 WIP）。
- A21 a11y / A23 services / A24 field_math（冻结件，改前须问用户）仍为 ⬜。

**当前下一步**：优先级 ① 清理 T.2 的六条弹窗化残留（R4–R9）与 `ExtraKind` 死变体；
② B2 剩余的键盘导航 / Enter / 实时过滤；③ 图层树虚拟化。涉及 `widgets/**` 冻结件的改动必须先与用户确认。

---

## U. 2026-11-11 19:1x 图层树虚拟化 + 父子连块 / 图标规格回正

### U.1 virtual_list 虚拟化（用户要求「现在接 virtual_list 虚拟化」）

`render_layer_tree` 由「每帧渲染全部行」改为 `crate::panels::right::virtual_list::v_virtual_list`：
- 全量 `rows` / `drag_rows` **仍保留**在 `LayerTreeState`，只把可见区间交给渲染闭包；
- 行渲染快照 `Rc<Vec<(LayerRow, Mark, Option<Rgba>, bool, bool)>>` 供 `'static` 闭包使用；
  闭包只拿到 `&mut App`，故 `layer_row` 改成关联函数，事件回调由捕获的 `Entity<CanvasView>`
  走 `me.update(app, ...)`（原先的 `cx.listener` 依赖 `Context<Self>`，闭包里拿不到）；
- `LayerTreeState` 新增 `scroll: VirtualListScrollHandle` 跨帧保留滚动位置；
- **拖拽不受影响**：`drag::calibrate` 本就用「命中行真实矩形 + 固定 pitch」推算整列，
  全局 `layer_on_global_move` 继续按全量 `drag_rows` 解析落点；`on_drag` / `on_drag_move`
  仍挂在每行的 `Stateful<Div>` 上。虚拟列表按 `ix*ROW_H` 精确定位、无额外 gap。

### U.2 用户现场纠正：两处回正

| 现象 | 我的误判 | 正解 / 修法 |
|---|---|---|
| 展开父项后子项「不连续」、跟原样式违背 | T 节把「每一个单独颜色块之间的间距是 8」误读成「**每行都独立成块**」，删掉了 `selection_runs`，父子选中的整块被拆散 | 恢复 `selection_runs()`：一个连续 run 是**一片**浅底（父深子浅），只在 run 首/末行上下各缩 4pt 并加圆角，run 内相邻行不缩 → 连成一片；**不同 run 之间**才留 4+4=8pt。父集用 band 实色、子集用同色系浅色 |
| 图标「快被干到 24 了，很怪」 | 把 `ICON_BOX=24` 同时用作**格子**和 `paint_icon` 的**字形**尺寸 | `ICON_BOX=24` 只做点击格子（24×24），新增 `ICON_GLYPH=16`：chevron / eye / lock / 节点类型图标的字形一律 16，节点图标额外居中 |

### U.3 数值（保持 T 节口径不变）

`ROW_H=32`（步进 = 色块 24 + 块间 8）、`BLOCK_PAD=4`、`ICON_BOX=24`、`ICON_GLYPH=16`、
`LAYER_INDENT=24`、`ROW_INSET=8`；每行 `.flex_none()` 钉死高度防 flex 压缩。

### U.4 验证

- `cargo check -p figcraft-ui-gpui --lib`：RC=0。
- `cargo test -p figcraft-ui-gpui --lib --features test-support`：**251 通过 / 3 失败 / 3 忽略**；
  3 条仍是记录在案的基线（main_menu aim / extra Shift+D / panel_resize hit area），无新增回归。
- `run.exe` 重编并重启窗口。改动文件仅 `crates/ui-gpui/src/layer_tree.rs`（未碰任何 WIP 文件）。

### U.5 未完成（诚实边界）

- **本地样式 1:1 还原（圆形色块版）尚未落地**。已定位权威参照为
  `UI/local-styles-desigma-layout` 分支的 `ui-egui/src/style_picker.rs::local_styles`
  （`paint_dot` 用 `circle_filled` 画圆、`LS_*` 全套布局常量、文件夹树 `entries`/`flatten`、
  `drop_move` 落点 → `style.move`）。GPUI 侧对应位置是右栏 `sections.rs::page()` 的
  「Local styles」节，目前仍是简化版（四组小标题 + 平铺行，方形/图标 badge）。
- 全盘搜过所有分支 / worktree / stash / 历史，**没有 GPUI 现成的圆形色块版本地样式模块**；
  用户所说「我之前写的最新版」如需另有出处，请指明路径，避免我自行重做一个简化件。
- 该项要动 `data.rs` / `sections.rs`（两处均为未提交 WIP 重灾区，合计约 895 行在制改动），
  故本轮未擅自叠加；实现方案：新建 `panels/right/local_styles.rs` 独立模块 + 精确插入接线，
  拖拽复用 `crate::drag` 通用件，落点照 `drop_move`。

## V. 2026-11-11 20:4x 右栏 Local styles 1:1 还原（补上 U.5 的缺口）

用户现场反馈：*「右栏的那个本地样式还是没有还原啊，连功能都缺失」*。本轮把
`ui-egui/src/widgets/style_picker.rs::local_styles` 的结构与动作 1:1 搬进
`crates/ui-gpui/src/panels/right/sections.rs::page()` 的第三小节。

### V.1 数据层（`panels/right/data.rs`）

`PanelData.styles` 由 `Vec<(String, String, &'static str)>` 改为 `Vec<LocalStyleRow>`：
新增 `id / name / ty / paint / text_meta / description` 六个字段。`paint` 取 `StyleKind::Paint`
的**最后一个** paint（egui `paint_dot` 画的就是它），`text_meta` 预格式化为
`"{font_size}/{line_height.label()}"`（egui `text_meta`），避免面板每帧回读 doc。
`d.styles` 在 `read()` 的**空选区提前 return 之前**填充（Local styles 属于 Page 节）。

### V.2 渲染（`panels/right/sections.rs`）

- **四组**：顺序与 egui `StyleSlot` 一致 `[Text, Fill, Effect, Grid]` → `text / paint / effect / grid`；
  空组 `continue`（egui 同样跳过空组）。
- **节标题 `+`**：`style.create {type, name}` 的「建哪种样式」菜单（Text/Color/Effect/Grid），
  与 egui 头部 `+` 的 `menu_popup` 一致；`ITEMS` 用 `static` 防闭包借本地数组越界。
- **每组小标题**：32pt caption（egui `LS_CAPTION_H`）+ 组内 `+`（直接建该类型）。
- **文件夹树**：`ls_entries / ls_branch_of / ls_flatten` 逐行对应 egui 的
  `entries / branch_of / flatten`；`/` 分隔的路径折成层级，空目录排最前。
- **样式行**：左侧 16pt 徽标 —— paint 用**圆形** `swatch`（`rounded_full`）、text 用 `Ag`、
  effect/grid 用类型图标（GPUI 无 `Icon::Effects`/`Icon::Grid`，分别用 `Star`/`ViewGrid`）；
  名字占满中间，文本样式的 `32/Auto` **贴行尾**（egui 右对齐在 `r.max.x - LS_TRAIL`）；
  层级 `depth * 24` 缩进（egui `LS_STEP`）；整行悬停 `t.hover` 圆角 5 高亮。
- **动作**：左键 → `style.apply {style}`；右键 → `Edit style` / `Duplicate` / `Delete`
  （`style.duplicate` / `style.delete`），全部走既有 `f.cmds` 通道。
  - **`Edit style` 为占位**：GPUI 无 `edit_local` 编辑器浮层，暂用现有「样式列表弹窗」
    （`open_list_window` + `ListWindow::Styles`，与 `style_menu` 同款，点击即 `style.apply`）顶替；
    真 1:1 需单独立项移植 `edit_local`。

### V.3 与 EGUI 的**有意偏差**（诚实边界）

1. **左键 / 右键 Edit 语义**：egui 左键、右键 `Edit style` 都打开 `edit_local` 样式编辑器浮层
   （`Picker { edit }`）；GPUI 尚无该浮层，本轮左键改为 `style.apply`（应用到当前选区，可撤销），
   右键 `Edit style` 暂用现有「样式列表弹窗」（`open_list_window` + `ListWindow::Styles`）占位。
   **样式编辑器浮层（`edit_local`）未移植**，如需 1:1 应单独立项。
2. **行高**：egui 样式/文件夹行 `LS_ROW_H = 24`；GPUI 复用通用件 `field_row_full`（`ROW_H = 32`），
   未改冻结件 `panel_section.rs`。caption 已与 egui 的 32pt 一致。
3. **文件夹交互已 1:1**（2026-11-12 补齐，见 V.5）：折叠态 `local_style_folded: Rc<RefCell<HashSet<String>>>`
   （键 `"<type>/<folder>"`）左键整行切换、chevron 随开合翻转（`ChevronDown`/`ChevronRight`）；
   文件夹行尾 `+` → `style.create {type, name:"<folder>/<default>"}`（与 egui `ls_add_in` 一致）；
   右键「Rename folder」→ 内联复用通用 `TextField`（`local_style_rename_input`，一次性预填当前名、
   `sync` 用 `mem::take` 不会覆盖草稿），提交即 `style.renameFolder {folder, name}`
   （空名 / 同名直接丢弃，防误提交）。三处均照搬 `style_picker.rs::local_styles`，未自造控件。
   **仍缺**：`style.move` 拖拽重排（`drop_move`）—— egui 整组是一条 drag 线，GPUI 尚未接 `ListDrag`。

### V.4 验证

- `cargo check -p figcraft-ui-gpui --lib`：RC=0，无新错误（增量缓存锁文件告警可忽略）。
- `cargo test -p figcraft-ui-gpui --lib --features test-support`：稳定基线 **278 通过 / 3 失败 / 3 忽略**；
  3 条为记录在案的基线（main_menu aim / extra Shift+D / panel_resize hit area），与本次改动无关。
  `present::tests` 下 3 个帧时序测试为**环境抖动**（headless 负载下偶发失败，空闲时连跑 3 遍全过）。
- 文件夹交互补齐（V.5，2026-11-12）：`cargo test` 仍为 **3 条基线失败、无新增回归**；
  基线通过数由 270 升到 278（其它分支同期并入的测试，非本次新增）。
- 改动文件：`crates/ui-gpui/src/panels.rs`（`RightFields` 增 `local_style_renaming` /
  `local_style_rename_input` / `local_style_folded` 三个字段）、
  `crates/ui-gpui/src/panels/right/sections.rs`（`local_style_folder` 重写 + `ls_entries` 接 `folded`）、
  `crates/ui-gpui/src/panels/right/data.rs`、`docs/gpui-egui-parity-plan.md`（本 V 节）。

### V.5 2026-11-12 文件夹交互补齐（折叠 / 行尾 + / 改名）

用户原话「啃上面哪个缺口，记得用可复用」→ 选 V.3 第 3 条里除 `style.move` 外的三件事，全部复用现有通用件：

- **折叠**：`LsLine::Folder` 增加 `path` 字段（沿用 `ls_branch_of` 的文件夹路径），`ls_entries` 的
  `folded` 闭包读 `local_style_folded`（键 `"<type>/<folder>"`），与 egui `folded_id` 一致；
  左键整行 `on_mouse_down(Left)` 切 `insert/remove`，chevron 在 `ChevronDown`/`ChevronRight` 间翻转。
- **行尾 `+`**：`local_style_folder` 末尾挂 `IconButton::Plus`，`on_click` 推
  `style.create {type: ty, name: "<path>/<ls_default_name(ty)>"}`（`ls_default_name` 对齐 egui
  `StyleSlot::default_name`：Text/Color/Effect/Grid）。
- **改名**：右键菜单「Rename folder」→ `local_style_renaming.borrow_mut().replace(path)`，
  并在同一 `on_pick` 里 `local_style_rename_input.update(set_value(path))` 预填；下一帧 `local_style_folder`
  见 `renaming == Some(path)` 即把行内 `label` 换成该 `TextField`。提交走 `local_style_rename_input` 的
  `on_commit`（建在 `RightFields::new`，读 `renaming.take()` 推 `style.renameFolder`），空名/同名丢弃。
- **未做**：`style.move` 拖拽重排（egui 走整组 `ListDrag` + `drop_move`，GPUI 未接 `ListDrag`）。

---

## W. 2026-11-11 21:0x 进度核对 + T.7 收尾（死变体 / palette 实时过滤 / R4–R6 内联改名）

### W.0 开工前状态（先核对，再动手）

- 分支 `GPUI-smooth`，最新提交 `af2aadc3`（11-11 19:35），**之后的工作全在工作区未提交**：
  32 个已跟踪文件约 **4900 行**在制改动 + 15 个未跟踪文件（整套 WGSL 材质、
  `render/glass.rs` 的 frost 链、Type settings 弹层、右栏 Local styles、`.tmp/` 脚本等）。
- 远端 `93b85863` 的 `lib.rs` 引用了没进仓库的 WGSL 模块；用户 18:4x 已拍板
  **「保持现状（不修远端），等 WGSL 工作做完一起提交」**。本轮没有 force push / 补推 / revert。
- 基线：`cargo test -p figcraft-ui-gpui --lib --features test-support` = 251 通过 / 3 失败 / 3 忽略。
- 文档 T.7 排的下一步：① 清 T.2 六条弹窗化残留 + `ExtraKind` 死变体；② B2 键盘导航 / Enter /
  实时过滤；③ 图层树虚拟化（③ 已于 U 节完成）。本轮做 ① 的一部分与 ②，并顺带把 R4–R6 落地。

### W.1 本轮做了什么

| # | 问题 | 修法 |
|---|---|---|
| 1 | R1/R2 入口已在 R.4 搬进左栏 tab，但 `ExtraKind::Assets / Icons` 变体、`title()` 两臂、`render()` 的 760×520 卡片分支、`lib.rs` 的 `matches!` 守卫、测试数组都还在（T.2 死路径） | 全部删除；`assets_body` / `icons_panel::body` 保留（左栏 tab 直接调）。`extra.rs` 文件头把「与 egui 的差异」改成明确的 **TODO（规划错配，R.7/R.8/R.9 未整改）**，不再当作既定设计；`lib.rs` 的 `ui.set {leftTab}` 分支不再判断死变体 |
| 2 | B2 命令面板只在 **Enter** 时过滤（T.7 记的「实时过滤」未做）；Up/Down + Enter 其实已有 | `palette_field` 加 `set_on_change`：每敲一个字重排 + 高亮复位第一行；`set_on_commit` 简化为「直接运行高亮行」，去掉「第一次 Enter 过滤、第二次 Enter 才执行」的两段式 |
| 3 | **R4/R5/R6 三个 ui_request 被 `dialogs::ui_action` 一律变成居中 `Kind::Rename` 对话框**（用户实测发现：双击画框标签 / 图层改名都弹窗） | 照 `ui-egui/src/app/commands.rs:135,252,255` 分流，在 `sync_ui_requests` 里**先于** `dialogs::ui_action` 接住：`canvas.renameLabel` → 画布标签上的内联输入框；`layers.rename` 单 id → 图层树**那一行内**的输入框，多 id 才开批量重命名对话框；`board.editText` → 直接调 `figcraft_engine::board::edit_text`（根本不是对话框）。提交统一走 `object.rename {ids, name}`，Escape 取消，失焦提交 |

**内联改名的实现要点**（`lib.rs` + `layer_tree.rs`）：一个跨帧复用的 `Entity<TextField>`，
`Label` 形态在根 div 上按 `tool.labels` 的 quad 绝对定位（页面坐标 → `to_screen`，逐帧重算，
平移缩放会跟着走），`Layer` 形态由 `render_layer_tree` 把 `(id, field)` 捕获进虚拟列表闭包、
在那一行把名字槽换成输入框。焦点由 render 里连续几帧 `focus_editor` 拿到（GPUI 要等元素挂载），
拿到焦点后失焦即结束。
★ 坑：`focus_editor` 发生在 `TextField::render` 消费 pending 之前，会把旧的空值快照进编辑缓冲；
因此 `begin_inline_rename` 除了 `tf.set_value()` 还必须同步 `tf.edit().set_value()`，否则框是空的。

### W.2 验证

- `cargo check -p figcraft-ui-gpui --lib`：RC=0；`cargo build --example run`：成功。
- 全套测试 **251 通过 / 3 失败 / 3 忽略**，3 条仍是基线（main_menu aim / extra Shift+D /
  panel_resize hit area），无新增回归。`dialogs.rs` 新增 4 条断言（三个命令不再是对话框、
  `dialog.batchRename` 仍是）。
- **exe 实测**（控制通道 7811）：
  - `select.all` → `object.rename {}` = 多选 → 居中「重命名」批量对话框 ✅（EGUI 同款行为）；
  - `select.set {ids:[2]}` → `object.rename {}` = 单 id → **图层行内**出现输入框，初值 `Frame 1`，
    **没有任何居中对话框** ✅；
  - 键入 ` - Renamed` + Enter → 行名变 `Frame 1 - Renamed`，输入框收回成普通标签 ✅。
- 顺手修 `.tmp/grab.py`：`SW_RESTORE` 会把最大化窗口还原成旧的 4K 矩形、覆盖之前
  `SetWindowPos` 设的尺寸，导致截图拍到桌面。现在还原后 `SW_SHOWNORMAL` + 重新设尺寸。

### W.3 诚实边界（本轮没做）

1. **R7 / R8 / R9 未整改**：Shortcuts 仍是居中浮层（EGUI 是窗口底部常驻条）、
   Versions 仍是居中卡片（EGUI 是独立面板）、DevCompare 仍是居中卡片（EGUI 占满主区）。
   `extra.rs` 文件头已明确标成 TODO，不伪装成完成。
2. **R4 未能在 exe 上复现**：引擎 `label_at` 依赖 `tool.labels`，而它只在标签实际绘制时
   （`overlays()` 的 `focus.is_none() && frame_like`）才填充；控制通道注入的双击没有命中标签。
   EGUI 有同样的依赖。GPUI 侧已加兜底：`tool.labels` 里找不到该 id 时按节点世界变换算
   帧左上角，保证不会出现「抢了键盘却看不见的输入框」。
3. 图层行内改名没有 **Tab / Shift+Tab 跳到下一行**（EGUI `take_tab` 有）；旋转帧的标签
   保持水平是 GPUI 画布的既有已知限制，内联框沿用。
4. `ExtraKind` 现在只剩 `Shortcuts / Versions / DevCompare / Home` 四个变体。
5. **本轮改动未提交、未推送**：工作区有用户在制的 WGSL 等 4900 行 WIP，整文件 `git add`
   会夹带（18:4x 的教训）；按用户「等 WGSL 做完一起提交」的决定，保持工作区原样。

---

## X. 2026-11-13 审计：计划表已滞后于代码 + 第 3/4 批推进

> 用户要求「继续第 3 批、第 3 批做完做第 4 批」。先核对了 `crates/ui-gpui/src` 与 `crates/ui-egui/src`
> 的真实状态，结论：**A/B/C/G 表严重滞后**——大量条目在代码里已经做成对话框 `Kind`、模块或空壳，
> 但计划表仍写着「0 命中 / 完全缺失」。下面按代码实况重审，作为后续推进的基线。

### X.0 代码实况（与计划表逐项对账）

| 计划表条目 | 计划表旧口径 | 代码实况（2026-11-13） |
|---|---|---|
| A4 插件面板 | 「0 命中」 | `dialogs.rs::Kind::Plugins` 已落地：已装列表（名称/版本/简介/Remove）+ `plugin.import`，仅缺「Browse 联网拉取插件目录」后台任务 |
| A8 批量重命名 | 「0 命中」 | ✅ **本日 1:1 落地**（Y 节）：Match / 正则 / Rename to / `$& $n $N` 三按钮 / Start / 前 5 行实时预览 + 错误 + and N more，提交 `object.batchRename`；不再是单一 Name 字段 |
| A9 分支 UI | 「0 命中」 | `Kind::Branch` 已落地：列表（Open/Archive）+ 创建；缺并排 review 窗口 |
| A10 色彩配置 | 「仅注释提到」 | `Kind::ColorProfile` **本日已 1:1**（sRGB/Display P3 单选起始于当前配置 + Keep appearance/Keep values → `file.colorProfile`） |
| A11 Credits | 「0 引用」 | `Kind::About` 仅版本/许可提示，**未做 egui 670 行贡献者表** |
| A11/A13 缺失字体 | 「0 命中」 | `Kind::MissingFonts` 已存在，但**引擎无 `file.missingFonts` 命令**（grep 无命中）→ 只能做提示窗，不能假造修复按钮 |
| A12 导入报告 | 「0 引用」 | `Kind::ImportReport` 已映射，但**引擎根本不发 `dialog.importReport` 请求**（grep 仅在 GPUI 映射里出现）→ 孤儿对话框，完整移植需在命令执行路径捕获导入 `warnings` |
| A15 取色器 | 「0 命中」 | `Kind::SampledColor` 已存在（提示窗）；egui 的 Style/Variable+集合+Create 未搬（需为 `Action` 加「按字段选命令」变体） |
| A17 菜单取值 | 「无对应实现」 | `Kind::Link` 已 1:1（URL 输入 → `text.link`）；egui 的 Offset/移动到页两类未搬 |
| B8 Present | 「约 1/6」 | `present.rs` 存在，规模仍远小于 egui 的 10 个子模块 |
| 模块存在性 | — | `present.rs / comments.rs / icons_panel.rs / board_bar.rs / board_picker.rs / palette.rs` 均已存在；`plugins.rs / a11y.rs / bidi.rs / services.rs / proto_canvas.rs / live.rs / field_math.rs` 仍不存在 |

### X.1 第 3 批真实剩余（按代码实况）

- **A19 对话框深化**（已存在但需 1:1）：ColorProfile ✅本日；SampledColor ✅本日（框架扩展）；**Rename 批量 ✅本日（Y 节，Match/正则/预览全部 1:1）**；ImportReport（引擎捕获）、MissingFonts（引擎缺命令，保持提示）、Link 的 Offset/移动到页仍待做。
- **A4 插件面板**：Browse 联网拉取（后台任务，未接）。
- **A27 画板协作**：board.timer/board.vote/addConnected/connected_place/BoardTimer 仍 0 命中（board_bar/board_picker 只做了工具组）。
- **A26 原型连线**：`proto_canvas` 模块完全缺失。
- **A2/A3/A5/A16 等**：A5 图标、A3 评论、A2 工具组已部分落地，细节见各节。

### X.2 第 4 批真实剩余（架构级，需单独立项，非一次移植）

| 条目 | 规模（egui） | 现状 | 处置 |
|---|---|---|---|
| B8 Present 子模块 | ~2731 行 / 10 子模块 | `present.rs` ~488 行 | 逐项补全多页/链接/响应式/退出 |
| A21 a11y | 1071 行 | 不存在 | 移植 AccessKit/F6/焦点环/方向键 |
| A22 bidi/ui_scale | ~244 行 | 不存在 | 移植 UAX#9/RTL/pref.uiScale |
| A23 services | — | 不存在 | 原生文件对话框/剪贴板/OPEN_EXTENSIONS/IME |
| A24 field_math | — | 不存在（冻结件，改前须确认） | 数字框相对运算/字体预览/实时提示/图层选择菜单 |
| A16 多窗口 | — | 弹层非 OS 多窗口 | 需重构 `Session` 所有权共享（J 节已记） |
| A14 更新器 | 依赖 craft-update 升版 | 提示窗 | 等 C2 升版 |
| A28 Live | 1672 行 | 完全缺失 | 光标/聊天/spotlight/共享/加入/断连，最大块 |
| B6/B7/B10 | — | 部分 | 变量表列行为/导出、属性面板 fill_rule 反向环、光标逐工具核对 |

### X.3 推进纪律

- 每落地一个条目，更新本 X 节 + G 表对应行 + I 节 `PARITY_CONTRACT_STATUS` 块（后者由测试双向核对，改文档标签须同步 `parity_contract.rs`）。
- 引擎缺命令的条目（MissingFonts）诚实保持提示窗，不假造命令；引擎不发请求的条目（ImportReport）标注为待捕获，不假装已接。

## Y. 2026-11-13 晚：第 3 批 A8 批量重命名 1:1 落地

> 承 X 节「继续第 3 批」。本轮只做 A8，且**严格照 `crates/ui-egui/src/dialogs/batch_rename.rs`
> 逐项抄**，不自造控件、不发明命令。分支 `feat/batch3-dialogs`（承 A15 SampledColor）。

### Y.1 改了什么

| egui 原件 | GPUI 落地 |
|---|---|
| `open_batch_rename`：`cmd::edit::z_sorted(doc, ids)` | `CanvasView::open_dialog` 对 `Kind::Rename` 注入 `ids` / `names`（z 序、真实层名）；菜单 `object.batchRename` 由 `menu_action` 改为开对话框，不再用 `{}` 静默执行 |
| 标题 `Rename layers`、宽 320、`{n} layers` | `Spec::new(tr("Rename layers")).width(320.0)` + `Note(trn("{n} layers", n, &[]))` |
| Match 输入框（hint 即标签） | `Field::Text { key: "find", label: tr("Match (empty: the whole name)") }` |
| `widgets::checkbox` Regular expression | 新增 `Field::Toggle`（冻结件 `Checkbox` 组合，运行时贡献 JSON bool），`Source::Toggle` |
| Rename to 输入框 | `Field::Text { key: "to", label: tr("Rename to") }` |
| 一行三按钮 `$& / $n / $N` 追加到输入框 | 新增 `Field::InsertCodes { target: "to", codes: .. }`；点击读 `TextField::value` 再 `set_value`，`wake()` 让下一帧 flush |
| `DragValue` Start 0..=1_000_000 | `Field::Number { key: "start", value: 1.0 }`，提交时 `clamp(0,1_000_000)` |
| 前 5 行 `old → new` 预览、非法正则红字、`and {n} more` | 新增 `Field::RenamePreview`；每帧用 `collect_sources` 读实时字段，调引擎 `BatchRename::new` / `apply(old, k, n)`；`render` 为此新增 `cx: &App` 参数 |
| `ready` 时才能 Rename | `batch_rename_params` 返回 `None` 即禁用按钮（灰显、不注册 handler）、`resolve_derived` 返回 `false` 保持窗口不关 |
| `batch_rename_params` → `object.batchRename` | `Action::Derived` 新增 `Kind::Rename` 分支；空 Match 写 `name`，否则写 `find` + `replace`；`derived_commands` 按 kind 返回，经契约审计为引擎真命令 |

框架扩展全部落在 `dialogs.rs` 的数据模型里（新增 3 个 `Field` 变体、1 个 `Source` 变体），
`widgets/**` 冻结件一行未改；`lib.rs` 只加一个 `match` 分支 + 把 `cx` 传给 `render`，
用字节精确替换保持 CRLF 与 2640 处双空行不变。

### Y.2 防回归

- `parity_contract.rs` 新增契约 `A8`，requires 含 `object.batchRename` / `batch_rename_params` /
  `BatchRename::new` / `InsertCodes` / `RenamePreview` / 四个字段 key；I 节状态块 `A8=done`。
  退回「单个 Name 字段 + `object.rename`」会同时丢掉这些 token，契约测试直接失败。
- `dialogs.rs` 新增纯函数级回归：空 Match 走 `name`、有 Match 走 `find/replace`、非法正则与空表单
  禁用提交、预览的 `$n`/`$N` 升降序、spec 字段顺序与 `menu_action` 映射。

### Y.3 诚实边界（未做）

- **Enter 提交**：egui 的 `to.lost_focus() && Enter` 提交，GPUI 冻结的 `TextField` 只有 `on_commit`
  语义，本轮未接（按钮提交已可用）；接它需要确认冻结件的焦点事件。
- **select_all_on_focus**：egui 两个输入框聚焦时全选，冻结 `TextField` 无此钩子，未做。
- **实机视觉验证**：像素级比对与双击打字冒烟需跑 GUI，单测不能替代。
- ImportReport（引擎捕获导入 warnings）、Link 的 Offset/移动到页、A4 Browse 联网、
  A27 计时器/投票、A26 原型连线仍是 X.1 的后续批次。
