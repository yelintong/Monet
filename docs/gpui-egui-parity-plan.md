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
| A1 | **首页 Home**（Recents / Online / Templates / Getting started） | `home.rs` + `home/` | `panels/extra.rs:1243` 测试直接写 `"GPUI 没有 Home，不能假装接了"`，`ui_action("home")` 返回 `None` | 启动即落在画布，无最近文件/模板入口 | P1 | 大 |
| A2 | **画板模式 Boards**（笔/荧光笔/橡皮/便签/画板形状/连接线 + 画板画布） | `canvas/board_bar.rs`、`canvas/board_facilitate.rs`、`canvas/board_connect.rs`、`widgets/board_picker.rs`、`engine/src/boards`（**原文写的 `panels/boards/*` 目录不存在**） | 工具条无 `Marker/Highlighter/Eraser/Sticky/BoardShape/Connector` 任何 id（`toolbar.rs:60-82` 只有设计模式 6 组）；`board.timer` / `board.vote` / `addConnected` / `facilitate` / `BoardTimer` 全部 0 命中 | 整块白板功能不可用（计时器、投票、自动连线、模板选择器也一起缺） | P1 | 大 |
| A3 | **评论 Comments**（评论工具之外的评论列表/气泡/回复） | `comments_*`、`canvas/comments.rs` | 只有工具按钮 `ToolId::Comment`（`toolbar.rs:80`），无评论面板/线程 UI | 能点工具，无处查看/回复评论 | P1 | 中 |
| A4 | **插件面板 Plugins**（浏览、安装、启用） | `dialogs/plugins_ui.rs`、`plugin_panel` | 只有主菜单 `"Plugins"` 字样（`main_menu.rs:37`），无浏览/管理面板 | 插件生态入口缺失 | P1 | 中 |
| A5 | **图标面板 Icons**（图标库插入） | `icons_panel.rs` | 0 处引用 | 图标字体插入路径缺失 | P2 | 中 |
| A6 | **导出对话框 Export**（格式/倍数/预览/多选导出） | `dialogs/export.rs:64 export_dialog` | 🔄 已通过 A19 通用壳接上：`dialogs.rs:239-251` 的 `Kind::Export` 有 Format(5)/Scale(4)/File name + `file.export`，菜单 `catalog.rs:48` 与 `menu_action()` 可达 | **预览、多选导出、逐格式参数、原生路径选择仍缺**（见 I 节） | P0 | 中 |
| A7 | **首选项 Preferences 对话框** | `dialogs.rs:211 show`（Preferences 分支） | 🔄 已通过 A19 通用壳接上：`dialogs.rs:253-267` 的 `Kind::Preferences` 有 Theme / Interface size / 左右面板宽度 + Apply→`view.panelWidths`；入口 `catalog.rs:267` 与 Cmd+, 可用 | **语言、滚轮缩放、面板宽度重置仍缺**（见 I 节） | P0 | 中 |
| A8 | **批量重命名 Batch rename** | `dialogs/batch_rename.rs` | 搜 `"Rename layers"` 0 命中 | 多选改名只能逐个 | P2 | 小 |
| A9 | **分支 UI Branch** | `dialogs/branch_ui.rs` | 搜 `branch` 只命中控制通道/文本编辑，无 UI | 分支管理不可用 | P2 | 中 |
| A10 | **色彩配置 Color profile** | `dialogs/color_profile.rs` | 仅 `panels/right/sections.rs:1823` 注释提到 Page 设置 | 色域 / 配置文件不可设 | P2 | 小 |
| A11 | **Credits / 贡献者** | `dialogs/credits.rs` | 0 处引用 | 关于页缺失 | P3 | 小 |
| A12 | **导入报告 Import report** | `dialogs/import_report.rs` | 0 处引用 | 导入结果/警告不可见 | P2 | 小 |
| A13 | **缺失字体提示 Missing fonts** | `dialogs/missing_fonts.rs` | 0 处引用 | 打开缺字体文件无提示/替换入口 | P2 | 小 |
| A14 | **更新对话框 Updates** | `dialogs/updates_ui.rs` + craft-update | 0 处引用 | 无法检查/安装更新 | P1 | 中 |
| A15 | **取色器 Eyedropper / Sampled color** | `dialogs/sampled_color.rs` | 只有光标名命中（`canvas/cursors.rs`），无取样 UI | 屏幕取色不可用 | P2 | 小 |
| A16 | **多窗口**（File ▸ New window / Open in new window / 标签拖出成窗口） | `windows.rs` | `open_window` 是弹层 `popovers::WindowState`，非 OS 多窗口 | 只能单窗口 | P2 | 大 |
| A17 | **菜单项取值对话框**（Create link / Offset path / Move to page 等） | `dialogs/menu_input.rs` | 无对应实现 | 这些命令没有输入入口 | P2 | 小 |
| A18 | **标签关闭未保存询问**（Don't Save / Cancel / Save；Alt+close 直接丢弃） | `windows.rs:392,422,542`（引擎请求 `dialog.unsaved`，**原文写的 `app.rs` #2032 不是对话框所在处**） | ✅ 已落地：`tabs_view.rs:388,392` 的 `force: e.modifiers.alt`、`lib.rs:1400` 消费 `dialog.unsaved`、`dialogs.rs:285-295` 三问 | Save 后不会自动再关闭；拖出成窗口（A16）仍缺（见 I 节） | P1 | 小 |
| **A19** | **通用对话框框架**（数据驱动：标题 / 说明 / 字段 / 按钮 / 确认取消 / Escape / 遮罩） | `dialogs.rs`（579 行，`renderers()` 统一 About·Preferences·Shortcuts·Find·Rename·Variables·Export） | `tabs_view.rs:11` 自认「还没有对话框框架」；`control.rs:356` 只有 `"dialogs": null` | **A6/A7/A8/A10/A11/A12/A13/A15/A17/A18 十个对话框的共同前置**；不做就只能每个各造一套壳 | **P0** | 中 |
| **A20** | **查找替换 Find and replace** | `dialogs.rs:368-404`（`edit.findReplace`） | `edit.findReplace` / `find_replace` / `Find and replace` 全 0 命中 | 文档内文本无法批量查找替换 | P1 | 小 |
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
| B4 | 标签条 | 拖拽重排、关闭询问、拖出成窗口、Alt+close 直接丢弃 | 拖拽重排（`tabs_view.rs:155-187`）与关闭询问均已接（G 表第 6 行）；**拖出成窗口（A16）仍缺** | 见 A16/A18 与 I 节 | 🔄 |
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
| C1 | #2135 / #2165 **Dev Mode → Developer Mode** | 商标原因改名，新增 `right_tab_label()` 与 `naming_tests.rs`；locales 里法/葡/荷的错误译名一并修正。**注意本地 `ui-egui/src/app.rs:352` 仍是 `tr("Dev Mode")`**（该 crate 刻意不同步），`right_tab_label` / `naming_tests.rs` 本地不存在，需以 `docs/feature-names.md` 为准 | GPUI 侧 ✅：`right_tab_label()` + 4 条命名回归、标签 `"开发"`、右栏第三 tab 按用户要求移除、底部切换器第 4 格 `Inspect`→`开发`。⚠️ **全仓清理未彻底**：`ui-egui/src/app.rs:352` 仍是 `tr("Dev Mode")` 的真实 UI 文案（该 crate 刻意不同步且本地破损），见 I 节 | ✅（GPUI）/ ⚠️（ui-egui 残留） |
| C2 | craft-update `c5a1cbf` → `1b34eba`：更新器上报 installing/ready/installed/failed，**退出时安装** | `start_pending_update()` 由 `FigApp` 提供，`main.rs` 退出时调用 | 无更新器 UI、无退出安装 | P1（依赖 A14） |
| C3 | `docs/feature-names.md` 新增 | 命名规范（不再用 Figma-class 比较措辞）；Developer Mode 一节 | 已同步文档，UI 文案尚未按规范对齐 | P2 |
| C4 | MCP display names 测试 | 非 UI（已同步） | 已同步 | — |
| C5 | `.fig` 实例缩放（#2091/#2097/#2100） | engine/format 层 | **已随本次同步落地** | 完成 |

