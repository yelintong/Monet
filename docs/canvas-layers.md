# 画布渲染方案（最终态）

> 本文只写**代码行为**和**实测数据**，不拿注释当论据。注释若与本文冲突，以代码为准。
> 关键约定：WGSL 材质不是「WGPU 旁路」，而是节点属性，管线为它保留正式合成阶段；
> 拖动**效果优先**——材质在拖动帧完整保留，不为帧率降级。

## 0. 结论

拖动期间**不冻结底图、不画局部补丁**：每帧照常提交整页渲染，`poll` 只取最新完成的那一张（`latest wins`）。被拖的层从底图里跳过，由一张**透明底替身**（`GrabAsk` → `canvas.drag_grab`）按当前包围盒偏移单独贴合；替身只在拖动开始时抓一次，之后每帧只改偏移，不产生光栅工作。

**材质在拖动帧同样完整合成，不降级。** 实测：

| 场景 | 拖动帧耗时 | 说明 |
|---|---|---|
| 无材质 | 3.9~7.2ms，`work=Partial` | 只重绘脏区（增量缓存），无残影 |
| 8 个 `glass.wgsl` 节点 | 端到端 ~33ms（约 30fps） | 与 1 个材质时**相同**，材质数不线性恶化 |
| 替身跟手 | 底图 skip 被拖层 + 替身按偏移贴合 | 拖出/拖回各只 1 份像素（无 4700 双份残影） |

代价：拖动中内容比选框晚约 1 帧。这是不冻结模型的固有代价；替身把它压到只剩「底图刷新」这一项。

## 1. 代码事实

| 事实 | 位置 |
|---|---|
| 曾有 `fn drag_holds(moving) -> bool { !moving.is_empty() }`，`request_render` 首行 `if drag_holds(moving) { return; }` → **拖动期间一个整页渲染都不提交** | `render.rs`（已删） |
| `poll()` 每帧排空通道、取 seq 最新的一张换贴图，这条路径一直存在，只是被那个 `return` 饿死 | `render.rs` |
| 原工程 `request_render` 没有任何拖动特判；worker 是 `latest wins`（`better()` 取 seq 更大者，旧结果丢弃） | `D:/Projektdateien/Monet/crates/ui-egui/src/canvas/` |
| 原工程有 `cached_render_covers`（96px overscan）与 `coarse_view`（降采样预览） | 同上 `render_worker.rs` |
| 补丁机制（`Live`/`start`/`patch_for`/`affected_root`/`drag_flow`）是为填补那行 `return` 的空档才引入的 | `canvas/mod.rs`（已删） |

推论：**「拖动没有画面更新」不是平台限制，是那行 `return` 造成的**。删掉它之后，补丁机制失去存在理由，一并删除。

## 2. 为什么这样对

补丁模型的正确性依赖一个前提：**每帧算准「旧位置 ∪ 新位置」的范围**。这个前提在容器 Hug 变宽又缩回、`moving` 集合中途变化、`start` 被重置、拖入/拖回 AutoLayout 导致兄弟位移时都会失效，失效的表现就是半块残影。

不冻结模型没有这个前提：屏幕上每一帧都是「当前文档的完整渲染」，不存在需要被擦掉的历史像素。残影没有产生的余地，不是被修好的。这是结构性消除，不是逐处打补丁。

## 3. 拖动替身（drag_visual）

能力本来就在，只是被补丁方案写死成空值，现已接回：

- `render_worker::Job` 早已有 `grab: Option<GrabAsk>` 与 `skip: Vec<NodeId>`；`grab_layer()` 用 `Options { background: TRANSPARENT, only: ids }` 单独渲一张透明底替身，并临时关掉祖先 `clip` 让层「浮出」；`JobKey` 也有 `grab_key()` 指纹。
- `canvas/mod.rs::show()`：刚进入拖动（`started`）构造 `GrabAsk { page: bounds ± 32, at: [x0,y0], ids }`，只抓一次；底图请求带 `skip = drag_ids`（底图不含被拖层）。
- `poll()` 里 `done.grab` 存入 `CanvasState.drag_grab`；`CanvasOutput.live` 通道按 `to_screen(grab.page) + (当前 bounds 左上 − grab.at)` 定位贴合。
- 松手帧 `take()` 掉替身并 push 进 `retired`，交给视图层 drop（GPUI sprite atlas 只显式 drop 才释放）。

## 4. 材质是一等公民（效果优先）

