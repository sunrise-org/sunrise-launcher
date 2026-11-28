//! Settings › Appearance: the Edit theme dialog. A live preview (a launcher window in the
//! colours being edited), the theme's name and its nine colours in three groups, each a row
//! with its swatch and hex. Pressing a row opens a colour picker under the swatch
//! (`widgets::colour_picker`), and a change is painted as soon as it is made, dragged or typed.
//! What a row does comes from the table of colours, `themes::library::SLOTS`.
use super::inputs::{HEX, NAME};
use super::tiles::picture;
use super::*;
use crate::sheets::title;
use crate::widgets::colour_picker::{self, Handlers, Part, Spec};
use std::rc::Rc;
use sunrise_launcher::theme_files::Roles;
use sunrise_launcher::themes::hsv::Hsv;
use sunrise_launcher::themes::library::SLOTS;

/// A colour's swatch in the editor.
const CHIP: f32 = 22.;
/// How far under a swatch its picker starts.
const DROP: f32 = CHIP + 6.;
/// The preview's width, and how much bigger than a tile's picture it is drawn.
const PREVIEW_W: f32 = 280.;
const PREVIEW_SCALE: f32 = 1.7;

/// The groups of colours, each with its heading and the places of its colours in `SLOTS`.
const GROUPS: [(&str, &[usize]); 3] = [
    (crate::i18n::tr_key("Surfaces"), &[0, 1, 2, 6]),
    (crate::i18n::tr_key("Text"), &[3, 4]),
    (crate::i18n::tr_key("Accents"), &[5, 7, 8]),
];

impl Launcher {
    /// The dialog's body: the preview and the name on the left, the colours on the right, and
    /// Cancel and Done under them.
    pub(super) fn edit_dialog(&mut self, window: &Window, cx: &mut Context<Self>) -> Div {
        // Leaving the hex field takes what was typed in it; the picker stays open.
        let state = &mut self.ui.settings.appearance;
        let focus = state
            .hex_focus
            .get_or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true))
            .clone();
        let focused = focus.is_focused(window);
        let left = std::mem::replace(&mut state.hex_was_focused, focused) && !focused;
        if left && state.editing.is_some() {
            self.hex_apply(window);
            cx.notify();
        }
        let roles = self.editor_roles();
        let name = self
            .text_input(NAME, Self::dialog_confirm, window, cx)
            .w_full();
        let state = &self.ui.settings.appearance;
        let ready = !state.name.text().trim().is_empty();
        let problem = state
            .error
            .clone()
            .or_else(|| state.dialog_error.clone())
            .map(|why| self.dialog_problem(why));
        let side = div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(12.))
            .w(px(PREVIEW_W))
            .max_w_full()
            .child(picture(&roles, PREVIEW_SCALE))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(small_heading(tr("Name")))
                    .child(name),
            );
        let mut colours = div()
            .flex_1()
            .min_w(px(260.))
            .flex()
            .flex_col()
            .gap(px(14.));
        for (heading, slots) in GROUPS {
            colours = colours.child(self.colour_group(heading, slots, &roles, window, cx));
        }
        let ok = self.dialog_ok(tr("Done"), ready, cx);
        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(title(tr("Edit theme").into()))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_x(px(24.))
                    .gap_y(px(16.))
                    .child(side)
                    .child(colours),
            )
            .children(problem)
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(self.dialog_cancel("Cancel", cx))
                    .child(ok),
            )
    }

    /// One group of colours: its small heading and a row for each.
    fn colour_group(
        &mut self,
        heading: &'static str,
        slots: &[usize],
        roles: &Roles,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut group = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(small_heading(tr(heading)));
        for &i in slots {
            let slot = &SLOTS[i];
            group = group.child(self.colour_line(i, slot.label, (slot.get)(roles), window, cx));
        }
        group
    }

    /// One colour: its swatch, its name and its hex. Pressing the row opens its picker.
    fn colour_line(
        &mut self,
        i: usize,
        label: &'static str,
        shown: u32,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = palette();
        let open = self.ui.settings.appearance.editing == Some(i);
        let chip = div()
            .id(("alt-colour-swatch", i))
            .flex_none()
            .size(px(CHIP))
            .rounded(crate::tokens::r(crate::tokens::R_SMALL))
            .border_1()
            .border_color(rgb(p.line))
            .bg(rgb(shown));
        let anchor = div()
            .relative()
            .flex_none()
            .size(px(CHIP))
            .child(chip)
            .children(
                open.then(|| self.colour_picker(shown, window, cx))
                    .into_iter()
                    .flatten(),
            );
        let hex = div()
            .flex_none()
            .w(px(66.))
            .font_family(theme::MONO)
            .text_size(px(tokens::LABEL))
            .text_color(rgb(p.muted))
            .child(format!("#{shown:06x}"));
        div()
            .id(("alt-colour-row", i))
            .flex()
            .items_center()
            .gap(px(12.))
            .min_w_0()
            .px(px(6.))
            .py(px(4.))
            .rounded(crate::tokens::r(crate::tokens::R_SMALL))
            .reach()
            .cursor_pointer()
            .hover(move |s| s.bg(rgb(p.hover)))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.colour_start(i, window, cx);
                cx.notify();
            }))
            .child(anchor)
            .child(div().flex_1().min_w_0().truncate().child(tr(label)))
            .child(hex)
    }

    /// The open picker's popover: its parts, and the hex field that goes with it.
    fn colour_picker(
        &mut self,
        shown: u32,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> [gpui::Deferred; 2] {
        let state = &mut self.ui.settings.appearance;
        // A colour changed from outside the picker (the hex, a failed save) is read again; one
        // the picker made keeps its hue and the grey it may be on.
        if state.hsv.to_rgb() != shown {
            state.hsv = Hsv::from_rgb(shown);
        }
        let (now, before, areas) = (state.hsv, state.before, state.areas.clone());
        let focus = state
            .picker_focus
            .get_or_insert_with(|| cx.focus_handle())
            .clone();
        let hex = self.text_input(HEX, Self::colour_enter, window, cx);
        let this = cx.entity();
        let handlers = Handlers {
            press: Rc::new({
                let this = this.clone();
                move |part: Part,
                      at: gpui::Point<gpui::Pixels>,
                      window: &mut Window,
                      cx: &mut App| {
                    this.update(cx, |this, cx| {
                        this.picker_press(part, at, window);
                        cx.notify();
                    })
                }
            }),
            drag: Rc::new({
                let this = this.clone();
                move |ev: &gpui::MouseMoveEvent, window: &mut Window, cx: &mut App| {
                    this.update(cx, |this, cx| {
                        this.picker_drag(ev, window);
                        cx.notify();
                    })
                }
            }),
            release: Rc::new({
                let this = this.clone();
                move |window: &mut Window, cx: &mut App| {
                    this.update(cx, |this, cx| {
                        this.picker_release(window);
                        cx.notify();
                    })
                }
            }),
            close: Rc::new(move |window: &mut Window, cx: &mut App| {
                this.update(cx, |this, cx| {
                    this.colour_commit(window);
                    cx.notify();
                })
            }),
        };
        colour_picker::popover(
            Spec {
                id: "alt-colour-picker",
                now,
                before,
                drop: DROP,
                areas: &areas,
                focus: &focus,
            },
            hex,
            &handlers,
        )
    }
}

/// A group's or a field's small heading.
fn small_heading(text: impl Into<SharedString>) -> Div {
    div()
        .px(px(6.))
        .text_size(px(tokens::SMALL))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(palette().muted))
        .child(text.into())
}
