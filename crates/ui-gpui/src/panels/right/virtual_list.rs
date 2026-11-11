//! Virtual list.
//!
//! Renders only the items intersecting the scroll viewport, so a table with
//! thousands of differently-sized rows stays cheap. Unlike gpui's `uniform_list`
//! (which assumes equal item sizes), each item may have its own size along the
//! list axis; per-item origins are computed as prefix sums of the item sizes
//! (gap included) and the visible range is found with two binary searches.
//!
//! `Axis` only exposes `invert()` in this gpui version, so a local `AxisExt`
//! (`is_vertical` / `is_horizontal`) is supplied.
//! - `DeferredScrollToItem` / `ListSizingBehavior` / `ScrollStrategy` exist, but
//!   the corresponding scrollbar / interactive-element traits do not, so those
//!   impls are dropped.
//! - `smallvec` is not a dependency of this crate, so `Vec<AnyElement>` replaces
//!   `SmallVec<[AnyElement; 64]>`.
//! - The original builder requires `Entity<V: Render>`; ours takes a plain
//!   `'static` closure instead, because the variables editor is a plain struct
//!   (not a GPUI `Entity`/`Render` view) and its row renderer must read retained
//!   caches out of a `Rc<RefCell<..>>`.

use std::cell::RefCell;
use std::ops::{Deref, Range};
use std::rc::Rc;

use gpui::{
    Along, AnyElement, App, AvailableSpace, Axis, Bounds, ContentMask, DeferredScrollToItem,
    Div, Element, ElementId, GlobalElementId, Half, Hitbox, InspectorElementId, InteractiveElement,
    IntoElement, IsZero, LayoutId, ListSizingBehavior, Pixels, Point, ScrollHandle, ScrollStrategy,
    StatefulInteractiveElement,
    Size, Stateful, Styled, StyleRefinement, Window, div, point, px, size,
};

/// gpui 0.2.2's `Axis` only exposes `invert()`; the virtual list relies
/// on `is_vertical()` / `is_horizontal()`, which are not exposed here.
/// Re-declare them locally so this implementation compiles unchanged.
pub trait AxisExt {
    fn is_vertical(&self) -> bool;
    fn is_horizontal(&self) -> bool;
}

impl AxisExt for Axis {
    fn is_vertical(&self) -> bool {
        matches!(self, Axis::Vertical)
    }
    fn is_horizontal(&self) -> bool {
        matches!(self, Axis::Horizontal)
    }
}

struct VirtualListScrollHandleState {
    axis: Axis,
    items_count: usize,
    /// Content bounds size (excluding padding and border) from the last frame.
    last_content_size: Option<Size<Pixels>>,
    deferred_scroll_to_item: Option<DeferredScrollToItem>,
}

/// A scroll handle for [`VirtualList`]. See also [`ScrollHandle`].
#[derive(Clone)]
pub struct VirtualListScrollHandle {
    state: Rc<RefCell<VirtualListScrollHandleState>>,
    base_handle: ScrollHandle,
}

impl From<ScrollHandle> for VirtualListScrollHandle {
    fn from(handle: ScrollHandle) -> Self {
        let mut this = VirtualListScrollHandle::new();
        this.base_handle = handle;
        this
    }
}

impl AsRef<ScrollHandle> for VirtualListScrollHandle {
    fn as_ref(&self) -> &ScrollHandle {
        &self.base_handle
    }
}

impl Deref for VirtualListScrollHandle {
    type Target = ScrollHandle;

    fn deref(&self) -> &Self::Target {
        &self.base_handle
    }
}

impl Default for VirtualListScrollHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl VirtualListScrollHandle {
    /// Create a new `VirtualListScrollHandle`.
    pub fn new() -> Self {
        VirtualListScrollHandle {
            state: Rc::new(RefCell::new(VirtualListScrollHandleState {
                axis: Axis::Vertical,
                items_count: 0,
                last_content_size: None,
                deferred_scroll_to_item: None,
            })),
            base_handle: ScrollHandle::default(),
        }
    }

