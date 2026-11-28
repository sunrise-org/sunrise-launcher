//! Settings › Appearance: the look (Sunrise or Light), the colour editor and Home's header.
//!
//! Every colour of a theme is one row of the editor: its swatch and its hex. Pressing the row
//! opens a colour picker under the swatch (`widgets::colour_picker`), and a change is painted as
//! soon as it is made, dragged or typed. What a row does comes from the table of colours,
//! `themes::library::SLOTS`.
use super::inputs::HEX;
use super::*;
use crate::widgets::button::secondary;
use crate::widgets::choice::choices;
use crate::widgets::colour_picker::{self, Handlers, Part, Spec};
use crate::widgets::dropdown::{Dropdown, Item};
use gpui::SharedString;
use std::rc::Rc;
use sunrise_launcher::theme_files;
use sunrise_launcher::themes::hsv::Hsv;
use sunrise_launcher::themes::library::SLOTS;

/// A colour's swatch in the editor.
const CHIP: f32 = 22.;
/// How far under a swatch its picker starts.
const DROP: f32 = CHIP + 6.;

impl Launcher {
    /// Sunrise's night or its day: the two looks every theme starts from.
    pub(super) fn look_row(&mut self, cx: &mut Context<Self>) -> Div {
        let light = self.editor_roles().light;
        let options: Vec<(bool, SharedString)> =
            vec![(false, tr("Sunrise").into()), (true, tr("Light").into())];
        let this = cx.entity();
        let pick = choices("alt-look", &options, light, move |light, window, cx| {
            this.update(cx, |this, cx| {
                this.look_pick(light, window);
                cx.notify();
            })
        });
        row(
            tr("Look"),
            tr("Sunrise's night or its day. Choosing one puts the colours below back to its own."),
            div().child(pick),
        )
    }

    /// Where Home's header stands over the banner.
    pub(super) fn header_row(&mut self, cx: &mut Context<Self>) -> Div {
        use sunrise_launcher::backend::HeaderAlign;
        let current = self.settings.header_align;
        let open = self.ui.settings.appearance.header_open;
        let items: Vec<Item> = HeaderAlign::ALL
            .iter()
            .map(|&a| Item::new(tr(a.label())).current(a == current))
            .collect();
        let dropdown = Dropdown::new("alt-header-align", items, open)
            .on_toggle({
                let this = cx.entity();
                move |_, cx| {
                    this.update(cx, |this, cx| {
                        let was = this.ui.settings.appearance.header_open;
                        this.close_menus();
                        let state = &mut this.ui.settings.appearance;
                        state.more = None;
                        state.header_open = !was;
                        cx.notify();
                    })
                }
            })
            .on_close({
                let this = cx.entity();
                move |_, cx| {
                    this.update(cx, |this, cx| {
                        this.ui.settings.appearance.header_open = false;
                        cx.notify();
                    })
                }
            })
            .on_pick({
                let this = cx.entity();
                move |n, _, cx| {
                    this.update(cx, |this, cx| {
                        if let Some(&picked) = HeaderAlign::ALL.get(n) {
                            this.settings.header_align = picked;
                            this.save_settings();
                        }
                        cx.notify();
                    })
                }
            })
            .build();
        row(
            tr("Home header"),
            tr("Where the profile's name and mark stand over the banner."),
            dropdown,
        )
    }

    /// The colour editor: a row per colour of a theme, the accent's presets under its row, and
    /// the way back to Sunrise's own colours.
    pub(super) fn colours_row(&mut self, window: &Window, cx: &mut Context<Self>) -> Div {
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
        let mut lines = div().flex().flex_col().gap(px(8.));
        for (i, slot) in SLOTS.iter().enumerate() {
            let shown = (slot.get)(&roles);
            lines = lines.child(self.colour_line(i, slot.label, shown, window, cx));
        }
        let own = self
            .settings
            .theme
            .as_deref()
            .is_some_and(|id| id.starts_with(theme_files::PREFIX));
        let custom = own || self.ui.settings.appearance.draft.is_some();
        let label = if roles.light {
            tr("Reset to Light")
        } else {
            tr("Reset to Sunrise")
        };
        let reset = secondary("alt-colours-reset", label);
        let reset = if custom {
            reset.on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                let light = this.editor_roles().light;
                this.look_pick(light, window);
                cx.notify();
            }))
        } else {
            off(reset)
        };
        let head = self.wide_row(
            tr("Colours"),
            tr("Click a colour to pick it or type its hex. Every change is painted as you make it."),
            div().child(reset),
        );
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(head)
            .child(lines)
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
        let open = self.ui.settings.appearance.editing == Some(i);
        let chip = div()
            .id(("alt-colour-swatch", i))
            .flex_none()
            .size(px(CHIP))
            .rounded(crate::tokens::r(crate::tokens::R_SMALL))
            .border_1()
            .border_color(rgb(palette().line))
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
            .w(px(130.))
            .font_family(theme::MONO)
            .text_size(px(tokens::LABEL))
            .text_color(rgb(palette().muted))
            .child(format!("#{shown:06x}"));
        div()
            .id(("alt-colour-row", i))
            .flex()
            .items_center()
            .gap(px(12.))
            .cursor_pointer()
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
                move |_: &mut Window, cx: &mut App| {
                    this.update(cx, |this, cx| {
                        this.ui.settings.appearance.dragging = None;
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
