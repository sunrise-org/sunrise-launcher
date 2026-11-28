//! The dialog's left pane: the profile's icon and name, a line saying what it will be, the
//! colours to pick from and the button for a picture.
use super::*;
use crate::sheets::channel_name;
use crate::widgets::button::{button, Look, Size, Style};
use crate::widgets::colour_picker::{self, Spec};
use crate::widgets::focus::Reach;
use instances::Icon;

/// The swatches' size: three of them and two 8 px gaps across the pane's 180 px of room, in
/// three rows: the Sunrise mark, the seven colours and the custom colour.
const SWATCH: f32 = 54.;

impl Launcher {
    pub(super) fn profile_side(&mut self, name: &str, cx: &mut Context<Self>) -> Div {
        let p = palette();
        let Some(d) = self.instances.dialog.as_ref() else {
            return div();
        };
        let (busy, icon, channel, pin) = (d.busy, d.icon.clone(), d.channel, d.pin.clone());
        let picture = d
            .picture
            .as_ref()
            .and_then(Option::as_ref)
            .map(|c| c.preview.clone());
        let copying = d.copy.as_ref().map(|c| c.source().name.clone());
        let bound = d
            .copy
            .as_ref()
            .and_then(instances::CopyPreview::base)
            .map(|tag| self.version_label(tag));
        let build = match (&bound, &pin) {
            (Some(label), _) => label.clone(),
            (None, Some(tag)) => self.version_label(tag),
            (None, None) => String::new(),
        };
        let channel_line = if build.is_empty() {
            trf(
                "{channel} · follows the latest build",
                &[("channel", channel_name(channel))],
            )
        } else {
            trf(
                "{channel} · {tag}",
                &[("channel", channel_name(channel)), ("tag", &build)],
            )
        };
        let shown_name = if name.is_empty() {
            tr("New profile").to_owned()
        } else {
            name.to_owned()
        };
        let face = if let Some(image) = picture {
            div()
                .size(px(88.))
                .rounded(tokens::r(18.))
                .overflow_hidden()
                .child(
                    gpui::img(image)
                        .size_full()
                        .object_fit(gpui::ObjectFit::Contain),
                )
        } else {
            match icon.as_ref() {
                Some(Icon::Colour(c)) => {
                    let colour =
                        u32::from_str_radix(c.trim_start_matches('#'), 16).unwrap_or(p.accent);
                    let ink = if sunrise_launcher_theme::contrast(colour, p.text)
                        > sunrise_launcher_theme::contrast(colour, p.background)
                    {
                        p.text
                    } else {
                        p.background
                    };
                    div()
                        .size(px(88.))
                        .rounded(tokens::r(18.))
                        .bg(rgb(colour))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(rgb(ink))
                        .font_weight(FontWeight::BOLD)
                        .text_size(px(88. * 0.54))
                        .child(
                            name.chars()
                                .next()
                                .unwrap_or('S')
                                .to_uppercase()
                                .to_string(),
                        )
                }
                _ => crate::mark(88.),
            }
        };
        // Only a preview: a picture is chosen with "Use a picture…" under the colours.
        let icon_button = div()
            .id("new-picture-preview")
            .flex_none()
            .rounded(tokens::r(18.))
            .child(face);
        let muted = |text: String| {
            div()
                .text_size(px(tokens::SMALL))
                .text_color(rgb(p.muted))
                .text_center()
                .child(text)
        };
        let mut swatches = div().flex().flex_wrap().gap(px(8.)).w_full();
        swatches = swatches.child(
            div()
                .id("new-mark")
                .reach_edge()
                .size(px(SWATCH))
                .rounded(tokens::r(8.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .bg(p.card())
                .border_1()
                .border_color(p.input_edge())
                .when(icon.is_none(), |d| d.border_2().border_color(rgb(p.text)))
                .tooltip(crate::glass_ui::tip(tr("Sunrise")))
                .child(crate::mark(22.))
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(d) = this.instances.dialog.as_mut().filter(|d| !d.busy) {
                        d.icon = None;
                        // An explicit Sunrise mark displaces even a picture inherited by a copy.
                        d.picture = Some(None);
                    }
                    cx.notify();
                })),
        );
        for (idx, a) in sunrise_launcher::themes::Accent::ALL.iter().enumerate() {
            let colour = a.rgb();
            let choice = Icon::Colour(format!("#{colour:06x}"));
            let selected = icon.as_ref() == Some(&choice);
            swatches = swatches.child(
                div()
                    .id(ElementId::named_usize("new-colour", idx))
                    .reach_edge()
                    .size(px(SWATCH))
                    .rounded(tokens::r(8.))
                    .cursor_pointer()
                    .bg(rgb(colour))
                    .border_1()
                    .border_color(p.input_edge())
                    .hover(|s| s.border_color(rgb(p.text)))
                    .when(selected, |d| d.border_2().border_color(rgb(p.text)))
                    .tooltip(crate::glass_ui::tip(a.name()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(d) = this.instances.dialog.as_mut().filter(|d| !d.busy) {
                            d.icon = Some(choice.clone());
                            d.picture = Some(None);
                        }
                        cx.notify();
                    })),
            );
        }
        swatches = swatches.child(self.custom_swatch(icon.as_ref(), cx));
        let picture_button = button(
            "new-picture",
            tr("Use a picture…"),
            Look::new(Style::Secondary, Size::Medium).icon("image-01"),
        )
        .w_full()
        .when(!busy, |d| {
            d.on_click(cx.listener(|this, _, _, cx| {
                this.choose_profile_picture(cx);
                cx.notify();
            }))
        });
        let content = div()
            .relative()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(14.))
            .px(px(20.))
            .py(px(28.))
            .child(icon_button)
            .child(
                div()
                    .max_w_full()
                    .truncate()
                    .text_size(px(17.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(shown_name),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(2.))
                    .child(muted(channel_line))
                    .child(muted(match copying.as_deref() {
                        Some(from) => trf("Copies “{name}”", &[("name", from)]),
                        None => tr("Starts empty").to_owned(),
                    })),
            )
            .child(div().w_full().h(px(1.)).bg(rgb(p.divider)))
            .child(swatches)
            .child(picture_button);
        // On the plate's own fill, set off from the form by a divider only.
        div()
            .relative()
            .w(px(220.))
            .flex_none()
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .right_0()
                    .w(px(1.))
                    .bg(rgb(p.divider)),
            )
            .child(content)
    }

    /// The ninth swatch: a rainbow that opens the colour picker, or, while the profile's colour
    /// is none of the presets, that colour (selected). The popover hangs under it.
    fn custom_swatch(&mut self, icon: Option<&Icon>, cx: &mut Context<Self>) -> Div {
        let p = palette();
        let preset = |c: &str| {
            sunrise_launcher::themes::Accent::ALL
                .iter()
                .any(|a| c.eq_ignore_ascii_case(&format!("#{:06x}", a.rgb())))
        };
        let own = match icon {
            Some(Icon::Colour(c)) if !preset(c) => parse_colour(c),
            _ => None,
        };
        let state = &mut self.instances.picker;
        let open = state.open;
        let shown = match icon {
            Some(Icon::Colour(c)) => parse_colour(c).unwrap_or(p.accent),
            _ => p.accent,
        };
        let swatch = div()
            .id("new-colour-custom")
            .reach_edge()
            .size(px(SWATCH))
            .rounded(tokens::r(8.))
            .overflow_hidden()
            .cursor_pointer()
            .border_1()
            .border_color(p.input_edge())
            .hover(|s| s.border_color(rgb(p.text)))
            .when(own.is_some() || open, |d| {
                d.border_2().border_color(rgb(p.text))
            })
            .tooltip(crate::glass_ui::tip(tr("Custom colour")))
            .child(match own {
                Some(colour) => div().size_full().bg(rgb(colour)),
                None => rainbow(),
            })
            .on_click(cx.listener(|this, _, window, cx| {
                let start = match this.instances.dialog.as_ref().map(|d| &d.icon) {
                    Some(Some(Icon::Colour(c))) => parse_colour(c),
                    _ => None,
                };
                if this.instances.dialog.as_ref().is_some_and(|d| !d.busy) {
                    this.instances
                        .picker
                        .open_on(start.unwrap_or(palette().accent), window, cx);
                }
                cx.notify();
            }));
        let anchor = div().relative().flex_none().size(px(SWATCH)).child(swatch);
        if !open {
            return anchor;
        }
        state.sync(shown);
        let (now, before, areas, focus) = state.view(cx);
        let hex = div()
            .id("new-colour-hex")
            .flex()
            .items_center()
            .h(px(tokens::CONTROL))
            .px(px(8.))
            .rounded(tokens::r(tokens::R_SMALL))
            .border_1()
            .border_color(rgb(p.line))
            .font_family(crate::theme::MONO)
            .text_size(px(tokens::LABEL))
            .text_color(rgb(p.muted))
            .child(format!("#{:06x}", now.to_rgb()));
        let handlers = colour_picker::handlers(
            cx.entity(),
            |this: &mut Launcher| &mut this.instances.picker,
            |this: &mut Launcher, rgb: u32, _| {
                if let Some(d) = this.instances.dialog.as_mut().filter(|d| !d.busy) {
                    d.icon = Some(Icon::Colour(format!("#{rgb:06x}")));
                    d.picture = Some(None);
                }
            },
        );
        anchor.children(colour_picker::popover(
            Spec {
                id: "new-colour-picker",
                now,
                before,
                drop: SWATCH + 6.,
                areas: &areas,
                focus: &focus,
            },
            hex,
            &handlers,
        ))
    }
}

/// `#rrggbb` as `0xRRGGBB`.
fn parse_colour(text: &str) -> Option<u32> {
    u32::from_str_radix(text.trim_start_matches('#'), 16).ok()
}

/// The hue bar's rainbow, filling its parent: six gradients side by side.
fn rainbow() -> Div {
    let segment = |k: u32| {
        let (from, to) = (k as f32 * 60., (k + 1) as f32 * 60.);
        div().flex_1().h_full().bg(gpui::linear_gradient(
            90.,
            gpui::linear_color_stop(rgb(sunrise_launcher::themes::hsv::Hsv::pure(from)), 0.),
            gpui::linear_color_stop(rgb(sunrise_launcher::themes::hsv::Hsv::pure(to)), 1.),
        ))
    };
    div().size_full().flex().children((0..6).map(segment))
}