- 材质是节点属性；管线正式阶段在 `wgsl_runner::passes_for` → `wgsl_composite`。不用「旁路/外挂」描述它。
- `run()` 把绑了材质的节点**连同子树**从 CPU 底图挖空（`Options::skip`，不动 `job.skip`），因为 `glass.wgsl` 的 `sample_backdrop` 采样的是「本帧图像裁下来的那块背景」：节点自己若还在图里，玻璃会折射自己的填充色而不是背后画面。挖空后由 `apply_wgsl_materials` 在 GPU 上补回；GPU 补不回时用 CPU 兜底画回，绝不留空洞。
- **拖动帧同样挖空 + 合成**。`refine` / `refine_async` / `preview` / 复用帧 / GPU 场景路径都无条件走 `apply_wgsl_materials`，没有「拖动中跳过材质」的分支。
- **不能靠「拿旧结果平移」省成本**：`glass.wgsl` 片元输出 `vec4(col, coverage)` 虽带 alpha，但 `col` 已含 `sample_backdrop` 采到的背景，换透明目标折射就错；拿旧结果平移会把旧背景一起带走。
- **材质多不会崩**：`encode_submit` 把整条材质栈编码一次、submit 一次，只把 **material region**（`composite_region` 算出的并集矩形）拷回；不是整页上传、也不是整页回读。实测 1 个与 8 个 `glass` 的端到端都是 ~33ms；成本是「1 次 region readback + N 次 GPU draw」，不是「每材质一次同步等待」。

### 为什么拖动中不是 60fps（实测分解 + GPUI 约束）

**先纠正旧结论**：不是「GPU→CPU 整块读回」。代码事实（`crates/ui-gpui/src/wgsl_runner.rs`）：

- GPU 侧输入/输出都限于 **material region**：`encode_submit:979` 只把 `composite_region` 的并集矩形 `write_texture` 上去，`:1180` 也只 `copy_texture_to_buffer` 这一块；整页 push/pull 在 `:975-978` 被注释明确标为「pure waste」并删除。
- 唯一的**整页拷贝在 CPU 侧**：`splice_mapped:1254` 为把 region 拼回整页，`base[..w*h*4].to_vec()` 每帧分配+复制一次整页（1080p ≈ 8.3MB；2x 4.4MP ≈ 17.6MB），约 1~2ms/帧。
- 拖动**不阻塞**：`render_worker.rs:1521` 走 `refine_async → submit_async`（提交即返回，像素由 `poll_ready` 稍后收）；同步 `refine` 只在 `FIGCRAFT_GPU=1` 与 `render_now` 用。所以 ~33ms 端到端是「异步 readback 完成 + 下一轮 worker 调度」的延迟，不是 CPU 卡死。

**33ms 花在哪**：`[material]` 日志已把 `layers`（每材质节点的 children_layer 渲染）与 `composite`（上传+pass+读回+拼接）分开计时。历史实测 `composite` 3.7~7.5ms、8 个 glass 端到端 ~33ms，差额在 CPU 光栅（tiny-skia 整页）+ children_layer + 异步调度。本轮未重测（依赖 crate 当时不可编译），上述为历史实测值。

**GPUI 0.2.2 的硬约束（决定 #19 可行性）**：`RenderImage` 只吃 CPU 的 BGRA 内存（`gpui-0.2.2/src/assets.rs:42` `data: SmallVec<[Frame;1]>`，显示走 `AtlasTextureId`），**没有外部 wgpu 纹理入口**。所以「让 UI 直接持有 GPU 纹理、跳过 readback」在本版本**不 fork GPUI 做不到**——`paint_image`/`img` 都必须先有 CPU 字节。

**已实施的两步（不损失效果，纯「别做白工」）**：

1. **A1 — 被 skip 的层不算 damage**（`crates/render/src/retained.rs::patch`）。`Options::skip` 在 `draw.rs:567/1090/1129`、`lod.rs:206` 是遍历入口直接 `return` 的**递归剪枝**，被拖层连子树都不画；而 `patch` 原来只看 `changed_ids`、不看 skip，于是被拖层的「旧位置 ∪ 新位置」每帧都算成 damage → `Work::Partial` → 材质必须重合成。现在若某 changed id（或它的祖先）**在两帧的文档里都在 skip 子树内**，它对主图零像素影响，跳过它的 `changed_areas`。判据要求两帧都成立，节点被移动/换父时不会误判。效果：拖动稳态帧从 `Partial` 变 `Work::Reused`（回归测试 `retained_tests::moving_a_skipped_layer_leaves_the_page_reused`：改前 `Partial{pixels:7688}`，改后 `Reused`）。
2. **A2 — 输入没变就复用上一张合成图**（`render_worker::Materials::last_composite` + `composite_key`）。合成是 `(底图, passes)` 的纯函数：底图在 `Work::Reused` 时是**同一个 `Arc` 指针**（`run:346-363` 直接 `prev.clone()`），`composite_key` 把 passes 的几何/参数/`field_bytes`/uniform 字节 + 视图 + 尺寸哈希进去。两者都对得上就跳过 `children_layer` → GPU region 上传 → 合成 → readback → splice → `wrap_bgra`，把上一帧的 `Arc<RenderImage>` 原样交回；因为 `ImageId` 不变，GPUI atlas key 命中，**整页纹理重传也一并消失**。
   - `time` **故意不进哈希**：它只对**声明了** `time` uniform 的 shader 生效，那种 shader 会把它包进 `uniforms`，指纹每帧都变 → 自动退回重合成，动画不会被冻住；不声明 `time` 的 shader（如 `glass.wgsl`）字节不变 → 命中。
   - 三条通道都接了：同步 `try_apply_wgsl_materials`、异步 `try_submit_wgsl_materials`（新增 `Submit::Reused`）、异步收齐 `collect_ready`（把结果连同 base/key 存回缓存）。

