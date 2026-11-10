# GPUI 前端 ↔ 最新 EGUI 的界面还原差距与计划表

> 对照基准：`origin/main` = `a2e87433`（2026-11-09，storytold/figcraft）。
> 本地：`GPUI-smooth` = `1fd20805`（已完成非 UI 同步）。
> 扫描范围：`crates/ui-gpui/src`（81 个非测试文件）对 `crates/ui-egui/src`（130 个非测试文件）。

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
| A2 | **画板模式 Boards**（笔/荧光笔/橡皮/便签/画板形状/连接线 + 画板画布） | `panels/boards/*`、`canvas/board_bar.rs`、`engine/src/boards` | 工具条无 `Marker/Highlighter/Eraser/Sticky/BoardShape/Connector` 任何 id（`toolbar.rs:60-82` 只有设计模式 6 组） | 整块白板功能不可用 | P1 | 大 |
| A3 | **评论 Comments**（评论工具之外的评论列表/气泡/回复） | `comments_*`、`canvas/comments` | 只有工具按钮 `ToolId::Comment`（`toolbar.rs:80`），无评论面板/线程 UI | 能点工具，无处查看/回复评论 | P1 | 中 |
| A4 | **插件面板 Plugins**（浏览、安装、启用） | `dialogs/plugins_ui.rs`、`plugin_panel` | 只有主菜单 `"Plugins"` 字样（`main_menu.rs:37`），无浏览/管理面板 | 插件生态入口缺失 | P1 | 中 |
| A5 | **图标面板 Icons**（图标库插入） | `icons_panel.rs` | 0 处引用 | 图标字体插入路径缺失 | P2 | 中 |
| A6 | **导出对话框 Export**（格式/倍数/预览/多选导出） | `dialogs/export.rs:64 export_dialog` | 只有变量导出 `panels/right/variables.rs:760`（`ui.export` 的 JSON），无通用导出 UI | 无法从界面选格式/倍数导出 | P0 | 中 |
| A7 | **首选项 Preferences 对话框** | `app/frame.rs` | 只有菜单项 `"Preferences"`（`main_menu.rs:37`），无对话框 | 主题/语言/滚轮缩放等设置无 UI | P0 | 中 |
| A8 | **批量重命名 Batch rename** | `dialogs/batch_rename.rs` | 搜 `"Rename layers"` 0 命中 | 多选改名只能逐个 | P2 | 小 |
| A9 | **分支 UI Branch** | `dialogs/branch_ui.rs` | 搜 `branch` 只命中控制通道/文本编辑，无 UI | 分支管理不可用 | P2 | 中 |
| A10 | **色彩配置 Color profile** | `dialogs/color_profile.rs` | 仅 `panels/right/sections.rs:1823` 注释提到 Page 设置 | P3 色域/配置文件不可设 | P2 | 小 |
| A11 | **Credits / 贡献者** | `dialogs/credits.rs` | 0 处引用 | 关于页缺失 | P3 | 小 |
| A12 | **导入报告 Import report** | `dialogs/import_report.rs` | 0 处引用 | 导入结果/警告不可见 | P2 | 小 |
| A13 | **缺失字体提示 Missing fonts** | `dialogs/missing_fonts.rs` | 0 处引用 | 打开缺字体文件无提示/替换入口 | P2 | 小 |
| A14 | **更新对话框 Updates** | `dialogs/updates_ui.rs` + craft-update | 0 处引用 | 无法检查/安装更新 | P1 | 中 |
| A15 | **取色器 Eyedropper / Sampled color** | `dialogs/sampled_color.rs` | 只有光标名命中（`canvas/cursors.rs`），无取样 UI | 屏幕取色不可用 | P2 | 小 |
| A16 | **多窗口**（File ▸ New window / Open in new window / 标签拖出成窗口） | `windows.rs` | `open_window` 是弹层 `popovers::WindowState`，非 OS 多窗口 | 只能单窗口 | P2 | 大 |
| A17 | **菜单项取值对话框**（Create link / Offset path / Move to page 等） | `dialogs/menu_input.rs` | 无对应实现 | 这些命令没有输入入口 | P2 | 小 |
| A18 | **标签关闭未保存询问**（Don't Save / Cancel / Save；Alt+close 直接丢弃） | `app.rs` #2032 | `tabs_view.rs:11` 自己写着：`未保存关闭的 Save / Don't Save / Cancel 三问（P1：GPUI 还没有对话框框架…）` | 关标签可能直接丢改动 | P1 | 小 |

---

## B. 已还原但不到位（有 UI，功能/文案不齐）

| # | 位置 | EGUI 做法 | GPUI 现状（证据） | 差距 | 优先级 |
|---|---|---|---|---|---|
| B1 | 右栏第三个 tab | `"Developer Mode"`（#2135 商标改名，`right_tab_label()` + `naming_tests`） | 仍叫 `"Inspect"`（`panels/right/sections.rs:243`），全仓无 `"Developer Mode"` | **命名不合规**（DEV MODE 是 Figma 注册商标），且缺 a11y 读取的 `right_tab_label` | **P0** |
| B2 | Actions 调色板（Cmd+K） | 三个 tab `All / Assets / Plugins & widgets`、`Recents`、前缀优先排序、最多 60 行（`menus/palette.rs:90,162,170`） | 单一平铺列表 `palette_rows()`（`lib.rs:311`），无 tab / Recents / 排序 | 找不到插件与资产，长列表体验差 | P1 |
| B3 | 工具条 | 设计模式 6 组 + **画板模式 8 组**（bmove/marker/sticky/bshape/connector/text/section…） | 设计模式 6 组齐全；画板组全无（`toolbar.rs`） | 见 A2 | P1 |
| B4 | 标签条 | 拖拽重排、关闭询问、拖出成窗口、Alt+close 直接丢弃 | `tabs.rs` / `tabs_view.rs` 有拖拽与重排；关闭询问与拖出成窗口未见 | 见 A16/A18 | P1 |
| B5 | 协作 Live | 59 个文件规模（光标/聊天/spotlight/共享/加入） | 26 个文件规模 | 需逐项比对（光标聊天、spotlight、加入流程） | P2 |
| B6 | 变量面板 | Full / Floating + 表格列、重命名、导出 | `variables_full` 有（`lib.rs:585`），`WindowKind::VariablesTable` 有 | 列行为/多选/导出细节待逐项核 | P2 |
| B7 | 属性面板（右栏 sections） | `panels/right/{effects,inspect,menu,paints,stroke}.rs` 分模块 | 集中在 `panels/right/sections.rs`（+inspect.rs/proto.rs/component.rs） | 需按 section 逐条比对（Effects/Stroke/Paints 的字段与交互） | P2 |
| B8 | 演示 Present | `present/` 完整（多页、链接、响应式） | `present.rs` 存在 | 多页/链接/退出细节待核 | P2 |
| B9 | **Hide UI / Minimize UI** | `view.toggleUi`（Cmd+\\）设 `ui_hidden`；`view.minimize`（Shift+Cmd+\\）设 `ui_minimized`，EGUI 据此收侧栏并保留宽度 | engine 两个命令与快捷键都在（`engine/src/cmd/view.rs:333,335`、`keymap.rs:164,228`），但 **GPUI 全仓不读 `ui_hidden` / `ui_minimized`** → 快捷键有反应、界面不动 | 接两个状态到 render 层（低工作量、高可见收益） | P1 |
| B10 | 画布光标/工具态 | 每工具+修饰键的光标、拖拽阈值、双击进文本 | `canvas/cursors.rs` 存在 | 需逐工具核对 | P3 |

---

## C. 上游最近 16 个提交里尚未跟进的 UI 相关变更

| # | 上游变更 | 说明 | GPUI 现状 | 优先级 |
|---|---|---|---|---|
| C1 | #2135 / #2165 **Dev Mode → Developer Mode** | 商标原因改名，新增 `right_tab_label()` 与 `naming_tests.rs`；locales 里法/葡/荷的错误译名一并修正 | 仍用 `"Inspect"`，无命名测试 | **P0** |
| C2 | craft-update `c5a1cbf` → `1b34eba`：更新器上报 installing/ready/installed/failed，**退出时安装** | `start_pending_update()` 由 `FigApp` 提供，`main.rs` 退出时调用 | 无更新器 UI、无退出安装 | P1（依赖 A14） |
| C3 | `docs/feature-names.md` 新增 | 命名规范（不再用 Figma-class 比较措辞） | 已同步文档，UI 文案尚未按规范对齐 | P2 |
| C4 | MCP display names 测试 | 非 UI（已同步） | 已同步 | — |
| C5 | `.fig` 实例缩放（#2091/#2097/#2100） | engine/format 层 | **已随本次同步落地** | 完成 |

---

## D. 建议实施顺序

### 第 1 批（合规 + 高频入口，1–2 天）
1. **B1 / C1**：右栏第三 tab 改名 `Developer Mode`，补 `right_tab_label()` 与命名回归测试，清掉全仓 `"Dev Mode"` 残留。
2. **B9**：把 `ui_hidden` / `ui_minimized` 接到 render 层（两个快捷键已经绑好，只差界面响应）。
3. **A6**：导出对话框（格式 / 倍数 / 预览），复用 `WindowKind` 壳。
4. **A7**：首选项对话框（主题 / 语言 / 滚轮缩放 / 面板宽度重置）。

### 第 2 批（主导航与内容完整性，3–5 天）
4. **A1**：首页 Home（Recents 卡片 + Templates + Getting started）。
5. **A18 / B4**：标签关闭询问（Don't Save / Cancel / Save）+ Alt+close。
6. **B2**：调色板三 tab + Recents + 前缀排序。
7. **A3**：评论面板（列表 / 气泡 / 回复）。
8. **A4**：插件面板（浏览 / 安装 / 启用）。

### 第 3 批（专业功能，5–8 天）
9. **A2 / B3**：画板模式整套（工具组 + 画板画布 + 顶栏）。
10. **A5**：图标面板。
11. **A8 / A12 / A13**：批量重命名、导入报告、缺失字体提示。
12. **A15**：取色器 / 取样色。

### 第 4 批（长尾，按需）
13. **A9 / A10 / A11 / A17**：分支 UI、色彩配置、Credits、菜单取值对话框。
14. **A14 / C2**：更新器 UI 与退出安装（需先升 craft-update，会牵动 ui-egui 编译）。
15. **A16**：多窗口。
16. **B5–B10**：Live / 变量 / 属性面板 / Present / 系统级 UI 的逐项精修。

---

## E. 风险与前置条件

| 事项 | 说明 |
|---|---|
| `ui-egui` 本地破损 | 62 个错误（重复字段/重复函数）挡着 `cargo check --workspace`。若要跟 C1/C2 的 EGUI 侧改动，需先修这份重复块（删除后出现的那一份 `picker_popover`/`style_picker_popover` 与 `toggle_picker`） |
| craft-update 升版 | 升到 `1b34eba` 会让 `ui-egui/updates_ui.rs` 用旧 API 编译失败，必须同时接受 EGUI 侧的 API 适配（`Status`、`manual_window_open` 的 cfg 收窄、`result_window_shows`） |
| 根 `Cargo.toml` / `Cargo.lock` | 本地独有依赖（`write-fonts`/`usvg`）与上游不同，**不能整体覆盖**，每次同步都要手工合 |
| 命名合规 | `"Dev Mode"` 属 Figma 商标，UI 文案、i18n 目录、帮助文本都要改；命令 id `view.devMode` 保持不动 |