    /// Get the base scroll handle.
    pub fn base_handle(&self) -> &ScrollHandle {
        &self.base_handle
    }

    /// Scroll to the item at the given index.
    pub fn scroll_to_item(&self, ix: usize, strategy: ScrollStrategy) {
        self.scroll_to_item_with_offset(ix, strategy, 0);
    }

    /// Scroll to the item at the given index, with an additional offset in items.
    fn scroll_to_item_with_offset(&self, ix: usize, strategy: ScrollStrategy, offset: usize) {
        let mut state = self.state.borrow_mut();
        state.deferred_scroll_to_item = Some(DeferredScrollToItem {
            item_index: ix,
            strategy,
            offset,
            scroll_strict: false,
        });
    }

    /// Scrolls to the bottom of the list.
    pub fn scroll_to_bottom(&self) {
        let items_count = self.state.borrow().items_count;
        self.scroll_to_item(items_count.saturating_sub(1), ScrollStrategy::Top);
    }
}

/// Create a [`VirtualList`] in the vertical direction.
///
/// `item_sizes` is the size of each row; only the `height` is used. The
/// cross-axis width is inferred by measuring the item at `item_to_measure_index`
/// (default 0).
///
/// `scroll_handle` must be persisted by the caller across frames (e.g. stored in
/// the editor state) so the scroll position survives re-renders; the closure is
/// `'static` and receives the visible `Range<usize>` plus the frame's `Window`
/// and `App`.
pub fn v_virtual_list(
    id: impl Into<ElementId>,
    scroll_handle: &VirtualListScrollHandle,
    item_sizes: Rc<Vec<Size<Pixels>>>,
    f: impl 'static + Fn(Range<usize>, &mut Window, &mut App) -> Vec<AnyElement>,
) -> VirtualList {
    virtual_list(id, Axis::Vertical, scroll_handle, item_sizes, f)
}

/// Create a [`VirtualList`] in the horizontal direction.
pub fn h_virtual_list(
    id: impl Into<ElementId>,
    scroll_handle: &VirtualListScrollHandle,
    item_sizes: Rc<Vec<Size<Pixels>>>,
    f: impl 'static + Fn(Range<usize>, &mut Window, &mut App) -> Vec<AnyElement>,
) -> VirtualList {
    virtual_list(id, Axis::Horizontal, scroll_handle, item_sizes, f)
}

fn virtual_list(
    id: impl Into<ElementId>,
    axis: Axis,
    scroll_handle: &VirtualListScrollHandle,
    item_sizes: Rc<Vec<Size<Pixels>>>,
    f: impl 'static + Fn(Range<usize>, &mut Window, &mut App) -> Vec<AnyElement>,
) -> VirtualList {
    let id: ElementId = id.into();
    let render_items: Box<
        dyn for<'a> Fn(Range<usize>, &'a mut Window, &'a mut App) -> Vec<AnyElement>,
    > = Box::new(f);

    let mut base = div().id(id.clone()).size_full().track_scroll(scroll_handle);
    if axis.is_vertical() {
        base = base.overflow_y_scroll();
    } else {
        base = base.overflow_x_scroll();
    }

    VirtualList {
        id,
        axis,
        base,
        scroll_handle: scroll_handle.clone(),
        items_count: item_sizes.len(),
        item_sizes,
        render_items,
        sizing_behavior: ListSizingBehavior::default(),
        item_to_measure_index: 0,
    }
}

/// Virtual list component for rendering a large number of differently sized items.
pub struct VirtualList {
    id: ElementId,
    axis: Axis,
    /// Underlying element. `Div::id` (via `InteractiveElement`) returns `Stateful<Div>` in
    /// gpui 0.2.1, so the held type is `Stateful<Div>`, not `Div`.
    base: Stateful<Div>,
    scroll_handle: VirtualListScrollHandle,
    items_count: usize,
    item_sizes: Rc<Vec<Size<Pixels>>>,
    render_items: Box<
        dyn for<'a> Fn(Range<usize>, &'a mut Window, &'a mut App) -> Vec<AnyElement>,
    >,
    sizing_behavior: ListSizingBehavior,
    item_to_measure_index: usize,
}

