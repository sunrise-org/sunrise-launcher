//! The notice sheet: a problem's whole text, with one way out.

use super::*;
use crate::widgets::button::{primary, secondary};

/// The notice's text as its lead and its detail. The first paragraph says what happened; the
/// rest is the detail, which reads as a code box with a copy button. A notice of one paragraph
/// is all lead.
fn lead_and_detail(text: &str) -> (Vec<Div>, Option<Div>) {
    let paragraphs: Vec<&str> = text.split('\n').filter(|p| !p.trim().is_empty()).collect();
    let split = if paragraphs.len() > 1 {
        1
    } else {
        paragraphs.len()
    };
    let lead: Vec<Div> = paragraphs[..split]
        .iter()
        .map(|p| div().text_size(px(tokens::LABEL)).child(p.to_string()))
        .collect();
    let detail = (split < paragraphs.len()).then(|| {
        let blocks = paragraphs[split..]
            .iter()
            .map(|line| SharedString::from(line.to_string()))
            .collect();
        crate::code_box::code_box("alt-notice-detail", blocks, Some(240.)).w_full()
    });
    (lead, detail)
}

impl Launcher {
    /// What went wrong, in full: the lead, its detail in a code box, and one way out.
    pub(super) fn notice_sheet(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let (lead, detail) = lead_and_detail(&self.ui.sheets.notice);
        let main = |label: SharedString| primary("alt-notice-way-out", label);
        let way_out = self.ui.sheets.way_out.clone().map(|way| match way {
            WayOut::MinecraftFolder => main(tr("Choose Minecraft folder").into()).on_click(
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.close_sheet();
                    this.choose_folder(cx);
                    cx.notify();
                }),
            ),
            WayOut::ProfileFolder(home) => {
                main(tr("Open folder").into()).on_click(move |_, _, cx| {
                    toast::open(cx, &home);
                })
            }
            WayOut::Newest(channel) => main(match channel {
                instances::Channel::Stable => tr("Use the newest Stable").into(),
                instances::Channel::Nightly => tr("Use the newest Nightly").into(),
                instances::Channel::Dev => tr("Use the newest Dev").into(),
            })
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.native_error = None;
                // The chooser opens under Home's build picker if this switch is canceled.
                this.navigate(crate::frame::Page::Home);
                this.open_sheet(Sheet::Build);
                this.switch_channel(channel);
                cx.notify();
            })),
        });
        let body = div()
            .flex()
            .flex_col()
            .child(title(tr("What went wrong").into()))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .children(lead)
                    .children(detail),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        secondary("alt-notice-close", tr("Close")).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.close_sheet();
                                cx.notify();
                            },
                        )),
                    )
                    .children(way_out),
            );
        frame("alt-notice", 560., body, cx)
    }
}
