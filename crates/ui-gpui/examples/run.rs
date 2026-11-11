//! 启动 GPUI 前端（与 `cargo run -p figcraft -- --control <port>` 同义的 GPUI 版本）。
//!
//! 边框调试：设置 `FC_DEBUG_LAYOUT=1` 后，所有 `dbg_frame` 描边生效（分节头、分隔线、标题等），
//! 方便核对面板布局。
//!
//! ```text
//! FC_DEBUG_LAYOUT=1 cargo run -p figcraft-ui-gpui --example run
//! # 带本地控制通道（截图 / 注入输入）：
//! FC_DEBUG_LAYOUT=1 cargo run -p figcraft-ui-gpui --example run -- --control 7811
//! ```

fn main() {
    figcraft_ui_gpui::run();
}
