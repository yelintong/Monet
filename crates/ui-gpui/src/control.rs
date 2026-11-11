//! 控制通道的 UI 线程侧（照搬 `crates/ui-egui/src/control.rs`）。
//!
//! 原文头部：
//!
//! > The control channel's UI-thread side. The app crate accepts JSON lines on a localhost socket
//! > and queues [`ControlRequest`]s. [`background`] runs the phase-safe part from every logic
//! > callback; [`drain`] admits and answers the UI-only work inside a real frame so replies reflect
//! > the drawn UI.
//!
//! 方法（与原文同一套 id）：
//!
//! - `engine.execute {command, params}` — 跑任意命令（不会开对话框）
//! - `engine.commands {}` — 每条命令连同它的参数说明
//! - `document.inspect {depth?, allPages?}` / `node.get {id, depth?}`
//! - `ui.inspect {}` — 工具/视图/画布矩形/帧统计
//! - `ui.pointer {kind, x, y | sx, sy, to?, steps?, shift?, alt?, cmd?, ctrl?, pressure?}`
//! - `ui.key {key, shift?, alt?, cmd?, ctrl?}` — 当作在画布上按下的快捷键
//! - `ui.type {text}` — 往正在编辑的文字里输入
//! - `ui.render {path?, scale?, ids?}` — 文档 PNG（不含界面，无窗口也能出图）
//!
//! 与原文的差异（只有后端 API，不含行为）：
//!
//! - `egui::Context` 的 `request_repaint` → [`Wake`]（GPUI 的 `WeakEntity` + `AsyncApp`，
//!   跨线程 `cx.notify()`）。
//! - `app.handle_ui_requests()` / `app.invalidate_synthetic_copy` / `app.observe_presentation`
//!   是 egui 侧的面板与多窗口簿记；GPUI 还没有面板体系（§13.2 未搬），所以这几步不存在。
//! - `ui.screenshot` / `ui.input` / `ui.menu.invoke` / `ui.set` / `window.*` 依赖 egui 的
//!   窗口、面板与注入输入，GPUI 版未搬，按「未知方法」报错（见 [`handle`] 末尾）。

use std::sync::{Arc, Mutex};

use figcraft_engine::{PointerEvent, PointerKind};
use serde_json::{Value, json};

use crate::CanvasApp;

/// 跨线程唤醒 UI 线程（原文的 `egui::Context::request_repaint`）。
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// 控制请求的队列（原文 `type Queue = Arc<Mutex<Vec<ControlRequest>>>`）。
pub type Queue = Arc<Mutex<Vec<ControlRequest>>>;

pub struct ControlReply {
    pub id: Value,
    pub result: Result<Value, String>,
}

/// Which side has finished a control request.
///
/// The transport and the UI race to settle one request: while it is `Open`, either may cancel it
/// (the transport's timeout) or commit it (the UI's final reply). Once settled, neither side may
/// change it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RequestState {
    #[default]
    Open,
    /// The transport stopped waiting: no file may be written and no reply is wanted.
    Cancelled,
    /// The UI wrote the file (if any) and queued the reply before releasing the gate.
    Replied,
}

/// The gate the transport's timeout and the UI's final reply share for one request.
#[derive(Clone, Default)]
pub struct RequestGate {
    state: Arc<Mutex<RequestState>>,
}

