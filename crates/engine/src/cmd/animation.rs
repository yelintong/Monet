//! Animation commands: expose the prototype transition geometry to the UI and the control channel.
//!
//! The playback itself lives in `figcraft_doc::anim`; these commands are thin entry points so the
//! canvas, the prototype panel and the agent control channel can drive transitions without knowing
//! the geometry.

use figcraft_doc::anim::motion::{self, AnimProperty, Clip, ClipKind, ClipMode, Ease, NodeMotion};
use figcraft_doc::proto::Transition;
use figcraft_geom::Vec2;
use serde_json::{Value, json};
use crate::params::*;
use crate::{EngineError, Result, Session, bad, spec};
use super::CommandSpec;

pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "animation.transitionPoses",
            "Transition poses",
            "transition: a Transition object (type + optional direction), progress: 0..1, width, height → the from/to pose (offset + alpha) of a prototype transition.",
            false,
            transition_poses
        ),
        spec!(
            "motion.preset",
            "Animation preset",
            "{preset, at?} preset: a library preset name (Burst, Fade…), at: start in ms (default 0) → applies it to the target layers (ids or selection).",
            true,
            motion_preset
        ),
        spec!(
            "motion.keyframe",
            "Keyframe",
            "{property?, time?, value?, all?} adds a keyframe to the target layers (ids or selection); property is required unless `all` picks every animated track; a value of the wrong shape is refused before anything is written.",
            true,
            motion_keyframe
        ),
        spec!(
            "motion.autoKey",
            "Auto key",
            "{property, value, time?, ids?} the inspector's edit at playhead: if the property already has a keyframe track, sets a key at `time` (default 0); otherwise writes the node's static property and creates no animation. Matches Figma/desigma auto-key semantics.",
            true,
            motion_auto_key
        ),
        spec!(
            "motion.keyframeCopy",
            "Copy keyframes",
            "{from, to, ids?, properties?} copies the actual keys (value + easing) present at `from` to `to` on each target layer; omit `properties` to copy every track that has a key at `from`. One undo step; doesn't copy interpolated values, only real keyframes.",
            true,
            motion_keyframe_copy
        ),
        // PR#1624 的 10 条 motion 命令：以下 7 条本地原先缺失，UI 只能 `doc_mut()` 直写。
        // 补齐后 menu / 键盘 / CLI / --control / MCP 全走同一批 id（architecture.md「one path」）。
        spec!(
            "motion.keyframeMove",
            "Move keyframe",
            "{node, property, time, to} → moves a keyframe, clamped between its neighbours so two keys never share a time.",
            true,
            keyframe_move
        ),
        spec!(
            "motion.keyframeDelete",
            "Delete keyframe",
            "{node, property, time} → removes the keyframe; an emptied track disappears.",
            true,
            keyframe_delete
        ),
        spec!(
            "motion.keyframeEase",
            "Set keyframe easing",
            "{node, property, time, ease} → sets the easing of the segment starting at this keyframe.",
            true,
            keyframe_ease
        ),
        spec!(
            "motion.keyframeHandle",
            "Keyframe spatial handle",
            "{node, property, time, end: out|in, handle: [x, y] | null} → the segment's spatial Bezier handle, relative to the key's value; `null` clears it back to a straight line.",
            true,
            keyframe_handle
        ),
        spec!(
            "motion.clip",
            "Add animation clip",
            "{kind, mode?, time?, node? | ids?} → adds a clip of `kind` to the target layers.",
            true,
            clip_add
        ),
        spec!(
            "motion.clipUpdate",
            "Edit animation clip",
            "{node, id, clip} (the whole clip, as the editor holds it) → replaces one clip, sanitized and normalized.",
            true,
            clip_update
        ),
        spec!(
            "motion.clipDelete",
            "Delete animation clip",
            "{node, id} → removes one clip.",
            true,
            clip_delete
        ),
        spec!(
            "motion.track",
            "Show/hide track",
            "{node, property, visible?} (without `visible`, the track flips) → toggles a track's visibility.",
            true,
            track_visible
        ),
        spec!(
            "motion.replace",
            "Replace motion",
            "{node, motion} → replaces a node's whole motion (tracks + clips), normalized.",
            true,
            motion_replace
        ),
        spec!(
            "motion.timeline",
            "Motion timeline",
            "{duration?, fps?, loop?, page?} sets a page's motion timeline (default page when omitted); writing the defaults drops the page's entry.",
            true,
            motion_timeline
        ),
        spec!(
            "motion.export",
            "Export motion animation",
            "{id?, format?: gif|webm|png|zip|png-zip (default gif), fps?, scale?, quality? (webm 1-100, default 85), speed? (webm 0-10, default 9)} → {format, label, data (base64), frames, fps, width, height, loop}. Renders a frame's own timeline frame by frame with the CPU renderer and encodes it as a GIF, an AV1 WebM (pure Rust), or a zip of PNG frames. No document write, no undo.",
            false,
            |s: &mut Session, p: &Value| super::motion_export::run(s, p)
        ),
    ]
}

