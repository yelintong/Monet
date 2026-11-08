//! Localhost JSON-lines control server（照搬 `apps/figcraft/src/control_server.rs`）。
//!
//! 每行 `{"id", "method", "params"}` 排队给 UI 线程；UI 在真实的一帧里处理完，
//! 回一行 `{"id", "ok", "result" | "error"}`。
//!
//! 与原文的差异只有一处：原文唤醒 UI 用 `egui::Context::request_repaint`，
//! 这里用 [`Wake`]（GPUI 的 `WeakEntity` + `AsyncApp`，跨线程 `cx.notify()`）。
//! 连接数（8）、行长上限（32 MiB）、队列上限（8）、超时（60 s）与原文一致。

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::{Value, json};

use crate::control::{ControlRequest, MAX_QUEUED_REQUESTS, Queue, RequestGate, RequestState, Wake};

const MAX_LINE: usize = 32 * 1024 * 1024;
const MAX_CONNECTIONS: usize = 8;
const TIMEOUT: Duration = Duration::from_secs(60);
/// How long the accepting thread waits after a failed accept（原文 `ACCEPT_RETRY`）。
const ACCEPT_RETRY: Duration = Duration::from_millis(100);

/// Why a request's transport wait ended without a control reply.
#[derive(Debug, PartialEq, Eq)]
enum WaitFailure {
    Timeout,
    /// The UI side dropped its sender before replying (the app is closing).
    Disconnected,
}

/// Wait for the UI's reply, and settle a timeout under the same gate the UI's final commit uses.
fn wait_for_reply(
    rx: &std::sync::mpsc::Receiver<crate::control::ControlReply>,
    gate: &RequestGate,
    timeout: Duration,
) -> Result<crate::control::ControlReply, WaitFailure> {
    match rx.recv_timeout(timeout) {
        Ok(reply) => Ok(reply),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => after_timeout(rx, gate),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(WaitFailure::Disconnected),
    }
}

/// The timeout branch of [`wait_for_reply`].
fn after_timeout(
    rx: &std::sync::mpsc::Receiver<crate::control::ControlReply>,
    gate: &RequestGate,
) -> Result<crate::control::ControlReply, WaitFailure> {
    match gate.cancel_if_open() {
        RequestState::Replied => rx.try_recv().map_err(|_| WaitFailure::Disconnected),
        RequestState::Open | RequestState::Cancelled => Err(WaitFailure::Timeout),
    }
}

/// Start listening on `127.0.0.1:port`. Returns whether the listener came up.
pub fn start(port: u16, queue: Queue, wake: Wake) -> bool {
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("FigCraft control server failed to bind port {port}: {e}");
            return false;
        }
    };
    eprintln!("FigCraft control server listening on 127.0.0.1:{port}");
    let spawned = std::thread::Builder::new().name("figcraft-control".into()).spawn(move || {
        let live = Arc::new(AtomicUsize::new(0));
        for stream in accepted(listener.incoming(), || std::thread::sleep(ACCEPT_RETRY)) {
            spawn_connection(stream, queue.clone(), wake.clone(), &live, TIMEOUT);
        }
    });
    if let Err(e) = spawned {
        eprintln!("control server thread: {e}");
        return false;
    }
    true
}

/// The connections `incoming` accepts, calling `wait` after each failed accept.
fn accepted<T>(incoming: impl Iterator<Item = std::io::Result<T>>, mut wait: impl FnMut()) -> impl Iterator<Item = T> {
    incoming.filter_map(move |stream| stream.map_err(|_| wait()).ok())
}

/// 连接计数（原文靠 `workers.retain(is_finished)` 数线程；这里用原子计数更直接，
/// 上限仍是 8，退出的连接把名额还回去）。
fn spawn_connection(stream: TcpStream, queue: Queue, wake: Wake, live: &Arc<AtomicUsize>, timeout: Duration) {
    if live.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
        live.fetch_sub(1, Ordering::SeqCst);
        return;
    }
    let live = live.clone();
    let joined = live.clone();
    match std::thread::Builder::new().name("figcraft-control-conn".into()).spawn(move || {
        let _ = serve(stream, queue, wake, timeout);
        joined.fetch_sub(1, Ordering::SeqCst);
    }) {
        Ok(_) => {}
        Err(e) => {
            live.fetch_sub(1, Ordering::SeqCst);
            eprintln!("control connection thread: {e}");
        }
    }
}

/// Serve one control connection: read bounded JSON lines, reject oversized frames and a
/// full queue, queue one request, wait under its gate, and write one reply. Non-JSON input
/// answers with an error and closes the connection so HTTP body lines cannot become commands.
fn serve(mut out: TcpStream, queue: Queue, wake: Wake, timeout: Duration) -> std::io::Result<()> {
    out.set_read_timeout(Some(timeout))?;
    out.set_write_timeout(Some(timeout))?;
    let mut reader = BufReader::new(out.try_clone()?);
    let mut line = Vec::new();
    loop {
        line.clear();
        if (&mut reader).take((MAX_LINE + 1) as u64).read_until(b'\n', &mut line)? == 0 {
            break;
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        if line.len() > MAX_LINE {
            writeln!(out, "{}", json!({"id": null, "ok": false, "error": "message too large"}))?;
            break;
        }
        let Ok(line) = std::str::from_utf8(&line) else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(line) {
            Ok(v) => {
                let id = v.get("id").cloned().unwrap_or(Value::Null);
                let method = v.get("method").and_then(Value::as_str).unwrap_or("").to_string();
                let params = v.get("params").cloned().unwrap_or(json!({}));
                let (tx, rx) = std::sync::mpsc::channel();
                let gate = RequestGate::new();
                {
                    let mut pending = queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    pending.retain(|request| !request.is_cancelled());
                    if pending.len() >= MAX_QUEUED_REQUESTS {
                        drop(pending);
                        writeln!(out, "{}", json!({"id": id, "ok": false, "error": "control request queue is full"}))?;
                        continue;
                    }
                    pending.push(ControlRequest { id: id.clone(), method, params, reply: tx, state: gate.clone() });
                }
                // 唤醒 UI 线程：它下一帧就会把队列抽走（原文 `ctx.request_repaint()`）。
                wake();
                match wait_for_reply(&rx, &gate, timeout) {
                    Ok(r) => match r.result {
                        Ok(res) => json!({"id": r.id, "ok": true, "result": res}),
                        Err(e) => json!({"id": r.id, "ok": false, "error": e}),
                    },
                    Err(WaitFailure::Timeout) => {
                        json!({"id": id, "ok": false, "error": "timed out waiting for the UI"})
                    }
                    Err(WaitFailure::Disconnected) => {
                        json!({"id": id, "ok": false, "error": "the FigCraft window closed before answering"})
                    }
                }
            }
            Err(e) => {
                // A line that isn't JSON ends the connection: a web page can make the browser send
                // an HTTP request here, and its header lines must not let the JSON lines of its body
                // through as commands.
                writeln!(out, "{}", json!({"id": null, "ok": false, "error": format!("bad JSON: {e}")}))?;
                break;
            }
        };
        writeln!(out, "{reply}")?;
    }
    Ok(())
}