impl RequestGate {
    /// Create a request gate in its default `Open` state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Return a copy of the gate's current request state.
    pub fn state(&self) -> RequestState {
        *self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Whether the gate is `Cancelled`.
    pub fn is_cancelled(&self) -> bool {
        self.state() == RequestState::Cancelled
    }

    /// Whether the gate is `Replied`.
    pub fn is_replied(&self) -> bool {
        self.state() == RequestState::Replied
    }

    /// Cancel an open request, as the transport's timeout does.
    pub fn cancel_if_open(&self) -> RequestState {
        let mut state = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if *state == RequestState::Open {
            *state = RequestState::Cancelled;
        }
        *state
    }

    /// Run the final commit (reply enqueue) only while the request is still open, then mark it
    /// replied before releasing the gate.
    pub fn commit<T>(&self, finish: impl FnOnce() -> T) -> Option<T> {
        let mut state = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if *state != RequestState::Open {
            return None;
        }
        let value = finish();
        *state = RequestState::Replied;
        Some(value)
    }
}

pub struct ControlRequest {
    pub id: Value,
    pub method: String,
    pub params: Value,
    pub reply: std::sync::mpsc::Sender<ControlReply>,
    /// The transport stops waiting after its timeout; discard requests still queued then.
    pub state: RequestGate,
}

impl ControlRequest {
    /// Whether this request's gate is `Cancelled`.
    pub fn is_cancelled(&self) -> bool {
        self.state.is_cancelled()
    }
}

/// 队列里最多留多少条请求（原文 `MAX_QUEUED_REQUESTS`，socket 侧与 UI 侧各一条同样的上限）。
pub const MAX_QUEUED_REQUESTS: usize = 8;

/// 每帧开头：丢弃已被传输侧取消的请求，再把队列里剩下的交给本帧处理。
///
/// 原文在 `background` 里做的是「截图/几何/可见性」。GPUI 没有截图请求（`ui.screenshot` 未搬），
/// 所以这里只剩「丢掉取消的、把能答的答掉」这一步，语义相同。
pub fn background(queue: &Queue, out: &mut Vec<ControlRequest>) {
    out.retain(|request| !request.is_cancelled());
    let mut queue = queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    queue.retain(|request| !request.is_cancelled());
    let count = queue.len().min(MAX_QUEUED_REQUESTS.saturating_sub(out.len()));
    out.extend(queue.drain(..count));
}

/// 在真实的一帧里把排到的请求答掉（回复因此反映画出来的 UI）。
///
/// 原文 `drain(app, ctx)` 的 UI 部分就是「逐条 `handle` + 回送」；这里一致。
/// 返回是否处理过请求（有的话调用方要 `cx.notify()` 再画一帧）。
pub fn drain(app: &mut CanvasApp, queue: &Queue) -> bool {
    let mut pending = Vec::new();
    background(queue, &mut pending);
    if pending.is_empty() {
        return false;
    }
    for request in pending {
        let result = handle(app, &request.method, &request.params);
        // 传输侧已经不等了：不写文件、不回包。
        let _ = request.state.commit(|| {
            let _ = request.reply.send(ControlReply { id: request.id.clone(), result });
        });
    }
    true
}

fn b(p: &Value, k: &str) -> bool {
    p.get(k).and_then(Value::as_bool).unwrap_or(false)
}

/// Handle one control method.
///
/// 引擎侧的四条（`engine.execute` / `engine.commands` / `document.inspect` / `node.get`）与原文
/// 逐字一致；`ui.*` 用的是 GPUI 的等价物（见模块头的差异说明）。
pub fn handle(app: &mut CanvasApp, method: &str, p: &Value) -> Result<Value, String> {
    match method {
        "engine.execute" => {
            let cmd = p.get("command").and_then(Value::as_str).ok_or("`command` required")?;
            let params = p.get("params").cloned().unwrap_or(json!({}));
            app.session.execute(cmd, &params).map_err(|e| e.to_string())
            // 原文随后还会：清掉评论草稿、作废合成副本、丢掉 `dialog.*` 的 UI 请求、
            // `handle_ui_requests()`、`observe_presentation()`。这些都属于 egui 的面板/多窗口
            // 簿记（§13.2 未搬），GPUI 侧没有对应物。
        }
        "engine.commands" => Ok(Value::Array(
            figcraft_engine::all_commands()
                .iter()
                .map(|c| {
                    json!({
                        "id": c.id,
                        "label": c.label,
                        "params": c.params,
                        "menu": c.menu(),
                        "shortcut": c.shortcut(),
                        "undoable": c.history,
                    })
                })
                .collect(),
        )),
        "document.inspect" => {
            let depth = p.get("depth").and_then(Value::as_u64).unwrap_or(8).min(64) as usize;
            Ok(figcraft_engine::inspect::document(&app.session, depth, b(p, "allPages")))
        }
        "node.get" => {
            let id = p.get("id").and_then(figcraft_engine::params::id_of).ok_or("`id` required")?;
            let depth = p.get("depth").and_then(Value::as_u64).unwrap_or(1) as usize;
            app.session.doc().map(|d| figcraft_engine::inspect::node_json(d, id, depth)).map_err(|e| e.to_string())
        }
        // egui `control.rs` 的 `ui.set`：motion / playhead / rightTab / fps 都是 UI 状态，
        // 引擎命令碰不到，交给 `CanvasView::sync_ui_requests` 在下一帧落地。
        "ui.set" => {
            app.session.request_ui("ui.set", p.clone());
            Ok(json!({ "ok": true }))
        }
        "ui.inspect" => Ok(inspect(app)),
        "ui.pointer" => pointer(app, p),
        "ui.key" => {
            let key = p.get("key").and_then(Value::as_str).ok_or("`key` required")?;
            let m = figcraft_engine::keymap::Mods { shift: b(p, "shift"), alt: b(p, "alt"), cmd: b(p, "cmd"), ctrl: b(p, "ctrl") };
            let platform = figcraft_engine::keymap::Platform::current();
            use figcraft_engine::keymap::{Context, lookup, normalize};
            // 正在编辑文字时，按键交给编辑器（原文同一条分支）。
            if app.session.ds().map(|d| d.text_edit.is_some()).unwrap_or(false) {
                if let Some(bd) = lookup(platform, Context::Text, key, m).filter(|_| m.cmd || m.alt || m.ctrl) {
                    crate::run_binding(app, bd.id);
                    return Ok(json!({"command": bd.id}));
                }
                let (word, line) = figcraft_engine::keymap::text_motion(platform, key, m);
                figcraft_engine::textedit::key(&mut app.session, key, m.shift, word, line).map_err(|e| e.to_string())?;
                return Ok(json!({"ok": true}));
            }
            // 向量编辑模式与形状构建器的键（工具、推点、删除、连接、Enter）。
            if app.session.tool_key(key, m.shift, m.alt, m.cmd).map_err(|e| e.to_string())? {
                return Ok(json!({"ok": true}));
            }
            if let Some(bd) = lookup(platform, Context::Canvas, key, m) {
                crate::run_binding(app, bd.id);
                return Ok(json!({"command": bd.id}));
            }
            let (name, _) = normalize(key).ok_or_else(|| format!("unknown key `{key}`"))?;
            match name {
                "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown" => {
                    let (dx, dy) = match name {
                        "ArrowLeft" => (-1, 0),
                        "ArrowRight" => (1, 0),
                        "ArrowUp" => (0, -1),
                        _ => (0, 1),
                    };
                    let (cmd, params) = if m.cmd || m.ctrl {
                        ("transform.nudgeSize", json!({"dw": dx, "dh": dy, "big": m.shift}))
                    } else {
                        ("transform.nudge", json!({"dx": dx, "dy": dy, "big": m.shift}))
                    };
                    app.session.execute(cmd, &params).map_err(|e| e.to_string())
                }
                _ => Err(format!("no shortcut `{key}`")),
            }
        }
        "ui.type" => {
            let text = p.get("text").and_then(Value::as_str).ok_or("`text` required")?;
            figcraft_engine::textedit::insert(&mut app.session, text).map_err(|e| e.to_string())?;
            Ok(json!({"ok": true}))
        }
        "ui.render" => render(app, p),
        other => Err(format!("unknown method `{other}`")),
    }
}

/// `ui.pointer`：把一次指针手势喂给当前工具（原文 `pointer`，同一套 `PointerEvent`）。
fn pointer(app: &mut CanvasApp, p: &Value) -> Result<Value, String> {
    let zoom = app.session.ds().map(|d| d.view.zoom).unwrap_or(1.0);
    let at = |x: Option<f64>, y: Option<f64>, sx: Option<f64>, sy: Option<f64>| -> Option<(f64, f64)> {
        match (x, y, sx, sy) {
            (Some(x), Some(y), _, _) => Some((x, y)),
            (_, _, Some(sx), Some(sy)) => {
                let pt = app.to_page(crate::canvas::geom::pos2(sx as f32, sy as f32));
                Some((pt.x, pt.y))
            }
            _ => None,
        }
    };
    let f = |k: &str| p.get(k).and_then(Value::as_f64);
    let (x, y) = at(f("x"), f("y"), f("sx"), f("sy")).ok_or("pass x, y (page) or sx, sy (screen)")?;
    let pressure = |k: &str| f(k).filter(|v| v.is_finite()).map(|v| v.clamp(0.0, 1.0) as f32);
    let mk = |kind: PointerKind, x: f64, y: f64| PointerEvent {
        kind,
        x,
        y,
        sx: x * zoom,
        sy: y * zoom,
        shift: b(p, "shift"),
        alt: b(p, "alt"),
        cmd: b(p, "cmd"),
        ctrl: b(p, "ctrl"),
        zoom,
        pressure: pressure("pressure"),
    };
    // A drag's pressure goes from `pressure` to `pressureTo`.
    let (pr0, pr1) = (pressure("pressure"), pressure("pressureTo").or(pressure("pressure")));
    let kind = p.get("kind").and_then(Value::as_str).unwrap_or("click");
    let run = |app: &mut CanvasApp, ev: PointerEvent| app.session.pointer(&ev).map_err(|e| e.to_string());
    match kind {
        "down" => run(app, mk(PointerKind::Down, x, y))?,
        "move" => run(app, mk(PointerKind::Move, x, y))?,
        "up" => run(app, mk(PointerKind::Up, x, y))?,
        "hover" => run(app, mk(PointerKind::Hover, x, y))?,
        "double" => run(app, mk(PointerKind::DoubleClick, x, y))?,
        "click" => {
            run(app, mk(PointerKind::Down, x, y))?;
            run(app, mk(PointerKind::Up, x, y))?;
        }
        "drag" => {
            let to = p.get("to").and_then(Value::as_array).ok_or("drag needs `to: [x, y]`")?;
            let tx = to.first().and_then(Value::as_f64).ok_or("bad `to`")?;
            let ty = to.get(1).and_then(Value::as_f64).ok_or("bad `to`")?;
            let steps = p.get("steps").and_then(Value::as_u64).unwrap_or(8).clamp(1, 200);
            run(app, mk(PointerKind::Down, x, y))?;
            for i in 1..=steps {
                let t = i as f64 / steps as f64;
                let pressure = pr0.zip(pr1).map(|(a, b)| a + (b - a) * t as f32);
                run(app, PointerEvent { pressure, ..mk(PointerKind::Move, x + (tx - x) * t, y + (ty - y) * t) })?;
            }
            run(app, PointerEvent { pressure: pr1, ..mk(PointerKind::Up, tx, ty) })?;
        }
        other => return Err(format!("unknown pointer kind `{other}`")),
    }
    Ok(json!({"selection": app.session.selection(), "tool": app.session.tool.tool.id()}))
}

/// `ui.inspect`：GPUI 侧能报的界面状态（原文字段名保持一致）。
///
/// 面板/对话框/实时协作这些属于 §13.2–13.1 的未搬部分，这里给 `null` 而不是编造。
fn inspect(app: &CanvasApp) -> Value {
    let ds = app.session.ds().ok();
    let view = ds.map(|d| {
        json!({
            "zoom": d.view.zoom,
            "origin": [d.view.origin().0, d.view.origin().1],
            "vw": d.view.vw,
            "vh": d.view.vh,
            "rulers": d.view.rulers,
        })
    });
    let mut v = json!({
        "tool": {
            "id": app.session.tool.tool.id(),
            "shapeHandles": app.session.shape_handles()
                .into_iter()
                .map(|(h, at)| json!({"handle": format!("{h:?}"), "at": at}))
                .collect::<Vec<_>>(),
        },
        "selection": app.session.selection(),
        "canvas": view,
        "canvasRect": [app.canvas.rect.min.x, app.canvas.rect.min.y, app.canvas.rect.max.x, app.canvas.rect.max.y],
        "editing": ds.and_then(|d| d.text_edit.as_ref()).map(|t| t.id.0),
        "cursor": app.cursor.map(|c| format!("{c:?}")),
        "pointer": app.pointer.map(|p| [p.x, p.y]),
        // 面板体系（§13.2）与对话框未搬：字段存在但为 null，不编造。
        "panels": null,
        "dialogs": null,
    });
    // 帧统计：`framesDrawn` / `repaintCauses`（后者 GPUI 无等价 API，见 `frame_stats`）。
    if let (Some(o), Some(s)) = (v.as_object_mut(), app.stats.to_json().as_object().cloned()) {
        o.extend(s);
    }
    v
}

/// `ui.render`：把文档导出成 PNG（不含界面，无窗口也能出图）。
///
/// 与 `preview.rs` 同一条路：`figcraft_render::Retained::render` 出 `Pixmap`，再 PNG 编码。
/// 原文还支持 `ids`（只导出这些层）；GPUI 侧目前整页出图。
fn render(app: &CanvasApp, p: &Value) -> Result<Value, String> {
    let scale = p.get("scale").and_then(Value::as_f64).unwrap_or(1.0).clamp(0.05, 8.0);
    let ds = app.session.ds().map_err(|e| e.to_string())?;
    let (doc, page) = (ds.doc.clone(), ds.page);
    // 出图范围：页面的绘制范围（含效果外扩），按 `scale` 换算成像素。
    let b = figcraft_render::paint_bounds(&doc, page).ok_or("empty page")?;
    let w = ((b.width() * scale).ceil() as u32).max(1).min(16384);
    let h = ((b.height() * scale).ceil() as u32).max(1).min(16384);
    let view = figcraft_render::View { origin: (b.x0, b.y0), scale, width: w, height: h };
    let opts = figcraft_render::Options { lod: false, magnified_image_pixels: true, ..Default::default() };
    let mut retained = figcraft_render::Retained::default();
    let (_, pix, _work) = retained.render(&doc, page, &view, &opts).ok_or("render failed")?;
    let img = image::RgbaImage::from_raw(pix.width(), pix.height(), pix.data().to_vec()).ok_or("pixel buffer mismatch")?;
    let mut png = Vec::new();
    image::ImageEncoder::write_image(
        image::codecs::png::PngEncoder::new(&mut png),
        img.as_raw(),
        pix.width(),
        pix.height(),
        image::ExtendedColorType::Rgba8,
    )
    .map_err(|e| e.to_string())?;
    match p.get("path").and_then(Value::as_str) {
        Some(path) => {
            std::fs::write(path, &png).map_err(|e| format!("{path}: {e}"))?;
            Ok(json!({"path": path, "format": "png", "width": pix.width(), "height": pix.height()}))
        }
        None => Ok(json!({
            "format": "png",
            "width": pix.width(),
            "height": pix.height(),
            "data": base64(&png),
        })),
    }
}

/// 标准 base64（原文用 `image` 生态里的编码器；这里只要能往返）。
fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_commands_lists_the_catalogue() {
        let mut app = CanvasApp::new(crate::app::sample_doc());
        let v = handle(&mut app, "engine.commands", &json!({})).expect("commands");
        let list = v.as_array().expect("array");
        assert!(list.len() > 20, "the catalogue should be large, got {}", list.len());
        assert!(list.iter().any(|c| c["id"] == json!("node.set")));
    }