fn motion_preset(s: &mut Session, p: &Value) -> Result<Value> {
    let name = p.get("preset").and_then(Value::as_str).unwrap_or_default();
    let at = num(p, "at").unwrap_or(0.0).max(0.0) as u32;
    // 支持显式 `ids`（控制通道 / CLI / UI 命令收敛后都传 ids），无 ids 才回退选区。
    let ids = req_targets("motion.preset", s, p)?;
    let Some(preset) = motion::find_preset(name) else {
        return Ok(json!({"applied": 0}));
    };
    let doc = s.doc_mut()?;
    let mut applied = 0;
    for id in &ids {
        if let Some(node) = doc.node_mut(*id) {
            let first = node.motion.next_clip_id();
            node.motion.clips.extend(preset.instantiate(at, first));
            node.motion.normalize();
            applied += 1;
        }
    }
    doc.mark_dirty(ids);
    Ok(json!({"applied": applied}))
}

fn property_of(name: &str) -> Option<AnimProperty> {
    AnimProperty::ALL.iter().copied().find(|p| p.name().eq_ignore_ascii_case(name))
}

/// PR#1624 命令参数 helper：从 `property` 取属性，非法值报错时列出全部合法名。
fn prop_param(cmd: &str, p: &Value) -> Result<AnimProperty> {
    let name = req_string(cmd, p, "property")?;
    property_of(&name).ok_or_else(|| {
        let names: Vec<&str> = AnimProperty::ALL.iter().map(AnimProperty::name).collect();
        bad(cmd, format!("`property` must be one of {}", names.join(", ")))
    })
}

fn kind_of(name: &str) -> Option<ClipKind> {
    ClipKind::ALL.into_iter().find(|k| k.name().eq_ignore_ascii_case(name))
}

/// 属性值的形状，用于「写前拒绝错形状」的错误信息。
fn shape(kind: motion::ValueKind) -> &'static str {
    match kind {
        motion::ValueKind::Scalar => "a single number",
        motion::ValueKind::Vec2 => "a pair of numbers",
        motion::ValueKind::Color => "a colour",
    }
}

fn mode_of(name: &str) -> Option<ClipMode> {
    match name.trim().to_ascii_lowercase().as_str() {
        "in" => Some(ClipMode::In),
        "out" => Some(ClipMode::Out),
        "custom" => Some(ClipMode::Custom),
        _ => None,
    }
}

/// 取节点的 motion，节点不存在则报 `NoNode`。
fn motion_mut(doc: &mut figcraft_doc::Document, node: figcraft_doc::NodeId) -> Result<&mut NodeMotion> {
    doc.node_mut(node).map(|n| &mut n.motion).ok_or(EngineError::NoNode(node))
}

fn motion_keyframe(s: &mut Session, p: &Value) -> Result<Value> {
    // PR#1624 等价：支持 property / all / value，且**写前**校验 value 形状。
    // 本地旧版只取一个 property、无 value、无校验：给 Position 写标量会让 normalize 丢掉
    // 两项 key，却报告 added=1（删了数据还报成功），这正是 PR body 里那条 fix。
    let time = num(p, "time").unwrap_or(0.0).max(0.0) as u32;
    let value: Option<motion::AnimValue> = parse("motion.keyframe", p, "value")?;
    let every_track = boolean(p, "all").unwrap_or(false);
    let property = match string(p, "property") {
        Some(name) => property_of(&name),
        None => None,
    };
    if property.is_none() && !every_track {
        return Err(bad("motion.keyframe", "`property` is required (or `all`, for every track the layer already animates)"));
    }
    // 支持显式 `ids`（UI 收敛后 add_key/commit_value 都传 ids），无 ids 才回退选区。
    let ids = req_targets("motion.keyframe", s, p)?;
    // 任一 target 上 value 形状不对 → 整条命令拒绝，一个字都不写。
    if let Some(v) = &value {
        let doc = s.doc()?;
        for id in &ids {
            let props: Vec<AnimProperty> = property
                .map_or_else(|| doc.node(*id).map(|n| n.motion.tracks.iter().map(|t| t.property).collect()).unwrap_or_default(), |p| vec![p]);
            if let Some(prop) = props.iter().find(|prop| !prop.accepts(v)) {
                return Err(bad("motion.keyframe", format!("`value` does not fit `{}`: it takes {}", prop.name(), shape(prop.kind()))));
            }
        }
    }
    let doc = s.doc_mut()?;
    let mut added = 0;
    for id in &ids {
        let props: Vec<AnimProperty> = property
            .map_or_else(|| doc.node(*id).map(|n| n.motion.tracks.iter().map(|t| t.property).collect()).unwrap_or_default(), |p| vec![p]);
        let mut keys: Vec<(AnimProperty, motion::AnimValue)> = Vec::new();
        for prop in props {
            let v = value.clone().or_else(|| doc.node(*id).and_then(|n| motion::keyframed_value_at(n, prop, time as f64)));
            if let Some(v) = v {
                keys.push((prop, v));
            }
        }
        if let Some(n) = doc.node_mut(*id) {
            for (prop, v) in keys {
                n.motion.ensure_track(prop).set_key(time, v, None);
                added += 1;
            }
            n.motion.normalize();
        }
    }
    doc.mark_dirty(ids);
    Ok(json!({"added": added, "time": time}))
}

