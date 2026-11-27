//! A refused Add or Update, said once, in one modal: what it means for the player first, by the
//! mod's name; the checker's own words and the build log behind Show details.
use super::*;
use crate::frame::c;
use crate::widgets::button::{primary, secondary};
use std::sync::Arc;
use sunrise_launcher::browser::sunrise::{sources::Failure, SourceError};

/// Why one mod couldn't be added or updated.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Refusal {
    /// What it means for the player.
    pub(crate) said: String,
    /// The checker's words, for whoever wants them: empty when the line says it all.
    pub(crate) details: String,
    pub(crate) log: Option<Arc<str>>,
}

impl Refusal {
    /// The refusal of `error` for the mod people know as `name`; none for a cancelled run.
    pub(crate) fn of(name: &str, error: &SourceError) -> Option<Self> {
        let named = |text: &str| trf(text, &[("mod", name)]);
        let (said, log) = match error {
            SourceError::Cancelled => return None,
            SourceError::DoesNotFit(_) => (
                named("{mod} isn't ready for your game version yet. Its author needs to update it for this build."),
                None,
            ),
            SourceError::Catalogue(why) => return Some(Self {
                said: why.clone(),
                details: String::new(),
                log: None,
            }),
            SourceError::Dependency { .. } => (
                named("{mod} can't be added together with the mods this profile has."),
                None,
            ),
            SourceError::Changed(_) => (
                named("The profile or its game build changed while {mod} was being checked. Nothing was changed; try again."),
                None,
            ),
            SourceError::Tools(_) => (
                named("Sunrise couldn't download the tools it needs to build {mod}. Check your connection and try again."),
                None,
            ),
            SourceError::Build { log, .. } => {
                (named("{mod} couldn't be built on this computer."), log.clone())
            }
            SourceError::Github(Failure::Offline(_)) => {
                (tr("You're offline. Connect and try again.").into(), None)
            }
            SourceError::Github(
                failure @ (Failure::RepositoryUnavailable
                | Failure::RefUnavailable
                | Failure::RateLimited { .. }
                | Failure::NoManifest),
            ) => {
                return Some(Self {
                    said: trf(
                        "{mod} couldn't be read from GitHub. {why}",
                        &[("mod", name), ("why", &SourceError::Github(failure.clone()).to_string())],
                    ),
                    details: String::new(),
                    log: None,
                });
            }
            SourceError::Github(Failure::Other(_)) => {
                (named("{mod} couldn't be read from GitHub."), None)
            }
            SourceError::DependencyGithub {
                name: dependency,
                failure,
            } => return Self::of(dependency, &SourceError::Github(failure.clone())),
            SourceError::Unproved(_) => (named("Sunrise couldn't add {mod}."), None),
        };
        Some(Self {
            said,
            details: error.to_string(),
            log,
        })
    }
}

/// One mod in the modal.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RefusedMod {
    pub(crate) name: String,
    /// `owner/repo`, for Open on GitHub.
    pub(crate) repo: Option<String>,
    pub(crate) refusal: Refusal,
}

/// What the modal shows: the mods an Add or Update refused, and whether the details are open.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Refused {
    pub(crate) update: bool,
    /// Every mod the run asked for, for the title.
    pub(crate) names: Vec<String>,
    pub(crate) mods: Vec<RefusedMod>,
    pub(crate) details: bool,
}

impl Refused {
    pub(crate) fn one(
        name: &str,
        repo: Option<String>,
        update: bool,
        error: &SourceError,
    ) -> Option<Self> {
        Some(Self {
            update,
            names: vec![name.to_owned()],
            mods: vec![RefusedMod {
                name: name.to_owned(),
                repo,
                refusal: Refusal::of(name, error)?,
            }],
            details: false,
        })
    }

    fn title(&self) -> String {
        let mods = crate::mods::names(&self.names);
        if self.update {
            trf("Couldn't update {mods}", &[("mods", &mods)])
        } else {
            trf("Couldn't add {mods}", &[("mods", &mods)])
        }
    }
}

/// The checker's words in the box every refusal shows them in, behind Show details.
pub(crate) fn details_box(id: impl Into<ElementId>, text: String) -> Stateful<Div> {
    div()
        .id(id)
        .max_h(px(180.))
        .overflow_y_scroll()
        .p(px(10.))
        .rounded(crate::tokens::r(tokens::R_SMALL))
        .bg(c(palette().background))
        .text_size(px(tokens::LABEL))
        .text_color(c(palette().muted))
        .child(crate::selectable::selectable(text))
}

/// The one label of the button that opens or closes those words.
pub(crate) fn details_label(open: bool) -> &'static str {
    if open {
        tr("Hide details")
    } else {
        tr("Show details")
    }
}

impl Launcher {
    pub(crate) fn open_refused(&mut self, refused: Refused) {
        self.mods_state.refused = Some(refused);
        self.open_sheet(crate::sheets::Sheet::Refused);
    }