    #[test]
    fn engine_execute_runs_a_real_command() {
        let mut app = CanvasApp::new(crate::app::sample_doc());
        let page = app.session.ds().expect("doc state").page;
        let created = handle(
            &mut app,
            "engine.execute",
            &json!({"command": "node.create", "params": {"type": "rectangle", "parent": page.0, "width": 40.0, "height": 40.0}}),
        )
        .expect("create");
        let id = created.get("id").expect("id");
        assert!(app.session.doc().unwrap().node(figcraft_engine::params::id_of(id).unwrap()).is_some());
    }

    #[test]
    fn ui_pointer_clicks_select_a_layer() {
        let mut app = CanvasApp::new(crate::app::sample_doc());
        // 示例文档里的蓝色矩形：页面坐标 (120,100)-(320,260)。
        let r = handle(&mut app, "ui.pointer", &json!({"kind": "click", "x": 200.0, "y": 180.0})).expect("click");
        assert!(r.get("selection").is_some());
        assert!(!app.session.selection().is_empty(), "clicking the rectangle should select it");
    }

    #[test]
    fn a_cancelled_request_is_not_answered() {
        let mut app = CanvasApp::new(crate::app::sample_doc());
        let queue: Queue = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = std::sync::mpsc::channel();
        let gate = RequestGate::new();
        gate.cancel_if_open();
        queue.lock().unwrap().push(ControlRequest { id: json!(1), method: "engine.commands".into(), params: json!({}), reply: tx, state: gate });
        assert!(!drain(&mut app, &queue), "a cancelled request leaves nothing to do");
        assert!(rx.try_recv().is_err(), "no reply may be sent for a cancelled request");
    }