于是带材质拖动的成本分层变成：**拖的不是材质节点**（最常见）→ 每帧只有 `passes_for` 一次遍历，画面直接复用，不再有 readback 与整页上传；**拖的正是材质节点**（它的 `rect_px` 变了，指纹必然变）→ 仍需每帧合成。

**A1 之前我先修了 `figcraft-render` 的 lib 测试**：该 crate 的测试在 HEAD 上就有 7 处编译错误（`crate::Begun` → `crate::retained::Begun`、`bounds_within/local_paint_bounds` 少了 `world` 参数、`draw::MAX_REACH` 已删），`lib.rs:641-646` 的注释记着这件事。不修就无法跑任何渲染层回归，A1 也就没有验证手段。修的是测试、不是生产语义。

**仍未做的**：`splice_mapped` 的整页 `to_vec`（`RenderImage` 要 owned `Frame`，交接所有权躲不掉）；`children_layer` 的内容级缓存（A2 命中时它根本不会被调用，只在真正重合成时才付）；fork GPUI 换外部纹理入口（Windows 后端是纯 DX11、我们的是 wgpu，见下）。

**什么时候仍需要 fork GPUI**：只有当**材质节点本身每帧都在变**（拖它、缩放、动画材质带 `time`）时，才必须每帧走一次「合成 → readback → 新 `RenderImage` → 整页重传」。要去掉这条链，只有两条路：fork GPUI 给 image 原语加外部纹理通道，或等上游支持。而 GPUI 0.2.2 的 Windows 后端是**纯 DX11**（`directx_renderer` / `directx_atlas`，`ID3D11Device` + `UpdateSubresource`），`gpui` 根本不依赖 wgpu；本项目的 `WgslRunner` 是 wgpu 30（DX12/Vulkan）。设备与 API 都不同，纹理不能直传——所以「不 fork 就让 UI 直持 GPU 纹理」在本版本不成立，fork 也不是加个入口，而是重写整个 Windows 后端或做 wgpu↔DX11 的 DXGI 共享。本文不承诺时间。历史上曾实现「拖动中跳过材质换 60fps」，因效果必须保证已全部撤销；A1/A2 是另一条路——**不降级效果，只是不再重复做已经做过的事**。

## 5. 校验记录

| # | 假设 | 方法 | 结果 |
|---|---|---|---|
| A1 | 原工程不冻结 | 读 `request_render` 有无拖动特判 | ✅ 无特判 |
| A2 | 删掉 `drag_holds` 后拖动每帧提交 | 拖动时看 `[canvas] refine seq=` 是否连续 | ✅ seq 3→8 连续 |
| A3 | `shows()` 不拒绝拖动中的图 | refine 图无 seq 门槛 | ✅ 每张都上台 |
| A4 | 整页渲染撑得住拖动 | 读 `[render]` 耗时 | ✅ 3.9~7.2ms，`work=Partial` |
| A5 | 无残影 | 拖出中间项，按颜色统计像素（1 份≈2350，2 份≈4700） | ✅ 2400/2350，无 4700 |
| A6 | 材质节点每帧重跑不拖垮 | 绑 `glass.wgsl` 到 1 个和 8 个节点，量 `[material]` 与端到端 | ✅ 8 个 `passes=8 drawn=8/8 composite=3.7~7.5ms`，端到端 ~33ms；拖动帧保留材质 |
| A7 | 启动期 `overflowed its stack` | 5 次启动 1 次，定位为 UI 线程递归渲染 | ✅ 已修（`on_render_stack`，5 处 8MB scoped 线程，借用 `&Document` 不 clone） |
| A8 | 被 skip 的层移动不该算 damage | `retained_tests::moving_a_skipped_layer_leaves_the_page_reused`：同 view、同 skip，只挪被 skip 层 | ✅ 改前 `Partial{pixels:7688}`，改后 `Reused`；全 crate **219 passed / 19 failed**，19 条与改前基线**完全同一批**（effect reach/spread/glass 的 cut-back 既有失败），零新增 |
| A9 | 底图与 passes 都没变时可复用合成图 | 隔离 worktree 编译 + `render_worker::a_drag_frame_hands_back_the_same_composited_picture`（拖动一层后第二帧必须 `Ready` 且 `Arc::ptr_eq` 第一帧的合成图） | ✅ 编译 0 error；`canvas::render_worker` **22 passed / 0 failed**（21 旧 + 1 新）；`canvas::` **57 passed / 0 failed** |

