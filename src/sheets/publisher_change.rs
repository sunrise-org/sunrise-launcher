//! Publisher-change confirmation sheet (mock 09).
//!
//! Shown when a verified v2 release comes from a different GitHub repository ID or build
//! workflow path than the pin remembered at first install on this computer. `Keep current
//! version` is always the default action; closing, pressing Escape, or clicking outside keeps
//! the current version without changing the computer's remembered pin.
use super::*;
use crate::widgets::button::{primary, secondary};
use sunrise_launcher::browser::sunrise::PublisherChangeQuestion;

pub(super) struct PendingPublisherChange {
    pub(super) question: Box<PublisherChangeQuestion>,
    /// `true` accepts the new publisher.
    pub(super) reply: Sender<bool>,
}

/// Player-facing lines for a publisher-change prompt, derived once from [`PublisherChangeQuestion`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublisherChangeCopy {
    pub(crate) title: String,
    pub(crate) subtitle: String,
    pub(crate) explanation: String,
    pub(crate) remembered_heading: String,
    pub(crate) remembered_repo: String,
    pub(crate) remembered_workflow: String,
    pub(crate) incoming_heading: String,
    pub(crate) incoming_repo: String,
    pub(crate) incoming_workflow: String,
    pub(crate) accept_note: String,
    pub(crate) keep_note: String,
    pub(crate) accept_button: &'static str,
    pub(crate) keep_button: &'static str,
}

fn format_repo(name: &str, id: u64, disambiguate_id: bool) -> String {
    if disambiguate_id {
        trf(
            "github.com/{repo} (ID {id})",
            &[("repo", name), ("id", &id.to_string())],
        )
    } else {
        trf("github.com/{repo}", &[("repo", name)])
    }
}

/// Builds exact, plain copy describing what changed between the remembered publisher pin and
/// the incoming release.
pub(crate) fn publisher_change_copy(question: &PublisherChangeQuestion) -> PublisherChangeCopy {
    let change = &question.change;
    let repo_changed = change.repository_changed();
    let workflow_changed = change.workflow_changed();
    let same_name_different_id = repo_changed
        && change
            .pinned
            .repository_name
            .eq_ignore_ascii_case(&change.incoming.repository_name);

    let title = trf(
        "Publisher changed for {mod}",
        &[("mod", &question.mod_name)],
    );
    let subtitle = if question.current_version.is_some() {
        trf(
            "Update paused before installing version {version}",
            &[("version", &question.incoming_version)],
        )
    } else {
        trf(
            "Install paused before installing version {version}",
            &[("version", &question.incoming_version)],
        )
    };
    // A new owner is a new publisher, whether or not the workflow changed with it.
    let owner_changed = change.owner_changed();
    let explanation = match (repo_changed, workflow_changed) {
        (true, true) => trf(
            "Version {version} comes from a different GitHub repository and build workflow than the one remembered when you first installed {mod}.",
            &[
                ("version", &question.incoming_version),
                ("mod", &question.mod_name),
            ],
        ),
        (true, false) => trf(
            "Version {version} comes from a different GitHub repository than the one remembered when you first installed {mod}.",
            &[
                ("version", &question.incoming_version),
                ("mod", &question.mod_name),
            ],
        ),
        (false, true) if !owner_changed => trf(
            "Version {version} comes from a different build workflow than the one remembered when you first installed {mod}.",
            &[
                ("version", &question.incoming_version),
                ("mod", &question.mod_name),
            ],
        ),
        (false, _) => trf(
            "Version {version} comes from a different publisher than the one remembered when you first installed {mod}.",
            &[
                ("version", &question.incoming_version),
                ("mod", &question.mod_name),
            ],
        ),
    };
    let remembered_heading = match &question.current_version {
        Some(version) => trf(
            "REMEMBERED AT FIRST INSTALL (VERSION {version})",
            &[("version", version)],
        ),
        None => tr("REMEMBERED AT FIRST INSTALL").into(),
    };
    let incoming_heading = trf(
        "NEW RELEASE (VERSION {version})",
        &[("version", &question.incoming_version)],
    );
    let remembered_repo = format_repo(
        &change.pinned.repository_name,
        change.pinned.repository_id,
        same_name_different_id,
    );
    let incoming_repo = format_repo(
        &change.incoming.repository_name,
        change.incoming.repository_id,
        same_name_different_id,
    );
    let remembered_workflow = trf(
        "Workflow: {workflow}",
        &[("workflow", &change.pinned.workflow_path)],
    );
    let incoming_workflow = trf(
        "Workflow: {workflow}",
        &[("workflow", &change.incoming.workflow_path)],
    );
    let accept_note = if repo_changed {
        trf(
            "Accepting updates {mod} to {version} and remembers the new repository for future updates.",
            &[
                ("mod", &question.mod_name),
                ("version", &question.incoming_version),
            ],
        )
    } else if !workflow_changed || owner_changed {
        trf(
            "Accepting updates {mod} to {version} and remembers the new publisher for future updates.",
            &[
                ("mod", &question.mod_name),
                ("version", &question.incoming_version),
            ],
        )
    } else {
        trf(
            "Accepting updates {mod} to {version} and remembers the new build workflow for future updates.",
            &[
                ("mod", &question.mod_name),
                ("version", &question.incoming_version),
            ],
        )
    };
    let keep_note = match &question.current_version {
        Some(version) => trf(
            "Keeping your current version leaves {version} installed.",
            &[("version", version)],
        ),
        None => tr("Keeping your current state leaves this mod uninstalled.").into(),
    };

    PublisherChangeCopy {
        title,
        subtitle,
        explanation,
        remembered_heading,
        remembered_repo,
        remembered_workflow,
        incoming_heading,
        incoming_repo,
        incoming_workflow,
        accept_note,
        keep_note,
        accept_button: tr("Accept new publisher"),
        keep_button: tr("Keep current version"),
    }
}