/// Auto-key: the edit the inspector makes when a property field changes at playhead `time`.
/// Ported from desigma `edit::auto_key` (`desigma-doc/src/motion/edit.rs:371`): if the layer
/// already animates `property`, set/update the key at `time`; otherwise change the node's static
/// property and create no animation (so editing a never-animated field doesn't invent a track).
fn motion_auto_key(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "motion.autoKey";
    let time = num(p, "time").unwrap_or(0.0).max(0.0) as u32;
    let property = string(p, "property").and_then(|name| property_of(&name)).ok_or_else(|| bad(cmd, "`property` is required and must be a known animatable property"))?;
    let value: motion::AnimValue = parse(cmd, p, "value")?.ok_or_else(|| bad(cmd, "`value` is required"))?;
    let ids = req_targets(cmd, s, p)?;
    // Atomic shape check: refuse before writing anything if any target rejects the value.
    for id in &ids {
        let accepts = s.doc()?.node(*id).is_some_and(|n| property.accepts(&value));
        if !accepts {
            return Err(bad(cmd, format!("`value` does not fit `{}` on node {id}", property.name())));
        }
    }
    let doc = s.doc_mut()?;
    let mut animated = 0usize;
    let mut static_edits = 0usize;
    for id in &ids {
        let is_animated = doc.node(*id).is_some_and(|n| n.motion.track(property).is_some());
        if is_animated {
            if let Some(n) = doc.node_mut(*id) {
                n.motion.ensure_track(property).set_key(time, value, None);
                n.motion.normalize();
            }
            animated += 1;
        } else {
            // Static edit: EvaluatedNode is desigma's exact equivalent (default().set().apply_to).
            // Only `property`'s field is Some, so apply_to leaves every other property untouched.
            if let Some(base) = doc.node(*id).cloned() {
                let mut ev = motion::EvaluatedNode::default();
                ev.set(property, value);
                let mut out = base.clone();
                ev.apply_to(&base, &mut out);
                if let Some(slot) = doc.node_mut(*id) {
                    *slot = out;
                }
            }
            static_edits += 1;
        }
    }
    doc.mark_dirty(ids);
    Ok(json!({"animated": animated, "static": static_edits, "time": time}))
}

/// Frame copy: duplicate the real keyframes (value + easing) at `from` onto `to`. Unlike
/// `motion.keyframe {all}` (which samples the interpolated value at `to`), this copies the actual
/// key objects, so a "copy frame" round-trips value and easing exactly. Omit `properties` to copy
/// every track that has a key at `from`. All targets are written in one session interaction = one
/// undo step (architecture.md "one path": menu / keyboard / --control / MCP share this id).
fn motion_keyframe_copy(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "motion.keyframeCopy";
    let from = num(p, "from").ok_or_else(|| bad(cmd, "`from` is required"))?.max(0.0) as u32;
    let to = num(p, "to").ok_or_else(|| bad(cmd, "`to` is required"))?.max(0.0) as u32;
    // Optional explicit property list; otherwise every animated track participates.
    let props: Option<Vec<AnimProperty>> = p.get("properties").and_then(Value::as_array).map(|arr| {
        arr.iter().filter_map(|v| v.as_str().and_then(property_of)).collect()
    });
    if let Some(list) = &props {
        if list.is_empty() {
            return Err(bad(cmd, "`properties` is empty or has no known animatable property"));
        }
    }
    let ids = req_targets(cmd, s, p)?;
    let doc = s.doc_mut()?;
    let mut copied = 0usize;
    for id in &ids {
        let Some(n) = doc.node_mut(*id) else { continue };
        // Snapshot (property, value, easing) of keys at `from` first — avoids borrowing the track
        // while mutating it through set_key.
        let keys: Vec<(AnimProperty, motion::AnimValue, Ease)> = n
            .motion
            .tracks
            .iter()
            .filter(|tr| props.as_ref().is_none_or(|list| list.contains(&tr.property)))
            .filter_map(|tr| tr.key_at(from).map(|k| (tr.property, k.value, k.easing)))
            .collect();
        for (prop, value, easing) in keys {
            n.motion.ensure_track(prop).set_key(to, value, Some(easing));
            copied += 1;
        }
        n.motion.normalize();
    }
    doc.mark_dirty(ids);
    Ok(json!({"copied": copied, "from": from, "to": to}))
}

