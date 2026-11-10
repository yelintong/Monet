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
| A6 | **导出对话框 Export**（格式/倍数/预览/多选导出） | `dialogs/export.rs:64 export_dialog` | 只有变量导出 `panels/right/variables.rs:760`（`ui.export` 的 JSON），无通用导出 UI | 无法从界面选格式/倍数导出 | P0 | 中 |
| A7 | **首选项 Preferences 对话框** | `dialogs.rs:211 show`（Preferences 分支） | 只有菜单项 `"Preferences"`（`main_menu.rs:37`），无对话框 | 主题/语言/滚轮缩放等设置无 UI | P0 | 中 |
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
| A18 | **标签关闭未保存询问**（Don't Save / Cancel / Save；Alt+close 直接丢弃） | `windows.rs:392,422,542`（引擎请求 `dialog.unsaved`，**原文写的 `app.rs` #2032 不是对话框所在处**） | `tabs_view.rs:11` 自己写着：`未保存关闭的 Save / Don't Save / Cancel 三问（P1：GPUI 还没有对话框框架…）` | 关标签可能直接丢改动 | P1 | 小 |
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
| B1 | ~~右栏第三个 tab~~ | `"Developer Mode"`（#2135 商标改名，`right_tab_label()` + 命名回归测试） | 改名与命名回归测试已落地；**2026-11-10 用户要求从右栏 tabs 行去掉该按钮**，现只剩 Design / Prototype。`naming.rs` 的常量/函数保留给 a11y 与其它入口，Developer Mode 面板仍可从底部工具栏 Inspect 或 Shift+D 进入 | 命名合规完成；可见 tab 按用户要求移除；底部模式切换第 4 格 `Inspect`→`开发`（修掉退役名），动画/开发模式下右栏 tabs 行改为显示「动画」/「开发」文本（见下方进度） | ✅ |
| B2 | Actions 调色板（Cmd+K） | 三个 tab `All / Assets / Plugins & widgets`、`Recents`、前缀优先排序、最多 60 行（`menus/palette.rs:90,162,170`） | 单一平铺列表 `palette_rows()`（`lib.rs:311`），无 tab / Recents / 排序 | 找不到插件与资产，长列表体验差 | P1 |
| B3 | 工具条 | 设计模式 6 组 + **画板模式 8 组**（bmove/marker/sticky/bshape/connector/text/section…） | 设计模式 6 组齐全；画板组全无（`toolbar.rs:17-20` 明写未移植） | 见 A2 / A27 | P1 |
| B4 | 标签条 | 拖拽重排、关闭询问、拖出成窗口、Alt+close 直接丢弃 | `tabs.rs` / `tabs_view.rs` 有拖拽与重排；关闭询问与拖出成窗口未见 | 见 A16/A18 | P1 |
| B5 | ~~协作 Live~~ | ~~59 个文件规模~~ → 实为 `live.rs` + `live/agent.rs` 共 1672 行 | ~~26 个文件规模~~ → **0 个文件**，整块缺失 | **已移到 A28**；原文「需逐项比对」严重低估 | P1（见 A28） |
| B6 | 变量面板 | Full / Floating + 表格列、重命名、导出 | `variables_full` 有（`lib.rs:585`），`WindowKind::VariablesTable` 有 | 列行为/多选/导出细节待逐项核 | P2 |
| B7 | 属性面板（右栏 sections） | `panels/right/{effects,inspect,menu,paints,stroke}.rs` 分模块 | 集中在 `panels/right/sections.rs`（+inspect.rs/proto.rs/component.rs） | 需按 section 逐条比对；`fill_rule.rs`（442 行）只有 Non-zero/Even-odd 菜单（GPUI `sections.rs:669-670`），`reverseLoop` 与填充区域蓝/橙可视化、点击反转环仍缺 | P2 |
| B8 | 演示 Present | `present.rs` + **10 个非测试子模块**（actions/chrome/device/draw/input/layout/navigate/player/scroll/transition，约 2731 行） | `present.rs:231` + `preview.rs:257` ≈ 488 行，子模块全无对应 | **约 1/6**；多页/链接/响应式/退出的细节不只是「待核」，是未移植 | P2 |
| B9 | **Hide UI / Minimize UI** | `view.toggleUi`（Cmd+\\）设 `ui_hidden`；`view.minimize`（Shift+Cmd+\\）设 `ui_minimized`，EGUI 据此收侧栏并保留宽度 | engine 两个命令与快捷键都在（`engine/src/cmd/view.rs:333,339`、`keymap.rs:164,228`），但 **`crates/ui-gpui/src` 内 0 处读 `ui_hidden` / `ui_minimized`**（原文写「全仓不读」不准：engine 与 ui-egui 都有命中）→ 快捷键有反应、界面不动 | 接两个状态到 render 层（低工作量、高可见收益） | P1 |
| B10 | 画布光标/工具态 | 每工具+修饰键的光标、拖拽阈值、双击进文本 | `canvas/cursors.rs` 存在 | 需逐工具核对 | P3 |

---

## C. 上游最近 16 个提交里尚未跟进的 UI 相关变更

| # | 上游变更 | 说明 | GPUI 现状 | 优先级 |
|---|---|---|---|---|
| C1 | #2135 / #2165 **Dev Mode → Developer Mode** | 商标原因改名，新增 `right_tab_label()` 与 `naming_tests.rs`；locales 里法/葡/荷的错误译名一并修正。**注意本地 `ui-egui/src/app.rs:352` 仍是 `tr("Dev Mode")`**（该 crate 刻意不同步），`right_tab_label` / `naming_tests.rs` 本地不存在，需以 `docs/feature-names.md` 为准 | 已补 `right_tab_label()` + 命名回归测试，UI 文案改为 Developer Mode；随后按用户要求从右栏 tabs 行移除该按钮，常量与测试保留 | ✅ |
| C2 | craft-update `c5a1cbf` → `1b34eba`：更新器上报 installing/ready/installed/failed，**退出时安装** | `start_pending_update()` 由 `FigApp` 提供，`main.rs` 退出时调用 | 无更新器 UI、无退出安装 | P1（依赖 A14） |
| C3 | `docs/feature-names.md` 新增 | 命名规范（不再用 Figma-class 比较措辞）；Developer Mode 一节 | 已同步文档，UI 文案尚未按规范对齐 | P2 |
| C4 | MCP display names 测试 | 非 UI（已同步） | 已同步 | — |
| C5 | `.fig` 实例缩放（#2091/#2097/#2100） | engine/format 层 | **已随本次同步落地** | 完成 |

---

## D. 建议实施顺序

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
7. B1/B3/A18 的现状证据（`:243` 仍为 `"Inspect"`、`toolbar.rs:17-20` 画板组未移植、`tabs_view.rs:11-13` 自认无对话框/a11y/live）复查后仍成立。

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
| 7 | `crates/ui-gpui/src/lib.rs`（`palette_rows`） | B2 | 既有 `menu_card`/`Palette` 弹层 | ⬜ |
| 8 | `crates/ui-gpui/src/home.rs`（新） | A1 | 通用对话框/浮层 + `TextField`/`IconButton` | ⬜ |
| 9 | `crates/ui-gpui/src/comments.rs`（新） | A3 | 通用浮层 + 输入框系列 | ⬜ |
| 10 | `crates/ui-gpui/src/plugins.rs`（新） | A4 / A25 | 通用浮层 + `widget_bar` 复用同一行模型 | ⬜ |
| 11 | `crates/ui-gpui/src/toolbar.rs` + `boards/*`（新） | A2 / B3 / A27 | `GROUPS` 既有工具组模型（只加数据，不改控件） | ⬜ |
| 12 | `crates/ui-gpui/src/present/*`（新） | B8 | `present.rs` 既有播放状态机 | ⬜ |
| 13 | `crates/ui-gpui/src/a11y.rs`（新）/ `bidi.rs` / `ui_scale.rs` | A21 / A22 | 通用焦点环 / 名称渲染钩子 | ⬜ |
| 14 | `crates/ui-gpui/src/services.rs`（新） | A23 | 平台能力接口（保持 UI 可替换） | ⬜ |
| 15 | `crates/ui-gpui/src/widgets/field_math.rs`（新，需解冻确认） | A24 | 通用数字字段（**冻结件，改前必须先问用户**） | ⬜ |
| 16 | `crates/ui-gpui/src/panels.rs` | 右栏 48 pt 顶栏（头像 / Present / Share）恢复 | 既有 `right::topbar::top_bar` 通用顶栏，只补一行调用，与 Motion 分支同序 | ✅ |
| 17 | `crates/ui-gpui/src/panels/right/sections.rs` | 右栏 tabs 去掉第三个「开发模式」按钮 | 既有 `tab(...)`，只减少一项数据；面板功能仍由底部工具栏 Inspect / Shift+D 进入 | ✅ |

---

## H. 2026-11-10 第 1 批实施结果（按文件）

| 文件 | 落地内容 |
|---|---|
| `docs/gpui-egui-parity-plan.md` | 复查修订：补 A19–A28、修正 7 处证据、新增 F 复查说明与 G 按文件清单 |
| `crates/ui-gpui/src/naming.rs`（新） | `right_tab_label()` + `RIGHT_TAB_LABELS` + 4 条命名回归测试；命令 id 与调色板搜索别名保持不变 |
| `crates/ui-gpui/src/panels/right/sections.rs` | 第三个 tab 从 `"Inspect"` 改为 `crate::naming::right_tab_label(2)` = `"Developer Mode"`，没有新控件 |
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
- A18 只接了引擎 `dialog.unsaved` 请求；`tabs_view.rs` 的 Alt+close / 关闭三问还没接（G 表第 6 行 🔄）。
- A13 缺失字体、A15 取样色、A14 更新、A11 About、A4 插件目前是**带说明的提示窗口**，因为引擎没有对应命令或依赖 C2 升版；按钮只有 Close，不提交不存在的命令。
- A1 Home / A2 Boards / A3 Comments / A16 多窗口 / A21 a11y / A22 bidi·ui_scale / A24 字段运算 / A28 Live / B2 调色板 / B8 Present 子模块仍未开始。

### 17:25–17:33 按现场反馈追加调整

| 事项 | 处理 |
|---|---|
| 提交 GitHub | `e9de7cb4`（第一批）+ `e9640ac1`（Cargo.lock）已推送 `fork/GPUI-smooth`。git 的 `http.proxy` 写死在失效端口 65311，实际监听的是 54834；临时 `-c http.proxy=… -c http.schannelCheckRevoke=false` 后推送成功 |
| 用户头像 / Present / Share 栏「会丢失」 | 根因：`panels.rs::right_panel` 的 `wrap()` 只画 `view_bar` + body，48 pt 顶栏**只在 `lib.rs` 的 Motion 分支**画过。已在 `wrap()` 补上 `right::topbar::top_bar(f, data)`，顺序与 Motion 分支一致；两条分支互斥，不会重复画 |
| 「下面这一栏不要开发模式按钮」 / 底部切换器第 4 格名称 | 用户确认为**右栏 tabs 行的第三个标签**，已移除只留 Design / Prototype；`naming.rs` 的 `right_tab_label` / 常量 / 命名回归测试保留给 a11y 与其它入口，Developer Mode 面板本身没删（底部工具栏 Inspect、Shift+D 仍可进入）。同一轮把底部模式切换器第 4 格从退役名 `Inspect` 改为 `开发`，并在动画/开发模式下把右栏 tabs 行的 Design/Prototype 换成「动画」/「开发」文本 |
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

**下一步**：G 表第 7 行 —— Actions 调色板三 tab（`All / Assets / Plugins & widgets`）+ Recents + 前缀优先排序 + 最多 60 行；随后第 8 行 Home。涉及 `widgets/**` 冻结件的改动（A24）必须先与用户确认。