    #[test]
    fn base64_round_trips_padding() {
        assert_eq!(base64(b"a"), "YQ==");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b"abc"), "YWJj");
    }

    /// `ui.render` 不需要窗口：直接出 PNG（原文同一语义）。
    #[test]
    fn ui_render_writes_a_png_without_a_window() {
        let mut app = CanvasApp::new(crate::app::sample_doc());
        let dir = std::env::temp_dir().join("figcraft-control-render.png");
        let path = dir.to_string_lossy().to_string();
        let v = handle(&mut app, "ui.render", &json!({"path": path, "scale": 0.25})).expect("render");
        assert_eq!(v["format"], json!("png"));
        let bytes = std::fs::read(&path).expect("png written");
        assert!(bytes.len() > 8 && bytes[..8] == [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A], "PNG signature");
        let _ = std::fs::remove_file(&path);
    }

    /// 端到端：真的开一个 loopback 口，写一行 JSON，UI 侧像帧循环那样 drain，读回一行回复。
    /// 证明 socket → 队列 → `handle` → 回复 整条链是通的（不依赖窗口）。
    #[test]
    fn the_socket_answers_a_request_through_the_queue() {
        use std::io::{BufRead, BufReader, Write};

        let queue: Queue = Arc::new(Mutex::new(Vec::new()));
        let (wake_tx, wake_rx) = std::sync::mpsc::channel::<()>();
        let wake: Wake = Arc::new(move || {
            let _ = wake_tx.send(());
        });

        // 端口 0 = 让系统挑一个空口；`start` 里 bind 后再报出来不便，这里直接选高位随机端口。
        let port = 34_000 + (std::process::id() % 900) as u16;
        assert!(crate::control_server::start(port, queue.clone(), wake), "control server binds");

        let mut app = CanvasApp::new(crate::app::sample_doc());
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
        stream.set_read_timeout(Some(std::time::Duration::from_secs(10))).unwrap();
        writeln!(stream, "{}", json!({"id": 7, "method": "engine.commands", "params": {}})).unwrap();

        // 模拟帧循环：等唤醒 → drain → 回包由 socket 线程写。
        wake_rx.recv_timeout(std::time::Duration::from_secs(5)).expect("woken");
        assert!(drain(&mut app, &queue), "a queued request is handled");

        let mut reply = String::new();
        BufReader::new(stream).read_line(&mut reply).expect("reply line");
        let v: Value = serde_json::from_str(&reply).expect("reply is JSON");
        assert_eq!(v["id"], json!(7));
        assert_eq!(v["ok"], json!(true));
        assert!(v["result"].as_array().is_some_and(|a| !a.is_empty()), "the command catalogue comes back");
    }
}
