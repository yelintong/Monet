# 动画 Gizmo + 矢量路径编辑：完整评估与修复方案

> 2026-11-12。本文只写**代码事实**（文件:行）与**分批执行计划**，不写推测。
> 目的：停止"冒一个改一个"，先把根因、边界、验收标准钉死，再整体修。

---

## 0. 结论摘要（先看这段）

| 问题 | 性质 | 根因（一句话） |
|---|---|---|
| 动画 gizmo 热区偏 / 轨迹跟着动 / 设计角串进动画 / 播放头在中间选不中 | **同一个根因** | 渲染期 `ds.doc` 被临时换成动画求值后的 preview 文档，而命中/轨迹/角度读的是同一份被换掉的文档，两边坐标系不同源 |
| 拖不动 / 整体也拖不动 / 往下跳乱跑 | **我上一轮改坏的** | 我给中心菱形加了拖动 + 改了 `input()` 事件所有权，已全部撤销 |
| 白环碎成一段段 | 已修（保留） | `paint_circle` 逐段画矩形、无 join → 已改闭合 stroke 路径 |
| 白边宽度随 hover 变 | 已修（保留） | 白色 5pt 圆环与蓝环叠在同一半径 → 已改恒定宽度白描边 |
| 动画里角度不归零 | 已修（待验证） | 右栏 Rotation 直接显示引擎绝对角（含设计角）→ `value_of` 减、`commit_value` 加回 |
| **矢量路径编辑手柄全丢** | **真缺口（P0）** | 引擎 `vectoredit` 完整（5524 行），GPUI 只接了右栏数据 + 区域斜线，**画布绘制/命中/指针/进入编辑全没接** |
| **动画路径要能像矢量路径一样编辑（Ctrl 拉曲线）** | **新功能（P1）** | egui 的 `motion/` 里**没有**画布运动路径编辑（只有 clip_popover/easing/inspector/timeline）；`motion_overlay.rs` 是 GPUI 新写的，无 egui 参照 |

---

## 1. 代码事实（核查证据）

### 1.1 矢量编辑：引擎完整，GPUI 只接了 30%

| 层 | 文件 | 状态 |
|---|---|---|
| 引擎 API | `crates/engine/src/canvas/vectoredit.rs`（1306 行） | ✅ `begin/begin_many/end/editing/set_tool/hit/hit_any/overlay/key/escape/selected_points/paint_all/enter_targets/network/edit/refit` |
| 引擎算法 | `vectoredit/{ops,pointer,transform,width,regions}.rs`（5524 行） | ✅ 含 `set_handle`（`:85`）、`move_handles`（`:118`）、`set_mirroring`（`:128`）、`bend`（`:171` 拉对称手柄） |
| 指针编辑 | `vectoredit/pointer.rs:201` `Session::vector_pointer`（`pub(crate)`） | ⚠️ 引擎内部方法，由 `tools/pointer.rs:135` 调用；GPUI 必须走引擎的 pointer 入口，不能直接调 |
| **Ctrl→Bend** | `vectoredit/pointer.rs:567` | ✅ `let tool = if ev.cmd && ve.tool == VTool::Move { VTool::Bend } else { ve.tool };` —— **这就是用户说的"按住 Ctrl 拉出控制柄"** |
| egui 绘制 | `ui-egui/src/canvas/overlays.rs:495-545` `vector_overlay()` | ✅ 贝塞尔段 + 方形点 + 手柄线（`bezier_handle`），hover 放大、选中 accent |
| egui 调用 | `ui-egui/src/canvas/overlays.rs:154-175` + `handles.rs:51` | ✅ 画点/段/手柄 + 多点变换框 + 区域斜线 |
| **GPUI 现状** | `ui-gpui/src/canvas/overlays.rs:168` | ❌ 只有 `region_hatch`（区域斜线） |
| | `ui-gpui/src/panels/right/data.rs:1033` `read_vector` | ✅ 右栏数据层已接 |
| | `ui-gpui/src/canvas/handles.rs:160` | ❌ 只是把 `vector_edit.is_some()` 排除在自动布局手柄外 |
| | `ui-gpui/src/lib.rs` | ❌ **只有 1598/2263 两处读 `vector_edit`**，没有 `begin/end/overlay/hit/key` 任何调用 |

**结论**：`ds.vector_edit` 在 GPUI 里**永远不会被置位**（没人调 `vectoredit::begin`），所以引擎的 `vector_pointer` 分支永远不触发，画布上也就什么都不画。这是"手柄全丢"的真正原因——不是画得不好，是**根本没进入矢量编辑模式**。

### 1.2 动画路径：egui 没有，属新功能