A7 不在拖动路径上，也不是本次改动引入的代码位置，但必须单独排查，不能当它不存在。偶发问题单次未复现不算证明；但根因已从「每条 UI 渲染路径」消除（`on_render_stack` 统一包住缩略图/预览的 5 处递归调用），属结构性修复。

## 6. 改动规模

- `canvas/mod.rs`：817 → 358 行（删补丁模型约 −500 行；接回 `drag_grab` 替身约 +42 行）。
- 移除：`Live`、`CanvasState.live`、`drag_flow`、`patch_for`、`patch_rect`、`MAX_PATCH_PIXELS`、`affected_root`、`flow_homes`、`live_clip`、`drag_holds` 及配套 11 项测试。
- 新增：`render_stack.rs::on_render_stack`（A7）；`canvas` 内 `drag_grab` / `GrabAsk` 替身接线。
- 保留：`render_worker::render_patch` / `patch_job`（HEAD 自带的同步局部渲染入口，供 `render_now` 与「局部 view 下材质子节点可见」回归测试用；**生产拖动不走它**）。
- 拖动状态只剩 `dragging_now` / `released`（松手作废渲染指纹）。
- A1：`crates/render/src/retained.rs::patch` 加「两帧都在 skip 子树内 → 不算 damage」；`retained_tests.rs` 加回归测试 + 修 7 处 HEAD 遗留的测试编译错误（`crate::Begun` 路径、`bounds_within` 参数、`MAX_REACH` 语义）。
- A2：`crates/ui-gpui/src/canvas/render_worker.rs` 加 `LastComposite` / `composite_key` / `reuse_composite`，`Submit` 三态替代 `Result<u64,()>`，`PendingFrame` 与 `RefineOutcome::Submitted` 各加 `key`；`render_worker` 加回归测试 `a_drag_frame_hands_back_the_same_composited_picture`。

### 验证（本次，E 盘 target / `--offline`）

| 命令 | 结果 |
|---|---|
| `cargo test --offline -p figcraft-render --lib`（**改前基线**） | 218 passed / 19 failed（19 条是 HEAD 既有的 cut-back 语义失败，与本方案无关） |
| `cargo test --offline -p figcraft-render --lib`（A1 之后） | **219 passed / 19 failed**——新增 1 条，失败集与基线**逐条相同**，零回归 |
| `cargo check --offline -p figcraft-ui-gpui --lib --features "test-support,gpu"` | **0 error** |
| `cargo test --offline -p figcraft-ui-gpui --lib --features "test-support,gpu" canvas::render_worker` | **22 passed / 0 failed**（21 旧 + A2 新测） |
| `cargo test --offline -p figcraft-ui-gpui --lib --features "test-support,gpu" canvas::` | **57 passed / 0 failed** |

`render_worker` 的 22 条覆盖效果优先最该守的回归：材质在 refine/preview 中确实出现、异步与同步精修逐字节一致、GPU 不可用时 CPU 兜底不留空洞、局部 view 下材质容器子节点可见，以及新增的「拖动帧复用同一张合成图」。测试机无 adapter 也能过（`missing.wgsl` 恒走失败分支）。

**`ui-gpui` 的三道是在隔离 worktree 里跑的**：共享工作区当时被另一会话改到一半的 `panels/right/sections.rs` 卡住（新增 6 参 `local_style_folder`，调用点没跟上；另一处 `key` 在 `Fn` 闭包里被 move），与本次改动无关。做法 `git worktree add --detach … HEAD` + 只拷入本次改动的 6 个文件，`CARGO_TARGET_DIR` 指向原 target 复用第三方依赖——**共享工作区一个字节没碰**。

## 7. 环境事实（复用，别再踩）

- GPUI 0.2.2：`Quad` 无矩阵；`MonochromeSprite.transformation` 有 `TransformationMatrix`，`PolychromeSprite` 没有；`paint_image` 做 `floor/ceil` 整数对齐。画布内容不是图片，不能降成整页位图再论证「无法仿射」——顶点级变换走 `to_device_position_transformed`。
- 验证用 `--offline`，`cargo test -E` 本机不支持（unexpected argument），用全量或单 NAME 过滤。
- 该 worktree 有另一会话并行改约 36 个文件；重编会把它一起编进去，验证结论必须说明这一点。测试基线里若见与本方案无关文件（motion/widgets/fonts/layout 等）的失败，按既有基线处理，别误判成本次回归。
