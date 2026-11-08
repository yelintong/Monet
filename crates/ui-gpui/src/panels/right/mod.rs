//! 右栏各节的真实实现集合（迁移自 `ui-egui::panels::right`）。
//!
//! 迁移规则（用户 2026-11-05 要求）：照搬 `ui-egui::panels::right` 的各节到 GPUI 右栏；
//! 用到通用组件（TextField / ColorField / Dropdown / IconButton / PanelSection 等）的节直接实现。
//! egui 源码逐字抄至 `reference/<name>.rs.txt`，供对照；各节真实行控件在 `sections.rs` 内。

/// 右栏列表数据的快照（selection → panel）。
pub(crate) mod data;
/// 3×3 对齐选择器（照搬 `layout_panel.rs::picker`，GPUI 自绘）。
pub(crate) mod picker;
/// Component / Instance 面板（egui `component_panel.rs`）。
pub(crate) mod component;
/// Inspect 标签（Dev Mode：属性分组 + 生成的代码）。
pub(crate) mod inspect;
/// Inspect 标签里的 Dev Mode 其余各节（egui `dev_mode.rs`：注释 / 开发资源 / 测量 / 就绪列表 / 分类）。
pub(crate) mod devmode;
/// Prototype 标签（egui `proto_panel.rs`：flows / settings / interactions / scroll）。
pub(crate) mod proto;
/// 按 egui 源码实现的各节真实行控件。
pub(crate) mod sections;
/// 变量编辑器「右边的列表」（egui `dialogs/variables_panel.rs::table()`）。
pub(crate) mod variables;
/// 列表虚拟化组件（适配 gpui 0.2.2；
/// 不走 `widgets/`，因为该模块已冻结）。
pub(crate) mod virtual_list;
