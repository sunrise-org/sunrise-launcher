//! The shared controls every page draws with: buttons, the segmented control, inputs, focus rings and reach, and the colour helpers they use.

use super::*;

/// The keyboard's ring, in `color`, around a filled control that has no edge of its own to turn
/// gold. Behind a see-through control it would show as a fill: give that one an edge instead.
pub(crate) fn focus_ring(color: u32) -> Vec<gpui::BoxShadow> {
    vec![gpui::BoxShadow {
        color: rgb(color).into(),
        offset: gpui::point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(2.),
        inset: false,
    }]
}

pub(crate) fn button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    button_base(id, label).hover(|s| s.bg(rgb(palette().hover)))
}

/// A [`button`] that draws its own label (the launcher update's, with its icon): no text child,
/// which would take a share of the gap and push what it draws off centre.
pub(crate) fn button_drawn(id: impl Into<ElementId>) -> Stateful<Div> {
    button_frame(id).hover(|s| s.bg(rgb(palette().hover)))
}

/// What makes a clickable thing that is not a button reachable by keyboard: a Tab stop that shows
/// the ring. Enter and Space press it, as a click does.
pub(crate) trait Reach {
    /// For a thing with no edge of its own: a gold line just inside it, which takes no room and
    /// is never cut off by what it sits in.
    fn reach(self) -> Self;
    /// For a thing that has an edge: that edge turns gold.
    fn reach_edge(self) -> Self;
    /// For every Tab stop: the page it is on scrolls to show it when the keyboard lands on it.
    fn seen(self) -> Self;
}

impl<T: InteractiveElement + ParentElement + Styled> Reach for T {
    fn reach(self) -> Self {
        self.tab_index(0)
            .rounded(crate::tokens::r(crate::tokens::R_SMALL))
            .focus_visible(|s| {
                s.shadow(vec![gpui::BoxShadow {
                    inset: true,
                    spread_radius: px(1.),
                    ..focus_ring(palette().focus).remove(0)
                }])
            })
            .seen()
    }

    fn reach_edge(self) -> Self {
        self.tab_index(0)
            .focus_visible(|s| s.border_color(rgb(palette().focus)))
            .seen()
    }

    fn seen(self) -> Self {
        // The toolkit tells nobody where the focused control is. So a focused control marks the
        // text style it hands its children (a background nothing can see), and a child that
        // covers it reads the mark while the frame is laid out, when it knows its place.
        self.focus(|s| s.text_bg(focus_mark())).child(
            gpui::canvas(scroll_to_show, |_, _, _, _| {})
                .absolute()
                .inset_0(),
        )
    }
}

pub(crate) fn c(x: u32) -> gpui::Hsla {
    gpui::rgb(x).into()
}

pub(crate) fn mix(a: gpui::Hsla, b: gpui::Hsla, v: f32) -> gpui::Hsla {
    let (a, b) = (gpui::Rgba::from(a), gpui::Rgba::from(b));
    let m = |x: f32, y: f32| x + (y - x) * v;
    gpui::Hsla::from(gpui::Rgba {
        r: m(a.r, b.r),
        g: m(a.g, b.g),
        b: m(a.b, b.b),
        a: m(a.a, b.a),
    })
}

/// Two views of one page side by side as underline sub-tabs (`id`, `label`, chosen, its page):
/// Mods' Installed | Discover and Servers' Join | Host. The pair is one Tab stop, and the arrow
/// keys go to the other view, as a click on it would; `go` opens a view.
pub(crate) fn segmented<P: Copy + 'static>(
    id: &'static str,
    views: [(&'static str, SharedString, bool, P); 2],
    go: fn(&mut Launcher, P),
    cx: &mut Context<Launcher>,
) -> Stateful<Div> {
    let other = views
        .iter()
        .find(|(_, _, on, _)| !*on)
        .map(|(_, _, _, page)| *page);
    let second_on = views[1].2;
    let slide = crate::motion::sliding(id, second_on);
    let mut pair = div()
        .id(id)
        .tab_index(0)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(24.))
        .px(px(4.))
        .mx(px(-4.))
        .mb(px(-2.))
        .rounded(tokens::r(tokens::R_SMALL))
        .border_1()
        .border_color(gpui::transparent_black())
        .focus_visible(|s| s.border_color(c(palette().focus)))
        .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, _, cx| {
            if !matches!(ev.keystroke.key.as_str(), "left" | "right" | "up" | "down") {
                return;
            }
            if let Some(page) = other {
                go(this, page);
            }
            cx.stop_propagation();
            cx.notify();
        }));
    let active_color = c(palette().text);
    let muted_color = c(palette().muted);
    let accent_color = c(palette().accent);
    for (idx, (view, label, on, page)) in views.into_iter().enumerate() {
        let bar: Option<AnyElement> = match slide {
            Some(slide) => {
                // Directional glide: when moving 0 -> 1, tab 0 retracts toward the right while
                // tab 1 extends from the left; when moving 1 -> 0, the reverse.
                let retract_right = idx == 0;
                Some(
                    div()
                        .absolute()
                        .bottom_0()
                        .h(px(2.))
                        .rounded(tokens::r(tokens::R_BAR))
                        .bg(accent_color)
                        .with_animation(
                            ElementId::named_usize(format!("{view}-bar"), slide),
                            crate::motion::slide_motion(),
                            move |d, p| {
                                if on {
                                    if retract_right {
                                        d.right_0().left(gpui::relative(1. - p)).opacity(p)
                                    } else {
                                        d.left_0().right(gpui::relative(1. - p)).opacity(p)
                                    }
                                } else if retract_right {
                                    d.right_0().left(gpui::relative(p)).opacity(1. - p)
                                } else {
                                    d.left_0().right(gpui::relative(p)).opacity(1. - p)
                                }
                            },
                        )
                        .into_any_element(),
                )
            }
            None if on => Some(
                div()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .right_0()
                    .h(px(2.))
                    .rounded(tokens::r(tokens::R_BAR))
                    .bg(accent_color)
                    .into_any_element(),
            ),
            None => None,
        };
        let label_el: AnyElement = match slide {
            Some(slide) => div()
                .child(label)
                .with_animation(
                    ElementId::named_usize(format!("{view}-label"), slide),
                    crate::motion::slide_motion(),
                    move |d, p| {
                        let v = if on { p } else { 1. - p };
                        d.text_color(mix(muted_color, active_color, v))
                    },
                )
                .into_any_element(),
            None => div().child(label).into_any_element(),
        };
        pair = pair.child(
            div()
                .id(view)
                .relative()
                .pt(px(2.))
                .pb(px(10.))
                .flex()
                .items_center()
                .text_size(px(tokens::BODY))
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .when(slide.is_none() && on, |d| d.text_color(active_color))
                .when(slide.is_none() && !on, |d| {
                    d.text_color(muted_color)
                        .hover(move |s| s.text_color(active_color))
                })
                .child(label_el)
                .children(bar)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if !on {
                        go(this, page);
                    }
                    cx.notify();
                })),
        );
    }
    pair
}