// ── PR#1624 补齐的 7 条命令：keyframeMove/Delete/Ease、clip/Update/Delete、track、replace ──
// 每条写前校验参数、`normalize()` 不变量、`mark_dirty` 标记，走 session 的 Arc 快照 undo。

fn keyframe_move(s: &mut Session, p: &Value) -> Result<Value> {
    let node = req_id("motion.keyframeMove", p, "node")?;
    let prop = prop_param("motion.keyframeMove", p)?;
    let from = req_num("motion.keyframeMove", p, "time")?.max(0.0) as u32;
    let to = req_num("motion.keyframeMove", p, "to")?.max(0.0) as u32;
    let doc = s.doc_mut()?;
    let m = motion_mut(doc, node)?;
    let mut landed = from;
    if let Some(tr) = m.track_mut(prop) {
        // key 必须留在相邻 key 之间：两个 key 同一时刻会让编辑器无法按时寻址。
        let lo = tr.prev_key_time(from).map_or(0, |p| p.saturating_add(1));
        let hi = tr.next_key_time(from).map_or(u32::MAX, |p| p.saturating_sub(1));
        let time = to.clamp(lo, hi);
        if let Some(key) = tr.key_at(from).copied()
            && let Some(idx) = tr.key_index(from)
        {
            tr.keys[idx] = motion::Keyframe { time_ms: time, ..key };
            landed = time;
        }
    }
    m.normalize();
    doc.mark_dirty([node]);
    Ok(json!({"time": landed}))
}

fn keyframe_delete(s: &mut Session, p: &Value) -> Result<Value> {
    let node = req_id("motion.keyframeDelete", p, "node")?;
    let prop = prop_param("motion.keyframeDelete", p)?;
    let time = req_num("motion.keyframeDelete", p, "time")?.max(0.0) as u32;
    let doc = s.doc_mut()?;
    let m = motion_mut(doc, node)?;
    let mut removed = false;
    if let Some(tr) = m.track_mut(prop) {
        removed = tr.key_at(time).is_some();
        tr.remove_key(time);
    }
    m.normalize();
    doc.mark_dirty([node]);
    Ok(json!({"removed": removed}))
}

fn keyframe_ease(s: &mut Session, p: &Value) -> Result<Value> {
    let node = req_id("motion.keyframeEase", p, "node")?;
    let prop = prop_param("motion.keyframeEase", p)?;
    let time = req_num("motion.keyframeEase", p, "time")?.max(0.0) as u32;
    let ease = parse::<Ease>("motion.keyframeEase", p, "ease")?.ok_or_else(|| bad("motion.keyframeEase", "`ease` is required"))?;
    let doc = s.doc_mut()?;
    let m = motion_mut(doc, node)?;
    let mut eased = false;
    if let Some(tr) = m.track_mut(prop)
        && let Some(idx) = tr.key_index(time)
    {
        tr.keys[idx].easing = ease.sanitized();
        eased = true;
    }
    m.normalize();
    doc.mark_dirty([node]);
    Ok(json!({"eased": eased}))
}

/// 空间贝塞尔手柄：`{node, property, time, end: out|in, handle: [x, y] | null}`。
/// 手柄相对该关键帧的值；`null` 清除（回到直线）。默认对称由 UI 传两个反向手柄实现。
fn keyframe_handle(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "motion.keyframeHandle";
    let node = req_id(cmd, p, "node")?;
    let prop = prop_param(cmd, p)?;
    let time = req_num(cmd, p, "time")?.max(0.0) as u32;
    let end = req_string(cmd, p, "end")?;
    let handle: Option<[f64; 2]> = match p.get("handle") {
        Some(Value::Null) | None => None,
        Some(v) => Some(serde_json::from_value(v.clone()).map_err(|e| bad(cmd, e.to_string()))?),
    };
    let doc = s.doc_mut()?;
    let m = motion_mut(doc, node)?;
    let mut set = false;
    if let Some(tr) = m.track_mut(prop)
        && let Some(idx) = tr.key_index(time)
    {
        match end.as_str() {
            "out" => tr.keys[idx].out_handle = handle,
            "in" => tr.keys[idx].in_handle = handle,
            _ => return Err(bad(cmd, "`end` must be `out` or `in`")),
        }
        set = true;
    }
    doc.mark_dirty([node]);
    Ok(json!({"set": set}))
}