- `crates/ui-egui/src/motion/` 只有 `clip_popover.rs / easing.rs / inspector.rs / mod.rs / timeline.rs`。
- **没有**画布运动路径、没有 gizmo、没有路径编辑。
- `crates/ui-gpui/src/canvas/motion_overlay.rs` 注释说"移植自 desigma"，但**本仓库没有 desigma crate**（`ls -d crates/desigma*` 无结果）。

**结论**：用户要求的"动画路径像矢量路径一样编辑（2 点直线、Ctrl 拉曲线）"**没有 egui 可抄**，必须按矢量编辑的交互模式新做。好消息是引擎的 `VectorNetwork` 能力（`set_handle/bend/HandleMirroring`）可以直接复用。

### 1.3 动画 gizmo 的根因链

`lib.rs:5608-5626`：

```rust
let saved_doc = self.motion_preview_doc().and_then(|preview| {
    let ds = self.app.session.ds_mut().ok()?;
    Some(std::mem::replace(&mut ds.doc, preview))   // ← 渲染期把 ds.doc 换成求值后文档
});
let CanvasOutput { ... } = canvas::show(...);
if let Some(real) = saved_doc { ds.doc = real; }    // ← 画完立刻还原
```

于是：

| 场景 | 读到的文档 | 后果 |
|---|---|---|
| `draw()` 画 gizmo（在 `canvas::show` 内） | preview（求值后） | 角度基准 = 设计角 + 动画增量 |
| 鼠标 `input()` 命中（事件回调里） | 静态文档 | 角度基准 = 设计角 |
| 两者相减 | 不一致 | **箭头朝向不同 → 热区偏；`delta_deg` 非 0 → 角度不归零** |
| 轨迹 `parent_world(doc)` | preview | **线条跟着手柄/播放头漂** |
| 画布选中命中 | 静态文档 vs 画面 preview | **播放头在中间时点不中** |

已做（保留）：`State.src` 静态源文档 + `sync_motion` 在替换前传入 + `geos()/draw()` 统一用 `st.src`。
待做：**画布选中命中也要用 preview 文档**（否则播放头中间永远选不中）。

---

## 2. 分批执行计划

### 批次 A（P0，先让动画可用）—— 预计 1 天

| # | 任务 | 文件 | 验收 |
|---|---|---|---|
| A1 | 画布选中命中改用"当前显示文档"（preview），与渲染同源。**⚠️ 不能简单临时替换 `ds.doc`**：`move_down` 会在 Down 后进入拖动并写文档，写进 preview 副本会丢失。可行修法二选一：① UI 层在 Motion 非播放时拦 Down，用 preview 做命中 → `set_selection` → 不把 Down 交给引擎（放弃该模式下的画布拖动，本来也该由 gizmo 接管）；② 引擎新增公开的"按指定文档命中图层"API（更干净，但要动引擎） | `ui-gpui/src/lib.rs::send`、`engine/src/canvas/tools/click.rs` | 播放头放中间仍能点中图层；`ds.doc` 未被污染（revision 不变）；新增单测锁住 |
| A2 | 验证角度归零（`value_of` 减 / `commit_value` 加回）在真实引擎下成对 | `motion/mod.rs` | 单测：设计旋转 45° + 无动画 → 右栏显示 0；输入 30 → 关键帧存 75；播放到该帧显示 30 |
| A3 | 热区与绘制同源回归测试（箭头/块/环/菱形在 `st.src` 下命中） | `motion_overlay.rs` | 已有 14 条测试 + 新增"播放头中间命中"用例 |
| A4 | 白描边 hover 不变量（已写测试 `hovering_the_ring_thickens_only_the_blue_band`） | 同上 | 通过 |

**不做**：中心菱形拖动（已撤销，等批次 C 用正确方式重做）。

### 批次 B（P0，矢量路径编辑 1:1 移植）—— 预计 2–3 天

照 `ui-egui` 补 GPUI 侧，**只调用引擎 `vectoredit` 公开 API，不自己发明**。