    /// The small line under a mod's button that reopens why its last Add or Update was refused.
    pub(crate) fn refused_line(
        &self,
        id: impl Into<ElementId>,
        refused: Refused,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let label = if refused.update {
            tr("Couldn't update · See why")
        } else {
            tr("Couldn't add · See why")
        };
        div()
            .id(id)
            .reach()
            .cursor_pointer()
            .text_size(px(tokens::LABEL))
            .text_color(c(palette().warning))
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                // A collection row under it toggles its tick on a click.
                cx.stop_propagation();
                this.open_refused(refused.clone());
                cx.notify();
            }))
    }

    pub(crate) fn refused_sheet(&mut self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let refused = self.mods_state.refused.as_ref()?;
        let many = refused.mods.len() > 1;
        let mut list = div().flex().flex_col().gap(px(12.));
        for (i, refused_mod) in refused.mods.iter().enumerate() {
            let refusal = &refused_mod.refusal;
            list =
                list.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .children(many.then(|| {
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(refused_mod.name.clone())
                        }))
                        .child(
                            div()
                                .text_size(px(tokens::BODY))
                                .child(refusal.said.clone()),
                        )
                        .children(
                            (refused.details && !refusal.details.is_empty()).then(|| {
                                details_box(("refused-details", i), refusal.details.clone())
                            }),
                        )
                        .children(refused.details.then(|| refusal.log.clone()).flatten().map(
                            |log| {
                                div().flex().child(self.nightly_log_button(
                                    &refused_mod.name,
                                    log,
                                    cx,
                                ))
                            },
                        ))
                        .children(
                            many.then(|| refused_mod.repo.clone())
                                .flatten()
                                .map(|repo| {
                                    div()
                                        .flex()
                                        .child(github_button(("refused-github", i), repo))
                                }),
                        ),
                );
        }
        let has_details = refused
            .mods
            .iter()
            .any(|m| !m.refusal.details.is_empty() || m.refusal.log.is_some());
        let single_repo = (!many)
            .then(|| refused.mods.first()?.repo.clone())
            .flatten();
        let body = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(crate::sheets::title(refused.title()))
            .child(
                div()
                    .id("refused-list")
                    .max_h(px(420.))
                    .overflow_y_scroll()
                    .child(list),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(8.))
                    .children(has_details.then(|| {
                        secondary("refused-show-details", details_label(refused.details)).on_click(
                            cx.listener(|this, _, _, cx| {
                                if let Some(refused) = &mut this.mods_state.refused {
                                    refused.details ^= true;
                                }
                                cx.notify();
                            }),
                        )
                    }))
                    .child(div().flex_1())
                    .children(single_repo.map(|repo| github_button("refused-github-one", repo)))
                    .child(primary("refused-close", tr("Close")).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.close_sheet();
                            cx.notify();
                        },
                    ))),
            );
        Some(crate::sheets::frame("refused", 560., body, cx))
    }
}

fn github_button(id: impl Into<ElementId>, repo: String) -> Stateful<Div> {
    let url = format!("https://github.com/{repo}");
    secondary(id, tr("Open on GitHub"))
        .gap(px(5.))
        .child("↗")
        .on_click(move |_, _, cx| cx.open_url(&url))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_says_what_it_means_by_name_and_keeps_the_checkers_words_for_details() {
        let checker = "jade pins build ed3ac19 and its package carries no list; run `cargo sunrise-mod build`";
        let refusal = Refusal::of("Jade", &SourceError::DoesNotFit(checker.into())).unwrap();
        assert_eq!(
            refusal.said,
            "Jade isn't ready for your game version yet. Its author needs to update it for this build."
        );
        assert_eq!(refusal.details, checker);

        // A raw error, a path included, never reaches the line people read.
        let raw = "Permission denied: /home/someone/.local/share/sunrise/profiles/a";
        let refusal = Refusal::of("Jade", &SourceError::Unproved(raw.into())).unwrap();
        assert_eq!(refusal.said, "Sunrise couldn't add Jade.");
        assert_eq!(refusal.details, raw);

        let log: Arc<str> = "error[E0425]".into();
        let built = Refusal::of(
            "Jade",
            &SourceError::Build {
                detail: "cargo failed".into(),
                log: Some(log.clone()),
                failed: Vec::new(),
            },
        )
        .unwrap();
        assert_eq!(built.said, "Jade couldn't be built on this computer.");
        assert_eq!(built.log, Some(log));

        assert_eq!(Refusal::of("Jade", &SourceError::Cancelled), None);

        let dep_missing = Refusal::of(
            "Corn Delight",
            &SourceError::DependencyGithub {
                name: "Farmer's Delight".into(),
                failure: Failure::NoManifest,
            },
        )
        .unwrap();
        assert_eq!(
            dep_missing.said,
            "Farmer's Delight couldn't be read from GitHub. No Sunrise mod at this commit."
        );
        assert!(dep_missing.details.is_empty());

        let dep_repo = Refusal::of(
            "Corn Delight",
            &SourceError::DependencyGithub {
                name: "Farmer's Delight".into(),
                failure: Failure::RepositoryUnavailable,
            },
        )
        .unwrap();
        assert_eq!(
            dep_repo.said,
            "Farmer's Delight couldn't be read from GitHub. This repository is private, removed or moved."
        );
    }
}