impl Styled for VirtualList {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl IntoElement for VirtualList {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl VirtualList {
    /// Bind this list to an externally-owned scroll handle (e.g. one shared with
    /// a table header). Clones the handle, so scroll state is preserved.
    pub fn track_scroll(mut self, scroll_handle: &VirtualListScrollHandle) -> Self {
        self.base = self.base.track_scroll(scroll_handle);
        self.scroll_handle = scroll_handle.clone();
        self
    }

    /// Set the sizing behavior for the list.
    pub fn with_sizing_behavior(mut self, behavior: ListSizingBehavior) -> Self {
        self.sizing_behavior = behavior;
        self
    }

    /// Set the item index used to infer the list's cross-axis size.
    pub fn with_item_to_measure_index(mut self, index: usize) -> Self {
        self.item_to_measure_index = index;
        self
    }

    fn scroll_to_deferred_item(
        &self,
        scroll_offset: Point<Pixels>,
        size_layout: &ItemSizeLayout,
        content_bounds: &Bounds<Pixels>,
        scroll_to_item: DeferredScrollToItem,
    ) -> Point<Pixels> {
        let Some(bounds) = size_layout.item_bounds(
            scroll_to_item.item_index + scroll_to_item.offset,
            self.axis,
            content_bounds,
        ) else {
            return scroll_offset;
        };

        let mut scroll_offset = scroll_offset;
        match scroll_to_item.strategy {
            ScrollStrategy::Center => {
                if self.axis.is_vertical() {
                    scroll_offset.y = content_bounds.top() + content_bounds.size.height.half()
                        - bounds.top()
                        - bounds.size.height.half();
                } else {
                    scroll_offset.x = content_bounds.left() + content_bounds.size.width.half()
                        - bounds.left()
                        - bounds.size.width.half();
                }
            }
            _ => {
                // Ref: gpui's uniform_list scroll-to logic.
                if self.axis.is_vertical() {
                    if bounds.top() + scroll_offset.y < content_bounds.top() {
                        scroll_offset.y = content_bounds.top() - bounds.top();
                    } else if bounds.bottom() + scroll_offset.y > content_bounds.bottom() {
                        scroll_offset.y = content_bounds.bottom() - bounds.bottom();
                    }
                } else {
                    if bounds.left() + scroll_offset.x < content_bounds.left() {
                        scroll_offset.x = content_bounds.left() - bounds.left();
                    } else if bounds.right() + scroll_offset.x > content_bounds.right() {
                        scroll_offset.x = content_bounds.right() - bounds.right();
                    }
                }
            }
        }
        self.scroll_handle.set_offset(scroll_offset);
        scroll_offset
    }

