//! The dialog's left pane: the profile's icon and name, a line saying what it will be, the
//! colours to pick from and the button for a picture.
use super::*;
use crate::sheets::channel_name;
use crate::widgets::button::{button, Look, Size, Style};
use crate::widgets::focus::Reach;
use instances::Icon;

/// The swatches' size: four of them and three 8 px gaps across the pane's 180 px of room.
const SWATCH: f32 = 39.;

impl Launcher {
    pub(super) fn profile_side(
        &mut self,
        window: &Window,
        name: &str,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = palette();
        let Some(d) = self.instances.dialog.as_ref() else {
            return div();
        };
        let (busy, icon, channel, pin) = (d.busy, d.icon.clone(), d.channel, d.pin.clone());
        let picture_focus = d.picture_focus.clone();
        let picture_focused = picture_focus.is_focused(window);
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
        let icon_button = div()
            .id("new-picture-preview")
            .flex_none()
            .rounded(tokens::r(18.))
            .border_2()
            .border_color(gpui::transparent_black())
            .when(!busy, |d| {
                d.track_focus(&picture_focus)
                    .cursor_pointer()
                    .focus_visible(|s| s.border_color(rgb(p.focus)))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.choose_profile_picture(cx);
                        cx.notify();
                    }))
            })
            .when(picture_focused && !busy, |d| d.border_color(rgb(p.focus)))
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
}
