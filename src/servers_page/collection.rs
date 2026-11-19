//! The shared fixed toolbar and scrolling row grid of Join and Host.
use super::*;
use crate::frame::button;
use crate::paint::palette;
use gpui::rgb;

impl Launcher {
    pub(crate) fn servers_narrow(&self) -> bool {
        self.window_width
            - crate::rail::rail_width(self.settings.sidebar_expanded)
            - 2. * tokens::FRAME_INSET
            - 62.
            < 660.
    }
}

pub(crate) fn table(
    controls: Div,
    rows: Stateful<Div>,
    scroll_name: &'static str,
    thumb: &crate::scrollbar::Thumb,
    cx: &mut Context<Launcher>,
) -> Div {
    let scroll = crate::frame::scrolls(scroll_name);
    div()
        .flex_1()
        .min_h_0()
        .px(px(24.))
        .pt(px(16.))
        .pb(px(24.))
        .flex()
        .flex_col()
        .child(
            crate::home::card()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(controls)
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .relative()
                        .flex()
                        .flex_col()
                        .pr(px(11.))
                        .child(rows.track_scroll(&scroll))
                        .child(thumb.draw(&scroll, cx).right_0()),
                ),
        )
}

pub(crate) fn toolbar() -> Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(8.))
        .p(px(12.))
}

pub(crate) fn rows(id: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .overflow_y_scroll()
}

pub(crate) fn row(narrow: bool) -> Div {
    div()
        .relative()
        .flex_none()
        .flex()
        .items_center()
        .h(px(52.))
        .px(px(if narrow { 8. } else { 12. }))
        .gap(px(if narrow { 8. } else { 12. }))
        .border_t_1()
        .border_color(rgb(palette().divider))
}

pub(crate) fn identity(id: impl Into<ElementId>, name: String, address: String) -> Stateful<Div> {
    let full = format!("{name}\n{address}");
    div()
        .id(id)
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .tooltip(crate::glass_ui::tip(full))
        .child(
            div()
                .truncate()
                .font_weight(FontWeight::SEMIBOLD)
                .child(name),
        )
        .child(
            div()
                .truncate()
                .text_size(px(tokens::SMALL))
                .text_color(rgb(palette().muted))
                .child(address),
        )
}

pub(crate) fn status(narrow: bool, first: String, second: Option<String>) -> Div {
    div()
        .flex_none()
        .flex()
        .w(px(if narrow { 84. } else { 124. }))
        .when(narrow, |d| d.flex_col())
        .when(!narrow, |d| d.gap(px(8.)).items_center())
        .text_size(px(tokens::SMALL))
        .text_color(rgb(palette().muted))
        .child(div().min_w_0().truncate().child(first))
        .children(second.map(|s| div().min_w_0().truncate().child(s)))
}

pub(crate) fn remove(id: impl Into<ElementId>, label: &'static str) -> Stateful<Div> {
    button(id, "")
        .w(px(32.))
        .px_0()
        .aria_label(label)
        .tooltip(crate::glass_ui::tip(label))
        .child(crate::glass_ui::icon(
            "delete-02",
            16.,
            rgb(palette().muted).into(),
        ))
}

pub(crate) fn selection_bar(check: Stateful<Div>, label: String, actions: Div) -> Div {
    toolbar()
        .min_h(px(48.))
        .py(px(7.))
        .border_t_1()
        .border_color(rgb(palette().divider))
        .child(check)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(tokens::LABEL))
                .child(label),
        )
        .child(actions.flex_none().flex().items_center().gap(px(8.)))
}

pub(crate) fn bulk_actions(
    id: &'static str,
    remove_label: &'static str,
    clear: fn(&mut Launcher),
    remove: fn(&mut Launcher),
    cx: &mut Context<Launcher>,
) -> Div {
    div()
        .child(
            button((id, 0usize), tr("Clear")).on_click(cx.listener(move |this, _, _, cx| {
                clear(this);
                cx.notify();
            })),
        )
        .child(
            button((id, 1usize), remove_label).on_click(cx.listener(move |this, _, _, cx| {
                remove(this);
                cx.notify();
            })),
        )
}

pub(crate) fn confirmation(
    id: &'static str,
    text: String,
    label: &'static str,
    close: fn(&mut Launcher),
    action: fn(&mut Launcher),
    cx: &mut Context<Launcher>,
) -> Div {
    toolbar()
        .border_t_1()
        .border_color(rgb(palette().divider))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(tokens::LABEL))
                .child(text),
        )
        .child(
            button((id, 0usize), tr("Cancel")).on_click(cx.listener(move |this, _, _, cx| {
                close(this);
                cx.notify();
            })),
        )
        .child(
            crate::frame::primary((id, 1usize), label).on_click(cx.listener(
                move |this, _, _, cx| {
                    action(this);
                    cx.notify();
                },
            )),
        )
}

pub(crate) fn check(
    id: impl Into<ElementId>,
    checked: bool,
    label: impl Into<SharedString>,
) -> Stateful<Div> {
    crate::mods::checkbox(id, checked).aria_label(label)
}
