# Motion 面板（ui-gpui）对源码（ui-egui）审计清单

> 审计范围：`crates/ui-gpui/src/motion/*` 与 `crates/ui-gpui/src/panels/right/sections.rs`（Fill/Stroke/Effects 复用件）
> 对照基准：`crates/ui-egui/src/motion/{mod,inspector,timeline,clip_popover,easing}.rs` + `crates/ui-egui/src/panels/right/{paints,effects}.rs`
> 结论：**致命 + 高优级缺陷已全部修复；中优先级缺口已修复 Fill/Stroke 关键帧菱形 + clip 下拉菜单化；剩余为 effects 菱形（被更大的 effects 详情字段缺口阻塞，详见第 4 节）。**

---

## 1. 已修复（致命 / 高优先级）

| # | 严重度 | 现象 | 根因 | 修复 |
|---|--------|------|------|------|
| 0 | 🔴 致命（**真根因**） | 播放头/时长线/片段**依然拖不动**（点一下跳一次，但完全不跟手） | 拖拽的瞬时状态（`drag`/`grabbed`/`last_press`/`began`）存在 `bar()` 内部的**每帧新建 `Rc<RefCell<…>>`** 里；`on_mouse_down` 的 `q.push` → `wake` → `cx.notify` → 下一帧重跑 `bar()` 又 `Rc::new` 出空状态 → `on_mouse_move` 读到 `None` 直接 early return | 四个瞬时状态改存 `thread_local!`（时间轴单例），跨帧存活；`dispatch_down` 去掉 `grabbed` 参数直接写 `GRABBED` |
| 1 | 🔴 致命 | 时间轴全盘失灵：播放头拖不动、clip 拖不动、时间输入框输不进去 | gpui `MouseDownEvent.position` 是**窗口坐标**，但时间轴 bar 贴在窗口底部，直接用窗口坐标去匹配局部命中表 → 全部命中落空 | `timeline.rs` 在 canvas paint 闭包里捕获 `bar_origin`（`b.origin`），down/move 时 `pos = e.position - bar_origin` |
| 1b | 🟠 高 | 播放头圆头渲染成「一坨」 | `Painter::paint_circle` 用 `PathBuilder` 四次 `cubic_bezier_to` + `paint_path`，GPUI 0.2.1 下曲线路径填充不稳 | 改用「正方形 + 四角半径 = r」的 `paint_quad`（同形同尺寸，最可靠图元）；删死代码 `circle_path` |
| 1c | 🟠 高 | 右栏「几乎没有任何东西可以点」 | `KeyframeFields::new(cx, 0.0, …)` 用 `width=0.0` 建字段，而 `Input::new(width)` 用 `width` 定位 lead/edit/trail 插槽 → 菱形跑到负坐标、编辑区 0 宽 | 加 `KeyframeFields::set_col(col, cx)`，`inspector::panel` 用 `panel_w` 按 Design 同款公式算列宽后推入 |
| 2 | 🟠 高 | 每按一次就记一个空 undo | 原 `on_mouse_down` 无条件 `BeginEdit("Move animation")`，play/loop/fold/close 等普通点击也记空 | 加 `began` 标志，仅 Clip/Layer 拖拽 + 关键帧移动才 `BeginEdit`+`EndEdit` |
| 3 | 🟠 高 | 横向滚动后时间全错 | `time_at` 没带 `scroll_x`，而 egui 的 `time_of` 带 `scroll` | `time_at(x, scale, scroll_x)` 加 `scroll_x` 参数；`dispatch_down` 同步 |
| 4 | 🟠 高 | 点选 Segment（关键帧区间）打不开 Easing 面板 | `reconcile()` 只从 `SelItem::Key` 推导 `ui.selected` | 改为 `SelItem::Key \| SelItem::Segment` |
| 5 | 🟠 高 | Layer 拖拽落点漂移 | `LayerDrag` 的 `lo/hi` 用了整时间轴 `0..duration`，egui 用的是该 layer 的 span | 改用 `node.motion.span_ms(duration)` |

---

## 2. 本次新修复（中优先级）

