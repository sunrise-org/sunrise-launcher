//! The bar under a list while rows are ticked: how many, a Select all / Clear toggle, and the
//! buttons for what applies to them. A card of its own, as wide as the list's cards and lifted
//! off the panel's edge; it rises in on the quick curve, and is simply there under reduced
//! motion.
use crate::frame::Reach as _;
use crate::paint::palette;
use crate::tokens;
use crate::widgets::button::{danger, secondary};
use gpui::{
    div, prelude::*, px, rgb, AnimationExt, AnyElement, Div, ElementId, FontWeight, SharedString,
    Stateful,
};
use sunrise_launcher::i18n::trf;

/// The bar's own text button (Select all, Clear). The caller adds its click.
pub(crate) fn link(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .reach()
        .flex_none()
        .cursor_pointer()
        .underline()
        .text_size(px(tokens::SMALL))
        .text_color(rgb(palette().muted))
        .hover(|s| s.text_color(rgb(palette().text)))
        .child(label.into())
}

/// One of the bar's buttons. Not `usable`, it is drawn dim and takes no Tab stop: the caller
/// adds a click only to a usable one.
pub(crate) fn action(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    usable: bool,
    destructive: bool,
) -> Stateful<Div> {
    let button = if destructive {
        danger(id, label)
    } else {
        secondary(id, label)
    };
    if usable {
        button
    } else {
        button.opacity(0.45).cursor_default().tab_stop(false)
    }
}

/// The bar: "{count} selected", the toggle, then `actions` on the right.
pub(crate) fn bar(count: usize, toggle: Stateful<Div>, actions: Vec<AnyElement>) -> AnyElement {
    div()
        .id("selection-bar")
        .flex_none()
        .overflow_hidden()
        // Room for the card's shadow, and a gap under it to the panel's edge.
        .pt(px(8.))
        .pb(px(12.))
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(10.))
                .pl(px(16.))
                .pr(px(8.))
                .py(px(8.))
                .min_h(px(52.))
                .rounded(tokens::r(tokens::R_CARD))
                .border_1()
                .border_color(rgb(palette().line))
                .bg(rgb(palette().raised))
                .shadow_md()
                .child(
                    div()
                        .flex_none()
                        .text_size(px(tokens::LABEL))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(trf("{count} selected", &[("count", &count.to_string())])),
                )
                .child(toggle)
                .child(div().flex_1())
                .children(actions),
        )
        .with_animation(
            "selection-bar-in",
            crate::motion::eased(sunrise_launcher_theme::QUICK),
            |bar, progress| {
                if progress < 1. {
                    bar.max_h(px(180. * progress)).opacity(progress)
                } else {
                    bar.opacity(1.)
                }
            },
        )
        .into_any_element()
}
