//! Settings › Appearance: the current theme's bar, the colour editor it opens and Home's header.
//!
//! Every colour of a theme is one row of the editor: its swatch and its hex. Pressing the row
//! opens a colour picker under the swatch (`widgets::colour_picker`), and a change is painted as
//! soon as it is made, dragged or typed. What a row does comes from the table of colours,
//! `themes::library::SLOTS`.
use super::inputs::HEX;
use super::preview::swatches;
use super::*;
use crate::widgets::button::secondary;
use crate::widgets::colour_picker::{self, Handlers, Part, Spec};
use crate::widgets::dropdown::{Dropdown, Item};
use crate::widgets::focus::c;
use gpui::SharedString;
use std::rc::Rc;
use sunrise_launcher::appearance;
use sunrise_launcher::themes::hsv::Hsv;
use sunrise_launcher::themes::library::SLOTS;
use sunrise_launcher::themes::{self, library, Resolved};

/// A colour's swatch in the editor.
const CHIP: f32 = 22.;
/// A pane narrower than this has its colours in one column, and its tiles two across.
const NARROW: f32 = 640.;
/// How far under a swatch its picker starts.
const DROP: f32 = CHIP + 6.;

impl Launcher {
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

    /// What the current theme is called: a built-in's name, a saved theme's, or "Custom" for the
    /// working copy the editor paints.
    pub(super) fn current_name(&self) -> String {
        match themes::resolve(self.settings.theme.as_deref()) {
            Resolved::Twilight => tr(themes::TWILIGHT_NAME).to_string(),
            Resolved::Builtin(theme) => tr(theme.name).to_string(),
            Resolved::Local(local) if local.slug == library::WORKING => tr("Custom").to_string(),
            Resolved::Local(local) => local.name,
            Resolved::Missing => tr(themes::DAWN_THEME.name).to_string(),
        }
    }

    /// The current theme's bar: its nine colours, its name, Share and Customize colours, and
    /// under them, while it is open, the colour editor.
    pub(super) fn current_card(&mut self, window: &Window, cx: &mut Context<Self>) -> Div {
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
        let open = self.ui.settings.appearance.colours_open;
        let roles = self.editor_roles();
        let share = secondary("alt-current-share", tr("Share")).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                this.share_current();
                cx.notify();
            },
        ));
        let chevron = crate::widgets::motion_bits::chevron(
            "alt-colours-chevron",
            open,
            14.,
            c(palette().text),
        );
        let customize = secondary("alt-colours-customize", tr("Customize colours"))
            .gap(px(6.))
            .child(chevron)
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.colours_toggle(window);
                cx.notify();
            }));
        let words = div()
            .flex_1()
            .min_w(px(240.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.current_name()),
                    )
                    .child(hint(tr("Changing a colour makes it your own copy."))),
            )
            .child(swatches(SharedString::from("alt-current-swatch"), &roles));
        let head = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x(px(16.))
            .gap_y(px(10.))
            .child(words)
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(share)
                    .child(customize),
            );
        let panel = self.colours_panel(window, cx);
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .p(px(16.))
            .rounded(crate::tokens::r(crate::tokens::R_CARD))
            .border_1()
            .border_color(palette().rule())
            .bg(palette().card())
            .child(head)
            .children(panel)
    }

    /// Opens or closes the colour editor; a picker left open closes with it.
    fn colours_toggle(&mut self, window: &Window) {
        self.colour_commit(window);
        let state = &mut self.ui.settings.appearance;
        state.colours_open = !state.colours_open;
        state.colours_since = Some(std::time::Instant::now());
    }

    /// Back to the built-in theme the colours being edited came from (Sunrise's night or day
    /// when it isn't known). Dead until the colours are the player's own copy.
    fn reset_button(
        &mut self,
        light: bool,
        origin: Option<&'static str>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let working = self.settings.theme.as_deref() == Some(library::WORKING_ID);
        let target = origin.unwrap_or(if light {
            themes::DAWN_LIGHT_THEME.id
        } else {
            themes::DAWN_THEME.id
        });
        let named = themes::theme(Some(target)).map_or(themes::DAWN_THEME.name, |t| t.name);
        let label = trf("Reset to {name}", &[("name", tr(named))]);
        let reset = secondary("alt-colours-reset", label);
        if working {
            reset.on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.appearance_change(|s| appearance::select_theme(s, target), window);
                cx.notify();
            }))
        } else {
            off(reset)
        }
    }

    /// The colour editor: a row per colour in a grid (one column when the pane is narrow), and
    /// the way back to the built-in theme the colours came from. It opens and closes by its
    /// height, at once under reduced motion.
    fn colours_panel(&mut self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        use gpui::AnimationExt as _;
        let quick = sunrise_launcher_theme::QUICK;
        let state = &self.ui.settings.appearance;
        let open = state.colours_open;
        let glide = state
            .colours_since
            .filter(|since| !crate::motion::still() && since.elapsed() < quick.duration());
        if !open && glide.is_none() {
            return None;
        }
        let origin = state.origin;
        let cols: u16 = if self.settings_pane_width() < NARROW {
            1
        } else {
            3
        };
        let roles = self.editor_roles();
        let mut lines = div().grid().grid_cols(cols).gap_x(px(24.)).gap_y(px(4.));
        for (i, slot) in SLOTS.iter().enumerate() {
            lines = lines.child(self.colour_line(i, slot.label, (slot.get)(&roles), window, cx));
        }
        let reset = self.reset_button(roles.light, origin, cx);
        let head = div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap(px(10.))
            .child(hint(tr(
                "Click a colour to pick it or type its hex. Every change is painted as you make it.",
            )))
            .child(reset);
        let body = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .pt(px(12.))
            .border_t_1()
            .border_color(palette().rule())
            .child(head)
            .child(lines);
        let Some(since) = glide else {
            return Some(body.into_any_element());
        };
        // The height the editor comes to, near enough: its head and the rows of colours.
        let rows = (SLOTS.len() as f32 / f32::from(cols)).ceil();
        let full = 90. + rows * 30. + (rows - 1.) * 4.;
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        let start = *START.get_or_init(std::time::Instant::now);
        let key = since.saturating_duration_since(start).as_micros() as usize;
        Some(
            body.with_animation(
                ElementId::named_usize("alt-colours-glide", key),
                crate::motion::eased(quick),
                move |d, p| {
                    let shown = if open { p } else { 1. - p };
                    if open && p >= 1. {
                        d
                    } else {
                        d.overflow_hidden().max_h(px(full * shown)).opacity(shown)
                    }
                },
            )
            .into_any_element(),
        )
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
            .w(px(66.))
            .font_family(theme::MONO)
            .text_size(px(tokens::LABEL))
            .text_color(rgb(palette().muted))
            .child(format!("#{shown:06x}"));
        div()
            .id(("alt-colour-row", i))
            .flex()
            .items_center()
            .gap(px(12.))
            .min_w_0()
            .py(px(4.))
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
