//! The unreviewed-code warning, for an added source and for a build's native mods nobody reviewed.

use super::*;
use crate::widgets::button::{primary, secondary};

impl Launcher {
    pub(super) fn unreviewed_warning_sheet(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let cancel_focus = self.unreviewed_cancel_focus(cx);
        if self.github.warn_needs_focus {
            self.github.warn_needs_focus = false;
            window.focus(&cancel_focus, cx);
        }
        let Stage::Warn(pending) = &self.github.stage else {
            unreachable!("only called for Stage::Warn");
        };
        let badge = pending.badge();
        let multiple = pending.count() > 1;
        let warn = Hsla::from(rgb(if pending.unsigned_local_file() {
            palette().danger
        } else {
            palette().warning
        }));
        let text = UnreviewedText {
            multiple,
            provenance: pending.provenance(),
            safety_body: if pending.any_native() {
                tr(NATIVE_DANGER).into()
            } else {
                tr("Unreviewed mod files have not been checked by Sunrise and can change how your game behaves.").into()
            },
            trust_line: trf(
                "Only continue if you wrote this mod yourself or trust the person who gave it to you. It will be labelled {badge} and will not receive automatic updates.",
                &[("badge", badge.label())],
            ),
            confirm_label: if multiple {
                tr("Install unreviewed mods")
            } else {
                tr("Install unreviewed mod")
            }
            .into(),
        };
        let body = unreviewed_warning(
            text,
            warn,
            ["add-source-confirm", "add-source-cancel"],
            &cancel_focus,
            Self::handle_warning_action,
            cx,
        );
        sheets::warning_frame("github-add-sheet", 560., warn, body, cx)
    }

    /// The warning a build shows before it compiles native mods that neither came verified from
    /// the registry nor were confirmed, such as a file copied into the mods folder by hand.
    pub(crate) fn unreviewed_build_sheet(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        let cancel_focus = self.unreviewed_cancel_focus(cx);
        let pending = self.ui.sheets.unreviewed.as_mut()?;
        if std::mem::take(&mut pending.needs_focus) {
            window.focus(&cancel_focus, cx);
        }
        let multiple = pending.found.len() > 1;
        let files = pending
            .found
            .iter()
            .map(|p| trf("{mod} ({file})", &[("mod", &p.name), ("file", &p.file)]))
            .collect::<Vec<_>>()
            .join(", ");
        let text = UnreviewedText {
            multiple,
            provenance: WarningProvenance {
                icon: "warning",
                headline: if multiple {
                    tr("Local files, not reviewed")
                } else {
                    tr("Local file, not reviewed")
                }
                .into(),
                detail: trf(
                    "{files} · These files are in this profile's mods folder, but did not come from the Sunrise registry. Sunrise cannot verify who built them or where they came from.",
                    &[("files", &files)],
                ),
            },
            safety_body: tr(NATIVE_DANGER).into(),
            trust_line: tr("Only continue if you wrote this code yourself or trust the person who gave it to you. You are asked once for each file.").into(),
            confirm_label: if multiple {
                tr("Build unreviewed mods")
            } else {
                tr("Build unreviewed mod")
            }
            .into(),
        };
        let warn = Hsla::from(rgb(palette().danger));
        let body = unreviewed_warning(
            text,
            warn,
            ["unreviewed-build-confirm", "unreviewed-build-cancel"],
            &cancel_focus,
            Self::answer_unreviewed_build,
            cx,
        );
        Some(sheets::warning_frame(
            "unreviewed-build-sheet",
            560.,
            warn,
            body,
            cx,
        ))
    }

    /// The build's question is answered: a confirm remembers these files, so the next build
    /// takes them without asking.
    fn answer_unreviewed_build(&mut self, action: WarningAction) {
        let Some(pending) = self.ui.sheets.unreviewed.take() else {
            return;
        };
        let confirmed = action == WarningAction::Confirm
            && match backend::allow_native_files(pending.found.into_iter().map(|p| p.sha256)) {
                Ok(()) => true,
                Err(error) => {
                    self.open_notice(error, None);
                    false
                }
            };
        let _ = pending.reply.send(confirmed);
    }

    fn unreviewed_cancel_focus(&mut self, cx: &mut Context<Self>) -> FocusHandle {
        self.github
            .cancel_focus
            .get_or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true))
            .clone()
    }
}

const NATIVE_DANGER: &str = crate::i18n::tr_key("Native mod code is compiled and runs directly on this computer with your permissions — it can read, change, or delete your files.");

/// A build's question about native mods nobody reviewed, waiting for the player.
pub(crate) struct PendingUnreviewed {
    found: Vec<sunrise_launcher::native_mods::Unreviewed>,
    reply: std::sync::mpsc::Sender<bool>,
    needs_focus: bool,
}