    /// Ref from gpui's uniform_list: measure the item at `item_to_measure_index`
    /// to infer the list's cross-axis size.
    fn measure_item(
        &self,
        list_width: Option<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Size<Pixels> {
        if self.items_count == 0 {
            return Size::default();
        }

        let item_ix = self.item_to_measure_index.min(self.items_count - 1);
        let mut items = (self.render_items)(item_ix..item_ix + 1, window, cx);
        let Some(mut item_to_measure) = items.pop() else {
            return Size::default();
        };
        let available_space = size(
            list_width.map_or(AvailableSpace::MinContent, |width| {
                AvailableSpace::Definite(width)
            }),
            AvailableSpace::MinContent,
        );
        item_to_measure.layout_as_root(available_space, window, cx)
    }
}

/// Frame state used by [`VirtualList`].
pub struct VirtualListFrameState {
    /// Visible items to be painted.
    items: Vec<AnyElement>,
    size_layout: ItemSizeLayout,
}

/// Per-item sizes along the list axis, gap included, and their prefix sums.
#[derive(Default, Clone)]
pub struct ItemSizeLayout {
    items_sizes: Rc<Vec<Size<Pixels>>>,
    axis_and_gap: Option<(Axis, Pixels)>,
    content_size: Size<Pixels>,
    sizes: Rc<[Pixels]>,
    origins: Rc<[Pixels]>,
    last_layout_bounds: Bounds<Pixels>,
}

impl ItemSizeLayout {
    /// The bounds of item `ix` in the list's content space: offset from the
    /// content origin along the list axis, filling it on the other.
    fn item_bounds(
        &self,
        ix: usize,
        axis: Axis,
        content_bounds: &Bounds<Pixels>,
    ) -> Option<Bounds<Pixels>> {
        let origin = *self.origins.get(ix)?;
        let item_size = self.sizes[ix];
        Some(match axis {
            Axis::Horizontal => Bounds {
                origin: point(content_bounds.left() + origin, px(0.0)),
                size: size(item_size, content_bounds.size.height),
            },
            Axis::Vertical => Bounds {
                origin: point(px(0.0), content_bounds.top() + origin),
                size: size(content_bounds.size.width, item_size),
            },
        })
    }
}

/// The items that intersect `viewport` along the list axis, plus one item of
/// overdraw past its end, given each item's origin and size (gap included).
///
/// Both edges are binary searches.
fn visible_range(origins: &[Pixels], sizes: &[Pixels], viewport: Range<Pixels>) -> Range<usize> {
    let count = origins.len();
    // Item ends grow with the index, so the first item ending past `edge` is a
    // partition point over the indices.
    let ends_before = |edge: Pixels| {
        let mut low = 0;
        let mut high = count;
        while low < high {
            let mid = low + (high - low) / 2;
            if origins[mid] + sizes[mid] <= edge {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        low
    };
    let first = ends_before(viewport.start);
    let past_end = ends_before(viewport.end);
    let last = if past_end == count {
        count
    } else {
        (past_end + 2).min(count)
    };
    first..last.max(first)
}

impl Element for VirtualList {
    type RequestLayoutState = VirtualListFrameState;
    type PrepaintState = Option<Hitbox>;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, VirtualListFrameState) {
        let rem_size = window.rem_size();
        let font_size = window.text_style().font_size.to_pixels(rem_size);
        let mut size_layout = ItemSizeLayout::default();
        // Measure vertical lists at the last known content width so relative
        // widths and text truncation resolve like the visible rows; an
        // unconstrained measure would turn one long non-wrapping text into a
        // phantom horizontal scroll range.
        let list_width = if self.axis.is_vertical() {
            self.scroll_handle
                .state
                .borrow()
                .last_content_size
                .map(|size| size.width)
                .filter(|width| !width.is_zero())
        } else {
            None
        };
        let longest_item_size = self.measure_item(list_width, window, cx);

        let axis = self.axis;
        let items_count = self.items_count;
        let item_sizes = &self.item_sizes;
        let sizing_behavior = self.sizing_behavior;

        let layout_id = self.base.interactivity().request_layout(
            global_id,
            inspector_id,
            window,
            cx,
            |style, window, cx| {
                size_layout = window.with_element_state(
                    global_id.unwrap(),
                    |state: Option<ItemSizeLayout>, _window| {
                        let mut state = state.unwrap_or(ItemSizeLayout::default());

                        // Including the gap between items for calculating the item size.
                        let gap = style
                            .gap
                            .along(axis)
                            .to_pixels(font_size.into(), rem_size);

                        if state.items_sizes != *item_sizes
                            || state.axis_and_gap != Some((axis, gap))
                        {
                            state.items_sizes = item_sizes.clone();
                            state.axis_and_gap = Some((axis, gap));
                            // Prepare each item's size along the axis.
                            state.sizes = item_sizes
                                .iter()
                                .enumerate()
                                .map(|(i, size)| {
                                    let size = size.along(axis);
                                    if i + 1 == items_count {
                                        size
                                    } else {
                                        size + gap
                                    }
                                })
                                .collect();

                            // Prepare each item's origin along the axis.
                            let mut cumulative = px(0.0);
                            state.origins = state
                                .sizes
                                .iter()
                                .map(|size| {
                                    let origin = cumulative;
                                    cumulative += *size;
                                    origin
                                })
                                .collect();

                            if axis.is_horizontal() {
                                state.content_size.width = cumulative;
                            } else {
                                state.content_size.height = cumulative;
                            }
                        }

                        if axis.is_horizontal() {
                            state.content_size.height = longest_item_size.height;
                        } else {
                            state.content_size.width = longest_item_size.width;
                        }

                        (state.clone(), state)
                    },
                );

                let layout_id = match sizing_behavior {
                    ListSizingBehavior::Infer => {
                        window.with_text_style(style.text_style().cloned(), |window| {
                            let size_layout = size_layout.clone();
                            window.request_measured_layout(style, {
                                move |known_dimensions, available_space, _, _| {
                                    let mut size = Size::default();
                                    if axis.is_horizontal() {
                                        size.width = known_dimensions.width.unwrap_or(
                                            match available_space.width {
                                                AvailableSpace::Definite(x) => x,
                                                AvailableSpace::MinContent
                                                | AvailableSpace::MaxContent => {
                                                    size_layout.content_size.width
                                                }
                                            },
                                        );
                                        size.height = known_dimensions.height.unwrap_or(
                                            match available_space.height {
                                                AvailableSpace::Definite(x) => x,
                                                AvailableSpace::MinContent
                                                | AvailableSpace::MaxContent => {
                                                    size_layout.content_size.height
                                                }
                                            },
                                        );
                                    } else {
                                        size.width = known_dimensions.width.unwrap_or(
                                            match available_space.width {
                                                AvailableSpace::Definite(x) => x,
                                                AvailableSpace::MinContent
                                                | AvailableSpace::MaxContent => {
                                                    size_layout.content_size.width
                                                }
                                            },
                                        );
                                        size.height = known_dimensions.height.unwrap_or(
                                            match available_space.height {
                                                AvailableSpace::Definite(x) => x,
                                                AvailableSpace::MinContent
                                                | AvailableSpace::MaxContent => {
                                                    size_layout.content_size.height
                                                }
                                            },
                                        );
                                    }

                                    size
                                }
                            })
                        })
                    }
                    ListSizingBehavior::Auto => window
                        .with_text_style(style.text_style().cloned(), |window| {
                            window.request_layout(style, None, cx)
                        }),
                };

                layout_id
            },
        );

        (
            layout_id,
            VirtualListFrameState {
                items: Vec::new(),
                size_layout,
            },
        )
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut VirtualListFrameState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        layout.size_layout.last_layout_bounds = bounds;

        let style = self
            .base
            .interactivity()
            .compute_style(global_id, None, window, cx);
        let border_widths = style.border_widths.to_pixels(window.rem_size());
        let paddings = style
            .padding
            .to_pixels(bounds.size.into(), window.rem_size());

        let item_sizes = &layout.size_layout.sizes;
        let item_origins = &layout.size_layout.origins;

        let content_bounds = Bounds::from_corners(
            bounds.origin
                + point(
                    border_widths.left + paddings.left,
                    border_widths.top + paddings.top,
                ),
            bounds.bottom_right()
                - point(
                    border_widths.right + paddings.right,
                    border_widths.bottom + paddings.bottom,
                ),
        );

        let axis = self.axis;
        let mut scroll_state = self.scroll_handle.state.borrow_mut();
        scroll_state.axis = axis;
        scroll_state.items_count = self.items_count;
        scroll_state.last_content_size = Some(content_bounds.size);

        let mut scroll_offset = self.scroll_handle.offset();
        if let Some(scroll_to_item) = scroll_state.deferred_scroll_to_item.take() {
            scroll_offset = self.scroll_to_deferred_item(
                scroll_offset,
                &layout.size_layout,
                &content_bounds,
                scroll_to_item,
            );
        }

        scroll_offset = scroll_offset
            .max(&point(
                content_bounds.size.width - layout.size_layout.content_size.width,
                content_bounds.size.height - layout.size_layout.content_size.height,
            ))
            .min(&point(px(0.0), px(0.0)));
        if scroll_offset != self.scroll_handle.offset() {
            self.scroll_handle.set_offset(scroll_offset);
        }

        let axis = self.axis;
        let items_count = self.items_count;
        let scroll_handle = self.scroll_handle.clone();
        let render_items = &self.render_items;

        self.base.interactivity().prepaint(
            global_id,
            inspector_id,
            bounds,
            layout.size_layout.content_size,
            window,
            cx,
            |_style, _, hitbox, window, cx| {
                if items_count > 0 {
                    let min_scroll_offset =
                        content_bounds.size.along(axis) - layout.size_layout.content_size.along(axis);

                    let is_scrolled = !scroll_offset.along(axis).is_zero();
                    let mut scroll_offset = scroll_offset;
                    if is_scrolled {
                        match axis {
                            Axis::Horizontal if scroll_offset.x < min_scroll_offset => {
                                scroll_offset.x = min_scroll_offset;
                                scroll_handle.set_offset(scroll_offset);
                            }
                            Axis::Vertical if scroll_offset.y < min_scroll_offset => {
                                scroll_offset.y = min_scroll_offset;
                                scroll_handle.set_offset(scroll_offset);
                            }
                            _ => {}
                        }
                    }

                    // The viewport in content space: the scroll offset is
                    // negative, and the leading padding is admitted at the start
                    // so an item under it is still drawn.
                    let viewport = match axis {
                        Axis::Horizontal => {
                            -(scroll_offset.x + paddings.left)
                                ..-scroll_offset.x + content_bounds.size.width
                        }
                        Axis::Vertical => {
                            -(scroll_offset.y + paddings.top)
                                ..-scroll_offset.y + content_bounds.size.height
                        }
                    };
                    let visible_range = visible_range(item_origins, item_sizes, viewport);

                    let items = render_items(visible_range.clone(), window, cx);

                    let content_mask = ContentMask { bounds };
                    window.with_content_mask(Some(content_mask), |window| {
                        for (mut item, ix) in items.into_iter().zip(visible_range.clone()) {
                            let item_origin = match axis {
                                Axis::Horizontal => {
                                    content_bounds.origin
                                        + point(
                                            item_origins[ix] + scroll_offset.x,
                                            scroll_offset.y,
                                        )
                                }
                                Axis::Vertical => {
                                    content_bounds.origin
                                        + point(
                                            scroll_offset.x,
                                            item_origins[ix] + scroll_offset.y,
                                        )
                                }
                            };

                            let available_space = match axis {
                                Axis::Horizontal => size(
                                    AvailableSpace::Definite(item_sizes[ix]),
                                    AvailableSpace::Definite(content_bounds.size.height),
                                ),
                                Axis::Vertical => size(
                                    AvailableSpace::Definite(content_bounds.size.width),
                                    AvailableSpace::Definite(item_sizes[ix]),
                                ),
                            };

                            item.layout_as_root(available_space, window, cx);
                            item.prepaint_at(item_origin, window, cx);
                            layout.items.push(item);
                        }
                    });
                }

                hitbox
            },
        )
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut VirtualListFrameState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let items = &mut layout.items;
        self.base.interactivity().paint(
            global_id,
            inspector_id,
            bounds,
            hitbox.as_ref(),
            window,
            cx,
            |_, window, cx| {
                for item in items.iter_mut() {
                    item.paint(window, cx);
                }
            },
        )
    }
}