fn clip_add(s: &mut Session, p: &Value) -> Result<Value> {
    let name = req_string("motion.clip", p, "kind")?;
    let kind = kind_of(&name).ok_or_else(|| {
        let names: Vec<&str> = ClipKind::ALL.iter().map(ClipKind::name).collect();
        bad("motion.clip", format!("`kind` must be one of {}", names.join(", ")))
    })?;
    let mode = match string(p, "mode") {
        Some(n) => mode_of(&n).ok_or_else(|| bad("motion.clip", "`mode` must be In, Out or Custom"))?,
        None => ClipMode::In,
    };
    let time = num(p, "time").unwrap_or(0.0).max(0.0) as u32;
    let ids = req_targets("motion.clip", s, p)?;
    let doc = s.doc_mut()?;
    let mut clips = Vec::new();
    for id in &ids {
        let Some(n) = doc.node_mut(*id) else { continue };
        let mut c = Clip::new(kind, mode);
        c.id = n.motion.next_clip_id();
        c.delay_ms = time;
        clips.push(json!({"node": id.0, "id": c.id}));
        n.motion.clips.push(c);
        n.motion.normalize();
    }
    doc.mark_dirty(ids);
    Ok(json!({"clips": clips}))
}

fn clip_update(s: &mut Session, p: &Value) -> Result<Value> {
    let node = req_id("motion.clipUpdate", p, "node")?;
    let id = req_num("motion.clipUpdate", p, "id")?.max(0.0) as u32;
    let mut clip = parse::<Clip>("motion.clipUpdate", p, "clip")?.ok_or_else(|| bad("motion.clipUpdate", "`clip` is required"))?;
    clip.id = id;
    clip.sanitize();
    let doc = s.doc_mut()?;
    let m = motion_mut(doc, node)?;
    let Some(c) = m.clips.iter_mut().find(|c| c.id == id) else {
        return Err(bad("motion.clipUpdate", format!("layer {} has no clip {id}", node.0)));
    };
    *c = clip;
    m.normalize();
    doc.mark_dirty([node]);
    Ok(json!({"ok": true}))
}

fn clip_delete(s: &mut Session, p: &Value) -> Result<Value> {
    let node = req_id("motion.clipDelete", p, "node")?;
    let id = req_num("motion.clipDelete", p, "id")?.max(0.0) as u32;
    let doc = s.doc_mut()?;
    let m = motion_mut(doc, node)?;
    let before = m.clips.len();
    m.clips.retain(|c| c.id != id);
    let removed = m.clips.len() != before;
    m.normalize();
    doc.mark_dirty([node]);
    Ok(json!({"removed": removed}))
}

fn track_visible(s: &mut Session, p: &Value) -> Result<Value> {
    let node = req_id("motion.track", p, "node")?;
    let prop = prop_param("motion.track", p)?;
    let wanted = boolean(p, "visible");
    let doc = s.doc_mut()?;
    let m = motion_mut(doc, node)?;
    let name = prop.name();
    let Some(tr) = m.track_mut(prop) else {
        return Err(bad("motion.track", format!("layer {} has no {name} track", node.0)));
    };
    tr.visible = wanted.unwrap_or(!tr.visible);
    let visible = tr.visible;
    doc.mark_dirty([node]);
    Ok(json!({"visible": visible}))
}

fn motion_replace(s: &mut Session, p: &Value) -> Result<Value> {
    let node = req_id("motion.replace", p, "node")?;
    let mut m = parse::<NodeMotion>("motion.replace", p, "motion")?.ok_or_else(|| bad("motion.replace", "`motion` is required"))?;
    m.normalize();
    let doc = s.doc_mut()?;
    *motion_mut(doc, node)? = m;
    doc.mark_dirty([node]);
    Ok(json!({"ok": true}))
}

fn motion_timeline(s: &mut Session, p: &Value) -> Result<Value> {
    // PR#1624：可指定 page（不指定用当前页）；写回默认值时**删除条目**而非留一条默认值，
    // 让「处于默认设置的页」不进文件、不进 collab（本地旧版总是 insert，文件会残留默认条目）。
    let page = match opt_id("motion.timeline", p, "page")? {
        Some(page) => page,
        None => s.page()?,
    };
    let mut settings = s.doc()?.timelines.get(&page).copied().unwrap_or_default();
    if let Some(d) = num(p, "duration") {
        settings.duration_ms = d.max(0.0) as u32;
    }
    if let Some(f) = num(p, "fps") {
        settings.fps = f.max(0.0) as u32;
    }
    if let Some(l) = p.get("loop").and_then(Value::as_bool) {
        settings.looping = l;
    }
    let settings = settings.sanitized();
    let doc = s.doc_mut()?;
    if settings.is_default() {
        doc.timelines.remove(&page);
    } else {
        doc.timelines.insert(page, settings);
    }
    doc.mark_dirty([page]);
    Ok(json!({"duration": settings.duration_ms, "fps": settings.fps, "loop": settings.looping}))
}