| # | 任务 | 参照 | 验收 |
|---|---|---|---|
| B1 | 进入/退出矢量编辑：双击路径 / Enter 键 → `vectoredit::begin` / `enter_targets` / `end` | `ui-egui/src/shortcuts.rs:638`、`engine/tools/pointer.rs:72-78` | 双击路径后 `ds.vector_edit` 被置位，右栏出现 Points 节 |
| B2 | 画布绘制：贝塞尔段 + 方形点 + 手柄线 + hover 放大 + 选中 accent | `ui-egui/src/canvas/overlays.rs:495-545` `vector_overlay()` | 与 egui 像素级对齐（点 7/9pt、手柄 3.5/4.5pt、段 1.5/2.5/3pt） |
| B3 | ~~指针编辑接入~~ **已天然可用，无需接线** | `lib.rs:2968` 的 `send()` 已调 `session.pointer(&ev)`；`engine/tools/pointer.rs:135` 在 `ds.vector_edit.is_some()` 时自动进 `vector_pointer` | 置位 `vector_edit` 后拖点/拖手柄/框选/加删点应自动可用（B1 完成后即可验证） |
| B4 | **Ctrl 拉曲线**：`VTool::Bend`（`pointer.rs:567`）+ `Cursor::Pen(PenCursor::Bend)` | 同上 | 按住 Ctrl 拖线段 → 拉出对称手柄，两点之间从直线变曲线 |
| B5 | 右栏 Points 节接线（数据层已有 `read_vector`） | `ui-egui/src/panels/vector_panel.rs` | 选中点后右栏显示 X/Y、镜像模式、加入/删除点 |
| B6 | 区域斜线保留（已有 `region_hatch`） | — | 不回归 |

**前置已确认（2026-11-12）**：`ui-gpui/src/lib.rs:2968` 的 `send()` 已经调用
`self.app.session.pointer(&ev)`，而 `engine/src/canvas/tools/pointer.rs:135` 会在
`ds.vector_edit.is_some()` 时自动进入 `vector_pointer`。**所以指针编辑链路是通的**，
批次 B 的真实缺口只有三块：① 进入/退出编辑（`begin/end`）② 画布绘制（`overlay`）
③ 右栏 Points 接线。`Session::vector_pointer` 虽是 `pub(crate)`，但由引擎内部转发，
GPUI 不需要直接调。

### 批次 C（P1，动画路径可编辑）—— 预计 2 天

无 egui 参照，按矢量编辑的交互模式新做：

| # | 任务 | 做法 |
|---|---|---|
| C1 | 运动路径从"采样折线"改成"可编辑贝塞尔链" | 用 `VectorNetwork` 承载路径（点 = 关键帧位置，手柄 = 缓动切线） |
| C2 | 2 点之间默认直线，Ctrl 拉出控制柄 | 复用 `vectoredit::ops::bend` / `set_handle` / `HandleMirroring::Angle` |
| C3 | 拖手柄 → 写回关键帧的缓动（不是改 Position 值） | 需要引擎新增命令或复用 `motion.keyframeEase` |
| C4 | 中心菱形拖动改动画 Position（这次用正确方式） | 先读"当前显示位置"，再按增量写，避免第一次写入跳变 |

**注意**：C3 涉及"路径形状 ↔ 缓动"的语义映射，**必须先确认产品语义**（改曲线是改缓动，还是改路径本身？）。这一条要你拍板，我不擅自决定。

### 批次 D（P1，PR #3 审查 6 条）—— 预计 0.5 天

| # | 问题 | 修法 |
|---|---|---|
| D1 | `style.create` 无选中层时静默失败 | 照 egui `new_style_params`（`style_picker.rs:1643`）补 `value` 默认值 |
| D2 | fold/rename 改状态后不刷新 | 补 `cmds.wake()`（`panels.rs:240` 已有） |
| D3 | `local_style_renaming` 只存 path | 改成存 `(type, path)` |
| D4 | Escape / 原名不变时状态残留 | 用 `TextEdit` 生命周期清理 |
| D5 | edit_rows 每行 O(N²) | 列表打开时才构建 |
| D6 | "Edit style" 用 apply 列表冒充编辑器 | 暂时移除该 action，等真编辑器 |

### 批次 E（P2，第 3 批剩余）—— 按原清单

1. A2/B3/A27 画板：计时器/投票/连线（`board_facilitate.rs` 340 行 + `board_connect.rs` 255 行，引擎命令齐全）
2. A5 图标面板补全（拖拽排序/改名/改码位/导出字体）
3. A8/A12/A13、A15/A10/A17/A11（A19 对话框补真字段）
4. A26 原型画布连线（`proto_canvas.rs` 341 行 + `overlay_ghost.rs` 283 行）

---

## 3. 执行纪律（避免重蹈覆辙）

1. **每批单独分支 + 单独 PR**，不混提交。
2. **改前备份 + 改后 md5 核对**（工作树有并行 WIP，IDE 可能覆盖）。
3. **同一文件多处改动串行 Edit**，绝不并行。
4. **每批必须跑** `cargo test -p figcraft-ui-gpui --lib --features test-support <模块>`，并把结果写进 PR 描述。
5. **不碰引擎语义**（如 Rotation 绝对角）；UI 层做 0 基准转换必须成对。
6. **没把握就不改**，先出只读核查结论。