pub(super) fn focus_mark() -> gpui::Hsla {
    gpui::hsla(0.125, 0.5, 0.25, 0.)
}

/// A sheet's or a page's main action: cream, like Play signed out.
pub(crate) fn primary(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    button_base(id, label)
        .bg(rgb(palette().text))
        .border_color(rgb(palette().text))
        .text_color(rgb(palette().background))
        .hover(|s| s.bg(rgb(palette().accent_on)))
        .focus_visible(|s| s.shadow(focus_ring(palette().focus)))
}

/// A copy button's face: `label`, or "Copied" with a tick on an accent tint while copy `shown`
/// (its count) shows. The focusable element stays in place while the label and decorative
/// children change, so keyboard focus stays on it.
pub(crate) fn copy_face(id: &'static str, label: &str, shown: Option<usize>) -> Stateful<Div> {
    let accent: gpui::Hsla = rgb(palette().accent).into();
    let base = button_base(
        id,
        shown.map_or_else(|| label.to_string(), |_| tr("Copied").into()),
    )
    .relative()
    .gap(px(6.));
    match shown {
        Some(n) => base
            .border_color(accent.opacity(0.5))
            .hover(move |s| s.border_color(accent.opacity(0.75)))
            // Give keyboard focus its own ring so the Copied hover border cannot hide it.
            .focus_visible(|s| s.shadow(focus_ring(palette().focus)))
            .child(crate::code_box::tick(
                &SharedString::from(id),
                n,
                16.,
                rgb(palette().text).into(),
            ))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                    .with_animation(
                        ElementId::named_usize(SharedString::from(format!("{id}-tint")), n),
                        crate::code_box::played(Duration::from_millis(200)),
                        move |d, t| d.bg(accent.opacity(0.16 * t)),
                    ),
            ),
        None => base.hover(|s| s.bg(rgb(palette().hover))),
    }
}

/// A button that copies `text`, then shows the copy on itself for a moment, as [`copy_face`].
/// The copy is remembered with `text`, so a new text (another log, a new report) starts at
/// `label` again.
pub(crate) fn copy_button(id: &'static str, label: &str, text: String) -> Stateful<Div> {
    let key = copy_key(id, &text);
    copy_face(id, label, crate::code_box::copied(&key)).on_click(move |_, window, cx| {
        crate::widgets::copy_quietly(cx, gpui::ClipboardItem::new_string(text.clone()));
        crate::code_box::note_copy(key.clone(), window, cx);
    })
}

pub(super) fn copy_key(id: &str, text: &str) -> SharedString {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{id}:{:016x}", hasher.finish()).into()
}

pub(crate) fn button_base(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
) -> Stateful<Div> {
    button_frame(id).child(label.into())
}

/// Every button's box, without what it says.
fn button_frame(id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .tab_index(0)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
        .border_1()
        .border_color(rgb(palette().line))
        .px(px(12.))
        .h(px(tokens::CONTROL))
        .text_color(rgb(palette().text))
        .text_size(px(tokens::LABEL))
        .font_weight(FontWeight::SEMIBOLD)
        .cursor_pointer()
        .focus_visible(|s| s.border_color(rgb(palette().focus)))
        .seen()
}

/// Every text input's box: one fill, one faint outline, the focus colour while typed in, its
/// text centred. The caller gives it its id, height and padding.
pub(crate) fn input(focused: bool) -> Div {
    div()
        .flex()
        .items_center()
        .overflow_hidden()
        .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
        .bg(palette().input_fill())
        .border_1()
        .border_color(if focused {
            rgb(palette().focus).into()
        } else {
            palette().input_edge()
        })
}