fn transition_poses(_s: &mut Session, p: &Value) -> Result<Value> {
    let transition = parse::<Transition>("animation.transitionPoses", p, "transition")?.unwrap_or(Transition::Dissolve);
    let progress = num(p, "progress").unwrap_or(0.0);
    let w = num(p, "width").unwrap_or(0.0);
    let h = num(p, "height").unwrap_or(0.0);
    let poses = figcraft_doc::anim::transitions::poses(&transition, progress, Vec2::new(w, h));
    Ok(json!({
        "from": { "offset": [poses.from.offset.x, poses.from.offset.y], "alpha": poses.from.alpha },
        "to": { "offset": [poses.to.offset.x, poses.to.offset.y], "alpha": poses.to.alpha },
        "toOnTop": poses.to_on_top,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params;
    use figcraft_doc::NodeId;
    use serde_json::{Value, json};

    fn id(v: &Value) -> NodeId {
        params::id_of(&v["id"]).unwrap()
    }

    fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
        s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
    }

    fn node(s: &Session, n: NodeId) -> &figcraft_doc::Node {
        s.doc().unwrap().node(n).unwrap()
    }

    fn rect(s: &mut Session) -> NodeId {
        id(&ex(s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 40, "height": 20})))
    }

    /// PR#1624 的 clip 增删改命令链：每个都是一次 execute，文档变化可断言、可撤销。
    #[test]
    fn clip_add_update_delete_commands() {
        let mut s = Session::new();
        let n = rect(&mut s);
        let out = ex(&mut s, "motion.clip", json!({"kind": "Position", "mode": "In", "time": 100, "ids": [n.0]}));
        let cid = out["clips"][0]["id"].as_u64().unwrap() as u32;
        assert_eq!(node(&s, n).motion.clips.len(), 1);

        // clipUpdate 整条替换（改 delay）：真实调用方传的是序列化后的整条 clip（含 id）。
        // 本地 `Clip.id` 没有 serde(default)，所以这里按真实形态补齐全部必填字段。
        let mut clip: Clip = serde_json::from_value(json!({
            "id": cid, "kind": "Position", "mode": "In", "direction": "Bottom",
            "amount": 0.0, "from": [0.0, 0.0], "to": [0.0, 0.0],
            "delay_ms": 0, "duration_ms": 500
        })).unwrap();
        clip.delay_ms = 250;
        ex(&mut s, "motion.clipUpdate", json!({"node": n.0, "id": cid, "clip": clip}));
        assert_eq!(node(&s, n).motion.clips[0].delay_ms, 250);

        let out = ex(&mut s, "motion.clipDelete", json!({"node": n.0, "id": cid}));
        assert_eq!(out["removed"], true);
        assert!(node(&s, n).motion.clips.is_empty());
    }

    /// keyframeMove 把 key 夹在相邻 key 之间，两个 key 永不共享同一时刻（编辑器可按时寻址）。
    #[test]
    fn keyframe_move_clamps_between_neighbours() {
        let mut s = Session::new();
        let n = rect(&mut s);
        for t in [0, 200, 400] {
            ex(&mut s, "motion.keyframe", json!({"property": "Opacity", "time": t, "ids": [n.0]}));
        }
        // 把 t=200 想拖到 500：被下一个 key(400) 夹到 399。
        let out = ex(&mut s, "motion.keyframeMove", json!({"node": n.0, "property": "Opacity", "time": 200, "to": 500}));
        assert_eq!(out["time"], 399);
        let times: Vec<u32> = node(&s, n).motion.track(AnimProperty::Opacity).unwrap().keys.iter().map(|k| k.time_ms).collect();
        assert_eq!(times, vec![0, 399, 400]);
    }

    /// 写前拒绝错形状：给 Position（Vec2）发标量，整条命令失败，一个 key 都不写。
    #[test]
    fn keyframe_refuses_wrong_value_shape_without_writing() {
        let mut s = Session::new();
        let n = rect(&mut s);
        let before = node(&s, n).motion.clone();
        let err = s.execute("motion.keyframe", &json!({"property": "Position", "time": 0, "value": 5.0, "ids": [n.0]}));
        assert!(err.is_err(), "标量不能给 Position");
        assert_eq!(node(&s, n).motion, before, "被拒的写入不碰文档");
    }

    /// timeline 写回默认值时删条目（默认页不进文件 / 不进 collab）；可指定 page。
    #[test]
    fn setting_the_defaults_takes_the_page_out_of_the_timelines() {
        let mut s = Session::new();
        let page = s.page().expect("a page");
        ex(&mut s, "motion.timeline", json!({"duration": 5000, "fps": 30, "loop": true, "page": page.0}));
        assert!(s.doc().unwrap().timelines.contains_key(&page));
        ex(&mut s, "motion.timeline", json!({"duration": 2000, "fps": 60, "loop": false, "page": page.0}));
        assert!(!s.doc().unwrap().timelines.contains_key(&page), "默认设置不留条目");
    }

    /// 用户 2026-11-12「画框 = 唯一动画单位」：`motion.timeline {page: frame}` 把设置写到该
    /// 画框的 key（UI 的 scope 写入依赖此契约），画框与页面各自独立，写默认值只删画框条目。
    #[test]
    fn timeline_settings_write_to_a_frame_scope_independently_of_its_page() {
        let mut s = Session::new();
        let page = s.page().expect("a page");
        let frame = id(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 300, "height": 200})));
        // A frame entry wins for that frame, leaving the page untouched.
        ex(&mut s, "motion.timeline", json!({"duration": 300, "fps": 60, "loop": true, "page": frame.0}));
        assert_eq!(s.doc().unwrap().timelines.get(&frame).unwrap().duration_ms, 300);
        assert!(!s.doc().unwrap().timelines.contains_key(&page), "frame write must not touch the page");
        // The page keeps its own, independent settings.
        ex(&mut s, "motion.timeline", json!({"duration": 1000, "fps": 30, "loop": false, "page": page.0}));
        assert_eq!(s.doc().unwrap().timelines.get(&page).unwrap().duration_ms, 1000);
        assert_eq!(s.doc().unwrap().timelines.get(&frame).unwrap().duration_ms, 300, "page write must not touch the frame");
        // Writing defaults back only removes the frame's entry.
        ex(&mut s, "motion.timeline", json!({"duration": 2000, "fps": 60, "loop": false, "page": frame.0}));
        assert!(!s.doc().unwrap().timelines.contains_key(&frame), "default frame settings leave no entry");
        assert!(s.doc().unwrap().timelines.contains_key(&page), "the page entry survives");
    }

    /// desigma `auto_key` 语义：没有 track 的属性改值时只改静态属性、**不凭空建动画**。
    #[test]
    fn auto_key_writes_static_value_when_no_track() {
        let mut s = Session::new();
        let n = rect(&mut s);
        assert!(node(&s, n).motion.track(AnimProperty::Opacity).is_none());
        let v = serde_json::to_value(motion::AnimValue::Scalar(0.5)).unwrap();
        let out = ex(&mut s, "motion.autoKey", json!({"property": "Opacity", "value": v, "time": 750, "ids": [n.0]}));
        assert_eq!(out["static"], 1);
        assert_eq!(out["animated"], 0);
        assert!((node(&s, n).opacity - 0.5).abs() < 1e-6, "static opacity written");
        assert!(node(&s, n).motion.track(AnimProperty::Opacity).is_none(), "no animation created");
    }

    /// 已有 track 的属性：在播放头 time 打/改帧，静态值不动（Figma 式）。
    #[test]
    fn auto_key_sets_a_key_at_playhead_when_animated() {
        let mut s = Session::new();
        let n = rect(&mut s);
        // Establish an Opacity track with one key at 0.
        let one = serde_json::to_value(motion::AnimValue::Scalar(1.0)).unwrap();
        ex(&mut s, "motion.keyframe", json!({"property": "Opacity", "time": 0, "value": one, "ids": [n.0]}));
        let static_before = node(&s, n).opacity;
        let quarter = serde_json::to_value(motion::AnimValue::Scalar(0.25)).unwrap();
        let out = ex(&mut s, "motion.autoKey", json!({"property": "Opacity", "value": quarter, "time": 750, "ids": [n.0]}));
        assert_eq!(out["animated"], 1);
        assert_eq!(out["static"], 0);
        let tr = node(&s, n).motion.track(AnimProperty::Opacity).unwrap();
        let times: Vec<u32> = tr.keys.iter().map(|k| k.time_ms).collect();
        assert!(times.contains(&750), "a key was inserted at the playhead");
        assert!((node(&s, n).opacity - static_before).abs() < 1e-6, "static value untouched");
        // Editing again at the same time updates that key, doesn't duplicate it.
        let three_quarters = serde_json::to_value(motion::AnimValue::Scalar(0.75)).unwrap();
        ex(&mut s, "motion.autoKey", json!({"property": "Opacity", "value": three_quarters, "time": 750, "ids": [n.0]}));
        let tr = node(&s, n).motion.track(AnimProperty::Opacity).unwrap();
        assert_eq!(tr.keys.iter().filter(|k| k.time_ms == 750).count(), 1, "same-time edit updates, no duplicate");
        assert!((tr.key_at(750).unwrap().value.scalar().unwrap() - 0.75).abs() < 1e-6);
    }

    /// 形状错误整条拒绝，一个字节都不写（与 motion.keyframe 的原子性一致）。
    #[test]
    fn auto_key_refuses_wrong_shape_without_writing() {
        let mut s = Session::new();
        let n = rect(&mut s);
        let before = node(&s, n).clone();
        // A well-formed AnimValue of the wrong kind: parse succeeds, `accepts` must refuse it.
        let scalar = serde_json::to_value(motion::AnimValue::Scalar(5.0)).unwrap();
        let err = s.execute("motion.autoKey", &json!({"property": "Position", "value": scalar, "ids": [n.0]}));
        assert!(err.is_err(), "scalar value cannot fit the Position (Vec2) property");
        assert_eq!(*node(&s, n), before, "refused write leaves the node untouched");
    }

    /// 造一个带两条 track 的层：Opacity 在 0（带 bezier ease）和 200 有 key，Position 在 0 有 key。
    fn layer_with_keys(s: &mut Session) -> figcraft_doc::NodeId {
        let n = rect(s);
        let ease = Ease::bezier(0.1, 0.2, 0.3, 0.4);
        let m = NodeMotion {
            tracks: vec![
                motion::Track {
                    property: AnimProperty::Opacity,
                    keys: vec![
                        motion::Keyframe { time_ms: 0, value: motion::AnimValue::Scalar(0.2), easing: ease, out_handle: None, in_handle: None },
                        motion::Keyframe { time_ms: 200, value: motion::AnimValue::Scalar(0.9), easing: Ease::default(), out_handle: None, in_handle: None },
                    ],
                    visible: true,
                },
                motion::Track {
                    property: AnimProperty::Position,
                    keys: vec![motion::Keyframe { time_ms: 0, value: motion::AnimValue::Vec2(Vec2::new(5.0, 6.0)), easing: Ease::default(), out_handle: None, in_handle: None }],
                    visible: true,
                },
            ],
            clips: vec![],
        };
        let motion_json = serde_json::to_value(m).unwrap();
        ex(s, "motion.replace", json!({"node": n.0, "motion": motion_json}));
        n
    }

    /// 帧复制：把 from=0 处**真实存在的 key**（含 value 与 easing）复制到 to=500，
    /// 不插值、不碰别的时刻，一条命令一个 undo 步。
    #[test]
    fn keyframe_copy_duplicates_real_keys_with_easing() {
        let mut s = Session::new();
        let n = layer_with_keys(&mut s);
        let src_ease = node(&s, n).motion.track(AnimProperty::Opacity).unwrap().key_at(0).unwrap().easing;
        let out = ex(&mut s, "motion.keyframeCopy", json!({"from": 0, "to": 500, "ids": [n.0]}));
        assert_eq!(out["copied"], 2, "Opacity + Position both have a key at 0");
        let op = node(&s, n).motion.track(AnimProperty::Opacity).unwrap();
        let pos = node(&s, n).motion.track(AnimProperty::Position).unwrap();
        let copied_op = op.key_at(500).unwrap();
        assert!((copied_op.value.scalar().unwrap() - 0.2).abs() < 1e-9, "value copied, not interpolated");
        assert_eq!(copied_op.easing, src_ease, "easing round-trips exactly");
        assert_eq!(pos.key_at(500).unwrap().value.vec2().unwrap(), Vec2::new(5.0, 6.0), "Position value copied");
        assert_eq!(op.key_at(200).unwrap().value.scalar().unwrap(), 0.9, "other-time key untouched");
        assert_eq!(op.keys.iter().filter(|k| k.time_ms == 500).count(), 1);
    }

    /// 只复制指定属性；且 from 处没有 key 的 track 不被凭空造帧。
    #[test]
    fn keyframe_copy_respects_property_filter_and_skips_missing_keys() {
        let mut s = Session::new();
        let n = layer_with_keys(&mut s);
        // 300 has no key on any track → copies nothing.
        let out = ex(&mut s, "motion.keyframeCopy", json!({"from": 300, "to": 700, "ids": [n.0]}));
        assert_eq!(out["copied"], 0);
        assert!(node(&s, n).motion.track(AnimProperty::Opacity).unwrap().key_at(700).is_none());
        // Filter to Opacity only → just 1 key copied at to=500, Position untouched.
        let out = ex(&mut s, "motion.keyframeCopy", json!({"from": 0, "to": 500, "ids": [n.0], "properties": ["Opacity"]}));
        assert_eq!(out["copied"], 1);
        assert!(node(&s, n).motion.track(AnimProperty::Opacity).unwrap().key_at(500).is_some());
        assert!(node(&s, n).motion.track(AnimProperty::Position).unwrap().key_at(500).is_none(), "filtered-out track not touched");
    }

}
