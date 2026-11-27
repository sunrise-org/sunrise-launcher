//! Settings › Privacy: what Sunrise sends, and what it never does.
use super::*;
use crate::widgets::button::secondary;

impl Launcher {
    pub(super) fn settings_privacy(&mut self, cx: &mut Context<Self>) -> Div {
        let consent = self.collection.consent.clone();
        // A row: its name with a "?" that says exactly what is sent, a line on why, the switch.
        let line = |this: &Self,
                    name: &'static str,
                    summary: &'static str,
                    full: &'static str,
                    toggle: Stateful<Div>,
                    cx: &mut Context<Self>| {
            let pinned = this.collection.sent_open.contains(&name);
            let shown = pinned || this.collection.sent_hover == Some(name);
            let help = div()
                .id(name)
                .tab_index(0)
                .seen()
                .flex_none()
                .size(px(16.))
                .rounded_full()
                .border_1()
                .border_color(rgb(if pinned {
                    palette().text
                } else {
                    palette().line
                }))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(tokens::CAPTION))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(if pinned {
                    palette().text
                } else {
                    palette().muted
                }))
                .cursor_pointer()
                .focus_visible(|s| s.border_color(rgb(palette().focus)))
                // One line the size of the ring's inside, its text centred: the "?" sits in the
                // middle both ways, whatever width its text element measures.
                .child(
                    div()
                        .w_full()
                        .text_center()
                        .line_height(px(16. - 2.))
                        .child("?"),
                )
                .on_hover(cx.listener(move |this, over: &bool, _, cx| {
                    let hover = &mut this.collection.sent_hover;
                    match *over {
                        true => *hover = Some(name),
                        false if *hover == Some(name) => *hover = None,
                        false => {}
                    }
                    cx.notify();
                }))
                // A press pins it open, for keyboards and touch; another press closes it.
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    let list = &mut this.collection.sent_open;
                    match list.contains(&name) {
                        true => list.clear(),
                        false => *list = vec![name],
                    }
                    cx.notify();
                }));
            let said = shown.then(|| {
                // Under the "?", or above its row when the row is low in the window,
                // but never on top of the "?": covering it ends the hover, which hides
                // the card, which uncovers the "?" again, and the card flickers.
                // The anchor sits just above the row. The offset puts the card below
                // the "?"; when that does not fit, gpui flips the card to end at the
                // anchor (dropping the offset), so it ends above the row.
                // The side padding keeps the card off the window edge when it is shifted.
                gpui::deferred(
                    div().absolute().top(px(-6.)).left(px(-8.)).child(
                        gpui::anchored().offset(gpui::point(px(0.), px(30.))).child(
                            div().px(px(8.)).child(
                                div()
                                    .w(px(340.))
                                    .p(px(12.))
                                    .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                                    .border_1()
                                    .border_color(palette().input_edge())
                                    .bg(rgb(palette().raised))
                                    .shadow_lg()
                                    .occlude()
                                    .child(hint(tr(full))),
                            ),
                        ),
                    ),
                )
                .with_priority(10)
            });
            div()
                .flex()
                // The switch in the middle of its row, as everywhere else in Settings.
                .items_center()
                .gap(px(16.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .relative()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(div().font_weight(FontWeight::SEMIBOLD).child(tr(name)))
                                .child(help)
                                .children(said),
                        )
                        .child(hint(tr(summary))),
                )
                .child(toggle)
        };
        let ping = line(
            self,
            telemetry::PING_NAME,
            telemetry::PING_SUMMARY,
            telemetry::PING,
            switch("alt-ping", !self.settings.ping_off).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    this.settings.ping_off = !this.settings.ping_off;
                    this.save_settings();
                    cx.notify();
                },
            )),
            cx,
        );
        // The game reads the same switch (`backend::no_ai`).
        let no_ai_on = backend::no_ai();
        let no_ai = line(
            self,
            "No AI",
            "Hides and turns off every AI feature.",
            "Every AI feature in the launcher and the game is hidden and turned off: agent control of the launcher and the game, sunrise-ctl and the MCP servers.",
            switch("alt-no-ai", no_ai_on).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    // A failed save is a problem like any other: the play bar says it.
                    if let Err(e) = backend::set_no_ai(!no_ai_on) {
                        this.client_error = Some(e);
                    }
                    cx.notify();
                },
            )),
            cx,
        );
        let mut reports = Vec::new();
        for (n, &(kind, name, summary, what)) in telemetry::SWITCHES.iter().enumerate() {
            let on = !consent.needs_asking() && consent.choices.allows(kind);
            let toggle = switch(ElementId::named_usize("alt-report", n), on).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    let mut choices = if this.collection.consent.needs_asking() {
                        Choices::NONE
                    } else {
                        this.collection.consent.choices
                    };
                    choices.set(kind, !on);
                    this.answer_consent(choices);
                    cx.notify();
                },
            ));
            reports.push(line(self, name, summary, what, toggle, cx));
        }
        let id = match consent.install_id.as_str() {
            "" => tr("No install ID: nothing has been collected.").to_string(),
            id => trf(
                "Install ID {prefix}…{suffix}",
                &[("prefix", &id[..4]), ("suffix", &id[id.len() - 4..])],
            ),
        };
        let waiting = if sunrise_launcher_telemetry::endpoint().is_some() {
            let count = sunrise_launcher_telemetry::outbox::pending(&telemetry::home()).len();
            trf(
                "{count} waiting to be sent",
                &[("count", &count.to_string())],
            )
        } else {
            tr("Sending is turned off on this computer").to_string()
        };
        let confirm = self.collection.confirm_delete;
        let viewing = self.collection.view.is_some();
        let mut yours = self.wide_row(
            tr("Your data"),
            format!("{id} · {waiting}"),
            div()
                .child(
                    secondary(
                        "alt-data-view",
                        tr(if viewing {
                            "Hide"
                        } else {
                            "See what would be sent"
                        }),
                    )
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.toggle_view();
                        cx.notify();
                    })),
                )
                .child(
                    danger(secondary(
                        "alt-data-delete",
                        tr(if confirm {
                            "Delete everything?"
                        } else {
                            "Delete my data"
                        }),
                    ))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        if this.collection.confirm_delete {
                            this.collection.note = Some(match telemetry::delete_my_data() {
                                Ok(done) => done,
                                Err(e) => e,
                            });
                            this.collection.consent = telemetry::consent();
                            this.collection.view = None;
                        }
                        this.collection.confirm_delete = !this.collection.confirm_delete;
                        cx.notify();
                    })),
                ),
        );
        if let Some((what, json)) = self.collection.view.clone() {
            yours = div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(yours)
                .child(hint(what))
                .child(crate::code_box::code_box(
                    "alt-data-json",
                    vec![json.into()],
                    Some(220.),
                ));
        }
        if let Some(note) = self.collection.note.clone() {
            yours = div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(yours)
                .child(hint(note));
        }
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(group(vec![ping]))
            .child(hint(tr("These reports are off until you turn them on.")))
            .child(group(reports))
            .child(group(vec![yours]))
            .child(hint(tr(telemetry::NEVER)))
            .child(group(vec![no_ai]))
    }
}