impl PendingUnreviewed {
    pub(crate) fn new(
        found: Vec<sunrise_launcher::native_mods::Unreviewed>,
        reply: std::sync::mpsc::Sender<bool>,
    ) -> Self {
        Self {
            found,
            reply,
            needs_focus: true,
        }
    }

    /// Answers no: nothing unreviewed is built.
    pub(crate) fn refuse(self) {
        let _ = self.reply.send(false);
    }
}

/// What the unreviewed-code warning says, beyond its fixed lines.
struct UnreviewedText {
    multiple: bool,
    provenance: WarningProvenance,
    safety_body: String,
    trust_line: String,
    confirm_label: String,
}

/// The unreviewed-code warning: Cancel has the keyboard and is the default, and confirming
/// takes the explicit button. `answer` gets the player's choice.
fn unreviewed_warning(
    text: UnreviewedText,
    warn: Hsla,
    [confirm_id, cancel_id]: [&'static str; 2],
    cancel_focus: &FocusHandle,
    answer: fn(&mut Launcher, WarningAction),
    cx: &mut Context<Launcher>,
) -> Div {
    let UnreviewedText {
        multiple,
        provenance: prov,
        safety_body,
        trust_line,
        confirm_label,
    } = text;
    let heading = if multiple {
        tr("These mods have not been reviewed")
    } else {
        tr("This mod has not been reviewed")
    };
    let safety_lead = if multiple {
        tr("Nobody has checked these mods for safety.")
    } else {
        tr("Nobody has checked this mod for safety.")
    };
    let header = div()
        .flex()
        .items_center()
        .gap(px(14.))
        .px(px(24.))
        .py(px(18.))
        .bg(warn.opacity(0.22))
        .border_b_1()
        .border_color(warn.opacity(0.45))
        .child(
            div()
                .size(px(44.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(crate::tokens::r(tokens::R_CARD))
                .bg(warn.opacity(0.2))
                .border_1()
                .border_color(warn.opacity(0.55))
                .child(blocks::icon("warning", 22., warn)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .min_w_0()
                .child(
                    div()
                        .text_size(px(tokens::CAPTION))
                        .font_weight(FontWeight::BOLD)
                        .text_color(warn)
                        .child(tr("UNREVIEWED CODE WARNING")),
                )
                .child(
                    div()
                        .text_size(px(tokens::TITLE))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(palette().text))
                        .child(heading),
                ),
        );

    let provenance_card = div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .p(px(14.))
        .rounded(crate::tokens::r(tokens::R_CARD))
        .bg(warn.opacity(0.10))
        .border_1()
        .border_color(warn.opacity(0.45))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(blocks::icon(prov.icon, 16., warn))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(palette().text))
                        .child(prov.headline),
                ),
        )
        .child(
            div()
                .text_size(px(tokens::SMALL))
                .text_color(rgb(palette().muted))
                .child(prov.detail),
        );

    let confirm = secondary(confirm_id, confirm_label)
        .border_color(warn.opacity(0.65))
        .bg(warn.opacity(0.12))
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            answer(this, WarningAction::Confirm);
            cx.notify();
        }))
        .on_key_down(cx.listener(move |this, e: &KeyDownEvent, _, cx| {
            if matches!(e.keystroke.key.as_str(), "enter" | "space") {
                answer(this, WarningAction::Confirm);
                cx.stop_propagation();
                cx.notify();
            }
        }));

    let cancel = primary(cancel_id, tr("Cancel"))
        .track_focus(cancel_focus)
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            answer(this, WarningAction::Cancel);
            cx.notify();
        }))
        .on_key_down(cx.listener(move |this, e: &KeyDownEvent, _, cx| {
            if matches!(e.keystroke.key.as_str(), "enter" | "space") {
                answer(this, WarningAction::Cancel);
                cx.stop_propagation();
                cx.notify();
            }
        }));

    let content = div()
        .flex()
        .flex_col()
        .gap(px(16.))
        .p(px(24.))
        .child(provenance_card)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(palette().text))
                        .child(safety_lead),
                )
                .child(
                    div()
                        .text_size(px(tokens::LABEL))
                        .text_color(rgb(palette().muted))
                        .child(safety_body),
                ),
        )
        .child(
            div()
                .text_size(px(tokens::LABEL))
                .text_color(rgb(palette().muted))
                .child(trust_line),
        )
        .child(
            div()
                .flex()
                .justify_end()
                .gap(px(10.))
                .child(confirm)
                .child(cancel),
        );

    div()
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(header)
        .child(content)
        .on_key_down(cx.listener(move |this, e: &KeyDownEvent, _, cx| {
            if e.keystroke.key == "enter" {
                answer(this, WarningAction::DEFAULT);
                cx.stop_propagation();
                cx.notify();
            }
        }))
}