| # | 现象 | 修复 |
|---|------|------|
| 6 | Fill / Stroke 行没有关键帧菱形（egui 在 `paints(..., Some(layer))` 给第一道实心 paint 加菱形） | `sections::paints` 新增 `keyframe_layer / keyed / motion_q` 三参；Motion host 计算 `fill_keyed`/`stroke_keyed`（`crate::motion::has_key`），第一道实心 paint 行尾追加菱形 `IconButton::Component/Instance`，点击 `Act::ToggleKey(layer, FillColor\|StrokeColor)`。Design 面板传 `None,false,&占位队列`，行为不变 |
| 7 | clip 设置浮窗的 Kind/Mode/Direction/Easing 是「点击循环切换」 | egui 是「菜单选」。改为每个下拉点击时 `root.update(cx, cv.open_menu(anchor, w, menu, on_pick, cx))`，菜单项对齐 egui（`ClipKind::ALL` / `[In,Out,Custom]` / `kind.directions()` / `Ease::PRESETS`） |

> 编译：0 error。Motion 单元测 6/6 通过（`commit_value_writes_key_at_playhead`、`clip_drag_never_accumulates_drift`、`preview_is_cached_by_doc_and_playhead`、`keyframe_add_remove_toggle_roundtrip`、`set_open_restores_the_right_width` 等）。

---

## 3. 验证状态

- ✅ `cargo check -p figcraft-ui-gpui`：0 error（39 个既有 warning，均与本改动无关，在非本次文件）。
- ✅ `cargo test -p figcraft-ui-gpui --lib motion`：6 passed / 0 failed。
- ⚠️ **未做的验证**：GPUI 窗口无法被 `SetCursorPos`/`mouse_event` 外部驱动，时间轴拖拽 / 浮窗交互的**真实鼠标手感**只能靠控制通道（`ui.inspect` / `engine.execute`，端口 7866）或 debug 预设验证，无法在此环境点一遍。坐标空间根因已通过源码确认 + 编译 + 单测锁定，但端到端拖拽仍需你在真机点一遍确认。

---

## 4. 仍为缺口 / 已知限制（诚实列出，未半吊子实现）

| # | 缺口 | 为什么没顺手修 | 计划 |
|---|------|----------------|------|
| 8 | **Effects（阴影/模糊）行的关键帧菱形缺失** | egui 的菱形挂在 shadow 的 X/Y/Blur/Color、blur 的详细字段上；但 gpui 的 `effects` 只渲染了「类型下拉 + 显隐 + 删除」，**根本没移植那些详细字段编辑 UI**。要加菱形得先移植整个 shadow/blur 字段编辑，是大缺口而非只差一个菱形。 | 单独排期：先移植 `effects` 的 shadow/blur 详细行（X/Y/Blur/Spread/Color…），再补菱形 |
| 9 | 菱形行**改色不回写动画** | egui 关键点 keyed 时，颜色文本编辑走 `commit_anim_value`（写动画关键帧），gpui 仍走静态 `fill.set`/`stroke.set`。本次只修了菱形的增删关键帧，颜色编辑仍是静态写回 | 在 `row_field` 的 `RowEditCtx` 增加 motion 回写通道，keyed 时改色→`Act::CommitValue`（Color）。需改共享 `RightFields`，需谨慎不殃及 Design 面板 |
| 10 | Motion `inspector::panel` 里 `FIELDS` 9–11（FillColor/StrokeColor/ShadowBlur）的 `KeyframeField` 实体**建了但从未渲染** | egui 不走独立字段，而是用第 6 项那种「共享 paint 行菱形」。这些孤儿实体被 `fields.sync` 每帧同步但不显示，属死代码 | 删掉 `FIELDS` 9–11 及相关 `get`/`sync` 分支（与 egui 对齐），或在审计确认后再清 |

---

## 5. 约束遵循情况

- ✅ 每行走统一网格（`field_row` / `field_row_full` / `field_row_full_trailing` → `PanelSection::child`），未手搓 flex 行。
- ✅ 尾列只用「一列 24 多图标」（菱形是第三枚 24pt 图标，沿用冻结文件的 `SLOT * n` 单列逻辑，未另加独立空列）。
- ✅ 浮窗复用 `Popover` 外壳，未自造窗口。
- ✅ `widgets/` 冻结区（含 `panel_section.rs`）**零改动**；本次改动全在 `motion/*` 与 `panels/right/sections.rs`（非冻结）。
- ✅ 带关键帧按钮的输入框仍从 `template::Input` 延伸；菱形图标复用 `Icon::Component`/`Icon::Instance`，未自绘图形。