pub(super) fn resolve_pending(slot: &mut Option<PendingPublisherChange>, accept: bool) -> bool {
    let Some(pending) = slot.take() else {
        return false;
    };
    let _ = pending.reply.send(accept);
    true
}

pub(super) fn dismiss_pending(slot: &mut Option<PendingPublisherChange>) -> bool {
    resolve_pending(slot, false)
}

pub(super) fn replace_pending(
    slot: &mut Option<PendingPublisherChange>,
    question: Box<PublisherChangeQuestion>,
    reply: Sender<bool>,
) {
    if let Some(old) = slot.replace(PendingPublisherChange { question, reply }) {
        let _ = old.reply.send(false);
    }
}

impl Launcher {
    pub(crate) fn publisher_change_question(
        &mut self,
        question: Box<PublisherChangeQuestion>,
        reply: Sender<bool>,
    ) {
        replace_pending(&mut self.ui.sheets.publisher_change, question, reply);
    }

    pub(super) fn dismiss_publisher_change(&mut self) -> bool {
        dismiss_pending(&mut self.ui.sheets.publisher_change)
    }

    pub(super) fn publisher_change_sheet(
        &self,
        question: &PublisherChangeQuestion,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let copy = publisher_change_copy(question);
        let warn = Hsla::from(rgb(palette().warning));
        let warn_tint = warn.opacity(0.11);
        let warn_edge = warn.opacity(0.45);

        let header = div()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(
                div()
                    .flex_none()
                    .size(px(40.))
                    .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                    .border_1()
                    .border_color(warn_edge)
                    .bg(warn_tint)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(crate::glass_ui::icon("shield", 20., warn)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(title(copy.title))
                    .child(
                        div()
                            .text_size(px(tokens::LABEL))
                            .text_color(warn)
                            .child(crate::selectable::selectable(copy.subtitle)),
                    ),
            );

        let comparison = card()
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .p(px(14.))
                    .child(
                        div()
                            .text_size(px(tokens::CAPTION))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette().muted))
                            .child(copy.remembered_heading),
                    )
                    .child(
                        div()
                            .text_size(px(tokens::BODY))
                            .font_weight(FontWeight::SEMIBOLD)
                            .font_family(crate::theme::MONO)
                            .text_color(rgb(palette().text))
                            .child(crate::selectable::selectable(copy.remembered_repo)),
                    )
                    .child(
                        div()
                            .text_size(px(tokens::SMALL))
                            .font_family(crate::theme::MONO)
                            .text_color(rgb(palette().muted))
                            .child(crate::selectable::selectable(copy.remembered_workflow)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .p(px(14.))
                    .border_t_1()
                    .border_color(warn_edge)
                    .bg(warn_tint)
                    .child(
                        div()
                            .text_size(px(tokens::CAPTION))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(warn)
                            .child(copy.incoming_heading),
                    )
                    .child(
                        div()
                            .text_size(px(tokens::BODY))
                            .font_weight(FontWeight::SEMIBOLD)
                            .font_family(crate::theme::MONO)
                            .text_color(rgb(palette().text))
                            .child(crate::selectable::selectable(copy.incoming_repo)),
                    )
                    .child(
                        div()
                            .text_size(px(tokens::SMALL))
                            .font_family(crate::theme::MONO)
                            .text_color(rgb(palette().muted))
                            .child(crate::selectable::selectable(copy.incoming_workflow)),
                    ),
            );

        let accept = secondary("publisher-change-accept", copy.accept_button)
            .border_color(warn_edge)
            .text_color(warn)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                resolve_pending(&mut this.ui.sheets.publisher_change, true);
                cx.notify();
            }));

        // Default safe action: keep current version.
        let keep = primary("publisher-change-keep", copy.keep_button).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                resolve_pending(&mut this.ui.sheets.publisher_change, false);
                cx.notify();
            },
        ));

        let body = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(header)
            .child(
                div()
                    .text_size(px(tokens::BODY))
                    .text_color(rgb(palette().text))
                    .child(crate::selectable::selectable(copy.explanation)),
            )
            .child(comparison)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(muted(copy.accept_note))
                    .child(muted(copy.keep_note)),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(accept)
                    .child(keep),
            );

        frame("publisher-change-sheet", 580., body, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use sunrise_launcher::attestation::{PublisherChange, PublisherPin};

    fn sample_question(
        old_id: u64,
        old_repo: &str,
        old_wf: &str,
        new_id: u64,
        new_repo: &str,
        new_wf: &str,
    ) -> PublisherChangeQuestion {
        PublisherChangeQuestion {
            mod_name: "Frontier".into(),
            current_version: Some("1.2.0".into()),
            incoming_version: "1.3.0".into(),
            change: PublisherChange {
                project_id: "frnt1234".into(),
                pinned: PublisherPin {
                    owner_id: None,
                    repository_id: old_id,
                    repository_name: old_repo.into(),
                    workflow_path: old_wf.into(),
                },
                incoming: PublisherPin {
                    owner_id: None,
                    repository_id: new_id,
                    repository_name: new_repo.into(),
                    workflow_path: new_wf.into(),
                },
            },
        }
    }

    #[test]
    fn dismissing_a_publisher_change_keeps_the_current_version() {
        let question = sample_question(
            100,
            "Zekrenlight/frontier",
            ".github/workflows/release.yml",
            200,
            "frontier-mod/sunrise-frontier",
            ".github/workflows/sunrise-release.yml",
        );
        let (tx, rx) = channel();
        let mut slot = None;
        replace_pending(&mut slot, Box::new(question), tx);
        assert!(dismiss_pending(&mut slot));
        assert!(slot.is_none());
        assert!(!rx.recv().unwrap());
        assert!(!dismiss_pending(&mut slot));
    }

    #[test]
    fn publisher_change_explicit_accept_sends_accept_new_publisher() {
        let question = sample_question(
            100,
            "Zekrenlight/frontier",
            ".github/workflows/release.yml",
            200,
            "frontier-mod/sunrise-frontier",
            ".github/workflows/release.yml",
        );
        let (tx, rx) = channel();
        let mut slot = None;
        replace_pending(&mut slot, Box::new(question), tx);
        assert!(resolve_pending(&mut slot, true));
        assert!(slot.is_none());
        assert!(rx.recv().unwrap());
    }

    #[test]
    fn publisher_change_replacing_pending_prompt_answers_keep_current_to_old() {
        let q1 = sample_question(100, "a/mod", "wf-a.yml", 200, "b/mod", "wf-b.yml");
        let q2 = sample_question(300, "c/mod", "wf-c.yml", 400, "d/mod", "wf-d.yml");
        let (tx1, rx1) = channel();
        let (tx2, rx2) = channel();
        let mut slot = None;
        replace_pending(&mut slot, Box::new(q1), tx1);
        replace_pending(&mut slot, Box::new(q2), tx2);
        assert!(!rx1.recv().unwrap());
        assert!(slot.is_some());
        assert!(dismiss_pending(&mut slot));
        assert!(!rx2.recv().unwrap());
    }

    #[test]
    fn publisher_change_copy_describes_repo_and_workflow_changes_and_disambiguates_same_name() {
        let both = publisher_change_copy(&sample_question(
            100,
            "Zekrenlight/frontier",
            ".github/workflows/release.yml",
            200,
            "frontier-mod/sunrise-frontier",
            ".github/workflows/sunrise-release.yml",
        ));
        assert_eq!(both.title, "Publisher changed for Frontier");
        assert!(both.explanation.contains("repository and build workflow"));
        assert_eq!(both.remembered_repo, "github.com/Zekrenlight/frontier");
        assert_eq!(
            both.incoming_repo,
            "github.com/frontier-mod/sunrise-frontier"
        );
        assert_eq!(both.keep_button, "Keep current version");
        assert_eq!(both.accept_button, "Accept new publisher");

        let workflow_only = publisher_change_copy(&sample_question(
            100,
            "Zekrenlight/frontier",
            ".github/workflows/old.yml",
            100,
            "Zekrenlight/frontier",
            ".github/workflows/new.yml",
        ));
        assert!(workflow_only
            .explanation
            .contains("different build workflow"));
        assert!(!workflow_only
            .explanation
            .contains("repository and build workflow"));
        assert!(workflow_only.accept_note.contains("new build workflow"));

        let recreated = publisher_change_copy(&sample_question(
            100,
            "Zekrenlight/frontier",
            ".github/workflows/release.yml",
            999,
            "Zekrenlight/frontier",
            ".github/workflows/release.yml",
        ));
        assert!(recreated.remembered_repo.contains("100"));
        assert!(recreated.incoming_repo.contains("999"));
        assert_ne!(recreated.remembered_repo, recreated.incoming_repo);

        // A repository transferred to another owner keeps its ID and workflow: the note names
        // the publisher, not a workflow that did not change.
        let mut transferred = sample_question(
            100,
            "Zekrenlight/frontier",
            ".github/workflows/release.yml",
            100,
            "someone/frontier",
            ".github/workflows/release.yml",
        );
        transferred.change.pinned.owner_id = Some(1);
        transferred.change.incoming.owner_id = Some(2);
        let transferred = publisher_change_copy(&transferred);
        assert!(transferred.explanation.contains("different publisher"));
        assert!(transferred.accept_note.contains("new publisher"));

        // Transferred and given a new workflow in the same release: still a new publisher.
        let mut moved = sample_question(
            100,
            "Zekrenlight/frontier",
            ".github/workflows/old.yml",
            100,
            "someone/frontier",
            ".github/workflows/new.yml",
        );
        moved.change.pinned.owner_id = Some(1);
        moved.change.incoming.owner_id = Some(2);
        let moved = publisher_change_copy(&moved);
        assert!(moved.explanation.contains("different publisher"));
        assert!(moved.accept_note.contains("new publisher"));
    }
}