---

## D. 建议实施顺序

> **2026-11-10 19:0x 逐项核查结果见 I 节**：下面写的是「建议」，不等于已落地 —— A6/A7 目前只是 A19 通用壳版本，A1 仍未开始。

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
| 8 | `crates/ui-gpui/src/home.rs`（新） | A1 | 通用对话框/浮层 + `TextField`/`IconButton` | ⬜ |
| 9 | `crates/ui-gpui/src/comments.rs`（新） | A3 | 通用浮层 + 输入框系列 | ⬜ |
| 10 | `crates/ui-gpui/src/plugins.rs`（新） | A4 / A25 | 通用浮层 + `widget_bar` 复用同一行模型 | ⬜ |
| 11 | `crates/ui-gpui/src/toolbar.rs` + `boards/*`（新） | A2 / B3 / A27 | `GROUPS` 既有工具组模型（只加数据，不改控件） | ⬜ |
| 12 | `crates/ui-gpui/src/present/*`（新） | B8 | `present.rs` 既有播放状态机 | ⬜ |
| 13 | `crates/ui-gpui/src/a11y.rs`（新）/ `bidi.rs` / `ui_scale.rs` | A21 / A22 | 通用焦点环 / 名称渲染钩子 | ⬜ |
| 14 | `crates/ui-gpui/src/services.rs`（新） | A23 | 平台能力接口（保持 UI 可替换） | ⬜ |
| 15 | `crates/ui-gpui/src/widgets/field_math.rs`（新，需解冻确认） | A24 | 通用数字字段（**冻结件，改前必须先问用户**） | ⬜ |
| 16 | `crates/ui-gpui/src/panels.rs` | 右栏 48 pt 顶栏（头像 / Present / Share）恢复 | 既有 `right::topbar::top_bar` 通用顶栏，只补一行调用，与 Motion 分支同序 | ✅ |
| 17 | `crates/ui-gpui/src/panels/right/sections.rs` | 右栏 tabs 去掉第三个「开发模式」按钮 | 既有 `tab(...)`，只减少一项数据；面板功能仍由底部工具栏 开发 / Shift+D 进入 | ✅ |

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
| **A19** 通用对话框框架 | 十个对话框共用一套壳 + 一套控件 + 一套 Escape/遮罩，数据驱动 | ✅ **完成** | `dialogs.rs` 687 行：`Kind` **15 种**、`Field::{Text,Number,Choice,Note}`、`Spec`/`Button`/`DialogState`、`ui_action()`/`menu_action()`、`render()`(452)、`popup_backdrop`(543)；壳用 `widgets::popover::Popover`、行用 `field_row`/`field_row_full`、按钮统一推 `CmdQueue`。`lib.rs`：状态(608)、`sync_ui_requests` 消费全部 `dialog.*`(1400)、带「…」菜单拦截(3317)、Escape(5289)/X/遮罩统一、render 挂层(6632)。**5 条单测**，**未改任何 `widgets/**` 冻结件** | A13/A15/A14/A11/A4 因引擎无对应命令，仍是「带说明的提示窗口」（按钮只有 Close），不等于 EGUI 的完整交互 |
| **B1 / C1** 命名合规 | 右栏第三 tab 改名、补 `right_tab_label()` 与命名回归测试、清掉全仓 UI 文案 `"Dev Mode"` 残留 | ✅ **GPUI 侧完成** / ⚠️ **全仓清理差一处** | `naming.rs`：`DEVELOPER_MODE = "开发"`、`RIGHT_TAB_LABELS`、`right_tab_label()` + **4 条**命名回归；右栏 tabs 按用户要求**移除**第三按钮（只剩 Design / Prototype），底部模式切换第 4 格 `Inspect`→`开发`，动画/开发模式 tabs 行显示「动画」/「开发」文本。`ui-gpui` 内 `"Dev Mode"` 只出现在注释、`RETIRED` 测试表与 `catalog::SEARCH_ALIASES` 搜索别名（标识符/测试，不是显示文案） | ⚠️ `crates/ui-egui/src/app.rs:352` 仍是 `i18n::tr("Dev Mode")` 的**真实 UI 文案**（该 crate 刻意不同步、本地 62 个既有编译错误）；`crates/i18n/src/tests.rs:19` 只是注释提及。C1 的「清掉全仓 UI 文案残留」严格说**没做完**，是否动 ui-egui 需先确认它解冻 |
| **B9** Hide UI / Minimize UI | 把 `ui_hidden` / `ui_minimized` 接到 render 层 | ✅ **完成** | `lib.rs::ui_flags()`(2231-2233) 读 `ds().view.ui_hidden/ui_minimized`；`dock()`(2239) 给有效宽度且**不改记住的 `left_w`/`right_w`**；render 条件渲染侧栏 / 标签条 / 工具栏 / 时间轴 / 分隔条命中区(4219-4222、5847、6674、6682) | 无 |
| **A6** 导出对话框 | 格式 / 倍数 / **预览**，复用 A19 壳 | 🔄 **部分完成** | `dialogs.rs:239-251` `Kind::Export`：Format(PNG/JPG/SVG/PDF/FIG，带 JSON values)、Scale(1x/2x/3x/0.5x)、File name 文本、Export→`file.export`；入口 `catalog.rs:48` `"Export…"` + `menu_action()`(229-230) 拦截，不会静默用默认参数导出 | **预览缺失**；多选导出、逐格式参数、原生路径选择仍缺，EGUI `dialogs/export.rs` 的完整交互未移植 |
| **A7** 首选项对话框 | 主题 / **语言** / **滚轮缩放** / **面板宽度重置**，复用 A19 壳 | 🔄 **部分完成** | `dialogs.rs:253-267` `Kind::Preferences`：Theme(3 项→`view.themeDark/Light/System`)、Interface size(3 项→`pref.uiScaleReset/Up/Down`)、Left/Right panel width 数值、Apply→`view.panelWidths`；入口 `catalog.rs:267` `"Preferences…"→app.preferences`，engine `cmd/view.rs:508` 发 `dialog.preferences`，`ui_action` 接住，Cmd+, 可用 | **语言选择、滚轮缩放开关缺**；面板宽度是「设值」不是「重置」，没有 Reset 按钮/命令 |
| **A18 / B4** 标签关闭三问 | Don't Save / Cancel / Save + Alt+close + 拖拽重排 + 拖出成窗口 | ✅ **A18 完成** / 🔄 **B4 部分** | `tabs_view.rs:388,392` 关闭改为 `force: e.modifiers.alt`（Alt+close 才直接丢弃）；`lib.rs:1400-1420` 消费 `dialog.unsaved`，弹窗前先 `doc.switch {index}` 保证 Save 存对文档；`dialogs.rs:285-295` 三问 spec；拖拽重排 `tabs_view.rs:155-187`（`file.moveTab`）已有 | **拖出成窗口（A16）仍缺**；Save 只保存、不会自动再执行一次关闭（保存失败也不丢改动），保存后需再点 X；EGUI 的「保存后自动关闭 / Save as」流程未移植 |
| **A1** 首页 Home（第 2 批第 1 项） | Recents 卡片 + Templates + Getting started | ⬜ **未开始** | 无 `crates/ui-gpui/src/home.rs`；`panels/extra.rs:1243` 测试仍断言 `ui_action("home", &empty, 0) == None`，注释原文「GPUI 没有 Home，不能假装接了」 | 整块未做，是 G 表第 8 行的目标 |

**结论**：第 1 批 6 项里 **3 项完成**（A19、B1/C1 的 GPUI 侧、B9），**3 项部分完成**（A6 缺预览、A7 缺语言/滚轮缩放/重置、A18/B4 中 A18 已完成但拖出成窗口仍缺）；C1 的「全仓 UI 文案清理」另差 `ui-egui` 一处。第 2 批 A1 **未开始**。这些缺口已回填到 A/B/C 表与 H 节，避免计划表继续显示旧证据。

**下一步（19:0x 核查后修订）**：先补第 1 批三处遗留缺口（A6 预览、A7 语言 / 滚轮缩放 / 面板宽度重置、B4 拖出成窗口属 A16），再进 G 表第 8 行 A1 首页 Home（Recents 卡片 / Templates / Getting started）；随后第 9–11 行 Comments / Plugins / Boards。`ui-egui` 的 `tr("Dev Mode")` 残留是否清理需先确认该 crate 是否解冻；涉及 `widgets/**` 冻结件的改动（A24 字段运算、调色板实时过滤）必须先与用户确认。
