//! Asked when a mod needs a dependency that several reviewed v2 projects provide. The launcher
//! never picks one: the player chooses, or cancels and nothing is installed.
use super::*;
use crate::widgets::button::secondary;
use sunrise_launcher::browser::sunrise::ProviderChoiceNeeded;

pub(super) struct PendingProviderChoice {
    pub(super) need: Box<ProviderChoiceNeeded>,
    /// The chosen project ID, or `None` to cancel.
    pub(super) reply: Sender<Option<String>>,
}

impl Launcher {
    pub(crate) fn provider_choice_question(
        &mut self,
        need: Box<ProviderChoiceNeeded>,
        reply: Sender<Option<String>>,
    ) {
        if let Some(old) = self
            .ui
            .sheets
            .provider_choice
            .replace(PendingProviderChoice { need, reply })
        {
            let _ = old.reply.send(None);
        }
    }

    /// Answers a pending choice; true when one was open.
    pub(super) fn answer_provider_choice(&mut self, project_id: Option<String>) -> bool {
        let Some(pending) = self.ui.sheets.provider_choice.take() else {
            return false;
        };
        let _ = pending.reply.send(project_id);
        true
    }

    pub(super) fn provider_choice_sheet(
        &self,
        need: &ProviderChoiceNeeded,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let mut list = card().overflow_hidden();
        for (i, candidate) in need.candidates.iter().enumerate() {
            let project_id = candidate.project_id.clone();
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .p(px(14.))
                    .when(i > 0, |row| {
                        row.border_t_1().border_color(rgb(palette().divider))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .text_size(px(tokens::BODY))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette().text))
                                    .child(crate::selectable::selectable(candidate.name.clone())),
                            )
                            .child(muted(trf(
                                "github.com/{repo} · {version}",
                                &[
                                    ("repo", &candidate.repository_name),
                                    ("version", &candidate.version),
                                ],
                            ))),
                    )
                    .child(
                        secondary(format!("provider-choice-use-{}", i), tr("Use this")).on_click(
                            cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.answer_provider_choice(Some(project_id.clone()));
                                cx.notify();
                            }),
                        ),
                    ),
            );
        }
        let body = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(title(trf(
                "Choose which mod provides {id}",
                &[("id", &need.mod_id)],
            )))
            .child(muted(trf(
                "{mod} needs {id}, and several reviewed mods provide it. Only one can be installed.",
                &[("mod", &need.required_by), ("id", &need.mod_id)],
            )))
            .child(list)
            .child(
                div().flex().justify_end().child(
                    secondary("provider-choice-cancel", tr("Cancel")).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.answer_provider_choice(None);
                            cx.notify();
                        },
                    )),
                ),
            );
        frame("provider-choice-sheet", 560., body, cx)
    }
}
