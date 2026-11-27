//! The launcher's update notice, at the sidebar's foot above Help & community: a row-high
//! accent card with the update's icon and text, a small "What's new" button, and a thin bar
//! along its bottom while the update downloads. Folded, it is the icon alone with a dot. When
//! there is no update nothing is drawn and no room is taken.
use super::*;
use crate::sheets::Sheet;
use crate::widgets::focus::Reach;
use gpui::{AnimationExt, AnyElement, Hsla, MouseButton};

/// The gap from the notice's icon to its text.
const GAP: f32 = 10.;
/// The "What's new" button's size, inside the card's right end.
const NOTES: f32 = 28.;

impl Launcher {
    /// The notice, as the sidebar draws it at `open`, or nothing when there is no update.
    pub(super) fn update_notice(
        &mut self,
        open: f32,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let notice = self.launcher_update_notice()?;
        let p = palette();
        let accent: Hsla = rgb(p.accent).into();
        // As a nav row, the text waits for the sidebar to be open; a hidden version never has it.
        let labelled = open >= 1. && !notice.dot;
        let installs = notice.installs;
        let mut card = div()
            .id("launcher-update")
            .reach()
            .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
            .relative()
            .h(px(36.))
            .flex()
            .items_center()
            .overflow_hidden()
            .cursor_pointer()
            .bg(accent.opacity(0.14))
            .hover(move |d| d.bg(accent.opacity(0.22)))
            .tooltip(crate::glass_ui::tip_unless(
                notice.sheet_open,
                notice.tip.clone(),
            ));
        card = if notice.dot {
            card.flex_none().w(px(CLOSED_ROW)).justify_center()
        } else if labelled {
            card.flex_1().min_w_0().px(px(ICON_PAD)).gap(px(GAP))
        } else {
            card.flex_1().min_w_0().justify_center()
        };
        card = card.child(crate::glass_ui::icon(
            notice.icon,
            ICON,
            if notice.dot {
                rgb(p.chrome_muted).into()
            } else {
                accent
            },
        ));
        if labelled {
            card = card
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(tokens::LABEL))
                        .text_color(rgb(p.chrome_text))
                        .child(notice.label.clone()),
                )
                .when(installs, |card| card.child(self.notes_button(cx)));
        } else {
            // The label is the tooltip; the dot says there is something to take.
            card = card.child(
                div()
                    .absolute()
                    .top(px(6.))
                    .right(px(if notice.dot { 4. } else { 2. }))
                    .size(px(7.))
                    .rounded_full()
                    .bg(accent),
            );
        }
        if let Some(progress) = notice.progress {
            card = card.child(
                div()
                    .absolute()
                    .left_0()
                    .bottom_0()
                    .h(px(2.))
                    .w(gpui::relative(progress.clamp(0., 1.)))
                    .bg(accent),
            );
        }
        // Where the notes popover hangs from.
        card = card.child(
            gpui::canvas(
                move |bounds, _, _| crate::updates::note_notice_bounds(bounds),
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0(),
        );
        card = if installs {
            card.on_click(cx.listener(|this, _, _, cx| {
                this.request_launcher_update();
                cx.notify();
            }))
        } else {
            card.on_click(cx.listener(|this, _, _, cx| {
                this.open_sheet(Sheet::LauncherUpdate);
                cx.notify();
            }))
        };
        Some(
            div()
                .flex()
                .flex_none()
                .w_full()
                .mt(px(8.))
                .child(card)
                // It fades in when it first appears (at once under reduced motion).
                .with_animation(
                    "launcher-update-notice",
                    crate::motion::eased(sunrise_launcher_theme::QUICK),
                    |d, t| d.opacity(t),
                )
                .into_any_element(),
        )
    }

    /// The small "What's new" button at the card's right end.
    fn notes_button(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let p = palette();
        let hover = rgb(p.hover);
        let sheet_open = self.ui.sheet == Some(Sheet::LauncherUpdate);
        div()
            .id("launcher-update-notes")
            .reach()
            .flex_none()
            .size(px(NOTES))
            .flex()
            .items_center()
            .justify_center()
            .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
            .cursor_pointer()
            .hover(move |d| d.bg(hover))
            .tooltip(crate::glass_ui::tip_unless(sheet_open, tr("What's new")))
            .child(crate::glass_ui::icon(
                "information-circle",
                16.,
                rgb(p.chrome_muted).into(),
            ))
            // A press on the button is its own: the card under it would start the update.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, _, cx| {
                this.open_sheet(Sheet::LauncherUpdate);
                cx.notify();
            }))
    }
}
