//! Settings › Appearance: the look (Sunrise or Light), the colour editor and Home's header.
//!
//! Every colour of a theme is one row of the editor: its swatch and the hex it is typed in, and
//! a change is painted as soon as it is made. What a row does comes from the table of colours,
//! `themes::library::SLOTS`.
use super::inputs::HEX;
use super::*;
use crate::widgets::button::secondary;
use crate::widgets::choice::choices;
use crate::widgets::dropdown::{Dropdown, Item};
use gpui::SharedString;
use sunrise_launcher::theme_files;
use sunrise_launcher::themes::{
    self,
    library::{self, SLOTS},
};

/// A preset accent swatch's size, its ring included.
const SWATCH: f32 = 26.;
/// A colour's swatch in the editor.
const CHIP: f32 = 22.;
/// What [`themes::SWATCHES`] are called, in their order.
const ACCENT_NAMES: [&str; 7] = ["Violet", "Blue", "Teal", "Green", "Amber", "Red", "Pink"];

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
        // Leaving the hex field takes what was typed in it.
        let state = &mut self.ui.settings.appearance;
        let focus = state
            .hex_focus
            .get_or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true))
            .clone();
        let focused = focus.is_focused(window);
        let left = std::mem::replace(&mut state.hex_was_focused, focused) && !focused;
        if left && state.editing.is_some() {
            self.colour_commit(window);
            cx.notify();
        }
        let roles = self.editor_roles();
        let mut lines = div().flex().flex_col().gap(px(8.));
        for (i, slot) in SLOTS.iter().enumerate() {
            let shown = (slot.get)(&roles);
            lines = lines.child(self.colour_line(i, slot.label, shown, window, cx));
            if i == library::ACCENT {
                lines = lines.child(self.accent_presets(shown, cx));
            }
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
            tr("Click a colour to type its hex. Every change is painted as you make it."),
            div().child(reset),
        );
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(head)
            .child(lines)
    }

    /// One colour: its swatch, its name and its hex, which is a text box while it is typed in.
    fn colour_line(
        &mut self,
        i: usize,
        label: &'static str,
        shown: u32,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let editing = self.ui.settings.appearance.editing == Some(i);
        let chip = div()
            .id(("alt-colour-swatch", i))
            .flex_none()
            .size(px(CHIP))
            .rounded(crate::tokens::r(crate::tokens::R_SMALL))
            .border_1()
            .border_color(rgb(palette().line))
            .bg(rgb(shown))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.colour_start(i, window, cx);
                cx.notify();
            }));
        let field = if editing {
            self.text_input(HEX, Self::colour_enter, window, cx)
        } else {
            crate::widgets::field::field_box(false, false)
                .id(("alt-colour-field", i))
                .tab_index(0)
                .focus_visible(|s| s.shadow(crate::frame::focus_ring(palette().focus)))
                .font_family(theme::MONO)
                .text_size(px(tokens::LABEL))
                .cursor_text()
                .child(format!("#{shown:06x}"))
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.colour_start(i, window, cx);
                    cx.notify();
                }))
        };
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(chip)
            .child(div().flex_1().min_w_0().truncate().child(tr(label)))
            .child(field.w(px(130.)).flex_none())
    }

    /// The named accents, under the accent's row: one press sets the accent to that colour.
    fn accent_presets(&mut self, now: u32, cx: &mut Context<Self>) -> Div {
        let mut swatches = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .pl(px(CHIP + 12.));
        for (i, colour) in themes::SWATCHES.into_iter().enumerate() {
            let on = colour == now;
            let swatch = div()
                .id(("alt-accent-swatch", i))
                .flex_none()
                .size(px(SWATCH))
                .rounded_full()
                .p(px(3.))
                .border_2()
                .border_color(if on {
                    rgb(palette().text).into()
                } else {
                    gpui::transparent_black()
                })
                .reach_edge()
                .tooltip(crate::glass_ui::tip(tr(ACCENT_NAMES[i])))
                .child(div().size_full().rounded_full().bg(rgb(colour)));
            swatches = swatches.child(if on {
                swatch.cursor_default()
            } else {
                swatch
                    .cursor_pointer()
                    .hover(|s| s.border_color(rgb(palette().muted)))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.colour_set(library::ACCENT, colour, window);
                        cx.notify();
                    }))
            });
        }
        swatches
    }
}
