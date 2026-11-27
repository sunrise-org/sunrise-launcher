//! The pinned source detail. GitHub content is data; it never selects the profile or executes code.
use super::super::project::look::{action_column, card, head, note};
use super::super::project::{about_row, back};
use super::*;
use crate::mods_page::install::{stage_label, SourceQuestion};
use crate::widgets::button::{primary, secondary};
use crate::widgets::chip::{self, tag};
use sunrise_launcher::browser::sunrise::SourceError;

impl Launcher {
    pub(in crate::mods) fn nightly_project(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let state = &self.mods_state.nightly;
        let open = state.open.as_ref().expect("a Nightly detail is open");
        let source = &open.source;
        let inspection = open.inspection.as_ref();
        let details = inspection.map(|i| &i.data);
        let commit = details.and_then(|d| d.commit.as_ref());
        let failure = details
            .and_then(|detail| detail.error.clone())
            .map(SourceError::Github);
        let rows = self.discover_installed();
        let presence = rows.map(|rows| rows.source_presence(source));
        let pending = rows.is_some_and(|rows| rows.unbuilt.contains(source.id()));
        let (installed, installed_file) = match presence {
            Some(SourcePresence::Installed { origin, file }) => (Some(origin), Some(file)),
            _ => (None, None),
        };
        let missing = presence == Some(SourcePresence::Missing);
        let conflict = presence == Some(SourcePresence::Conflict);
        let failed_job = (!conflict && presence.is_some())
            .then(|| self.source_outcome(source.id())?.as_ref().err())
            .flatten();
        let offline = !self.nightly_catalogue_current();
        let from = open.from;
        let installed_row = installed_file.and_then(|file| {
            self.labeled_installed_rows()
                .0
                .into_iter()
                .find(|row| row.archive_file() == Some(file))
        });
        let current = self
            .source_activity()
            .filter(|a| a.ids.iter().any(|id| id == source.id()));
        let retry = |id, cx: &mut Context<Self>| {
            let source = source.clone();
            secondary(id, tr("Try again")).on_click(cx.listener(move |this, _, _, cx| {
                this.inspect_nightly(source.clone(), true);
                cx.notify();
            }))
        };
        let build_label = if let Some(activity) = current.as_ref() {
            if activity.progress == sunrise::InstallProgress::Publishing {
                tr("Finishing…").into()
            } else {
                stage_label(activity.progress)
            }
        } else if matches!(failed_job, Some(SourceError::Tools(_))) {
            tr("Retry download").into()
        } else {
            tr("Add from source…").into()
        };
        let build = primary("nightly-build-source", build_label)
            .w_full()
            .h(px(tokens::ACTION));
        let listed = state
            .catalogue
            .as_ref()
            .is_some_and(|list| list.sources.contains(source));
        let can_build =
            !offline && listed && details.and_then(sources::Details::checked_commit).is_some();
        let busy = self.nightly_busy();
        let build = if busy || current.is_some() {
            dim(build)
        } else if !offline && listed && failed_job.is_some() {
            let source = source.clone();
            build.on_click(cx.listener(move |this, _, _, cx| {
                this.reread_and_add(source.clone());
                cx.notify();
            }))
        } else if can_build {
            build.on_click(cx.listener(|this, _, _, cx| {
                this.build_nightly();
                cx.notify();
            }))
        } else {
            dim(build)
        };
        let installed_actions = installed.map(|origin| {
            let locked = busy || self.mutating() || self.mods_locked().is_some();
            use sunrise_launcher::github_mods::SourceTrack;
            let update = origin
                .source
                .as_ref()
                .filter(|source| !matches!(source.track, SourceTrack::Pinned))
                .map(|_| {
                    let candidate = self.source_update_version(origin);
                    let checking = self.source_updates_checking();
                    let label = if let Some(version) = candidate {
                        trf("Update to {version}", &[("version", version)])
                    } else if checking {
                        tr("Checking source updates…").into()
                    } else {
                        tr("Check for update").into()
                    };
                    let b = primary("nightly-update-source", label).w_full();
                    if locked || checking || (offline && candidate.is_some()) {
                        dim(b)
                    } else if candidate.is_some() {
                        let id = origin.id.clone();
                        b.on_click(cx.listener(move |this, _, _, cx| {
                            this.submit_source_update(&id);
                            cx.notify();
                        }))
                    } else {
                        let origin = origin.clone();
                        b.on_click(cx.listener(move |this, _, _, cx| {
                            this.check_source_updates(Some(&origin));
                            cx.notify();
                        }))
                    }
                });
            let recheck = origin
                .source
                .as_ref()
                .filter(|source| !matches!(source.track, SourceTrack::Pinned))
                .filter(|_| {
                    offline
                        && self.source_update_version(origin).is_some()
                        && !locked
                        && !self.source_updates_checking()
                })
                .map(|_| {
                    let origin = origin.clone();
                    secondary("nightly-recheck-source", tr("Check for update"))
                        .w_full()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.check_source_updates(Some(&origin));
                            cx.notify();
                        }))
                });
            let controls = installed_row.as_ref().map(|row| {
                let cells = model::cells(
                    row,
                    self.mods_state.errors.get(&row.id).map(String::as_str),
                    self.build_here()
                        .filter(|_| row.on)
                        .and_then(|run| run.state_of(&row.id, &row.version)),
                );
                self.installed_project_controls(row, &cells, locked, cx)
            });
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(install_fact(short_sha(&origin.commit), pending))
                .children(update)
                .children(recheck)
                .children(controls)
                .children(
                    self.source_update_notice(Some(&origin.id))
                        .map(|notice| note(notice.to_owned())),
                )
        });
        let failure = if installed.is_some() {
            self.source_update_issue(source.id())
        } else if missing {
            failure.as_ref()
        } else {
            None
        };
        // An Add or Update this mod was refused: said once by the modal when it happened, and
        // reopened from one line under the button.
        let refused_line = failed_job
            .map(|error| (error, installed.is_some()))
            .or_else(|| installed.and(failure).map(|error| (error, true)))
            .and_then(|(error, update)| {
                let refused = crate::mods::refused::Refused::one(
                    source.name(),
                    Some(source.repo().to_owned()),
                    update,
                    error,
                )?;
                Some(self.refused_line("nightly-refused", refused, cx))
            });
        // Reading the source itself failed: what to do about it, in place.
        let failure_panel = failure.filter(|_| installed.is_none()).and_then(|error| {
            let words = source_error(error, Some(source))?;
            Some(div().flex().flex_col().items_start().gap(px(7.))
                .child(note(words))
                .children(matches!(error, SourceError::Github(_) | SourceError::DependencyGithub { .. } | SourceError::Changed(_))
                    .then(|| if busy || matches!(error, SourceError::Github(sources::Failure::RateLimited { retry_at: Some(at) }) | SourceError::DependencyGithub { failure: sources::Failure::RateLimited { retry_at: Some(at) }, .. } if *at > SystemTime::now()) {
                        dim(secondary("nightly-detail-retry", tr("Try again")))
                    } else { retry("nightly-detail-retry", cx) })))
        });
        let tracking = match source.tracker() {
            sources::Tracker::Branch(branch) => branch.as_str(),
            sources::Tracker::Release => commit
                .and_then(|pin| pin.release_tag())
                .unwrap_or(tr("Release")),
        };
        let pin = if let Some(origin) = installed {
            origin.tag.as_deref().map_or_else(
                || short_sha(&origin.commit).to_owned(),
                |tag| {
                    trf(
                        "{track} @ {commit}",
                        &[("track", tag), ("commit", short_sha(&origin.commit))],
                    )
                },
            )
        } else {
            commit.map_or_else(
                || tr("Not checked").to_owned(),
                |pin| {
                    trf(
                        "{track} @ {commit}",
                        &[("track", tracking), ("commit", short_sha(pin.sha()))],
                    )
                },
            )
        };
        let repo_url = format!("https://github.com/{}", source.repo());
        let allow_icon_fetch = !offline
            && details
                .and_then(|details| details.error.as_ref())
                .is_none_or(|error| *error == sources::Failure::NoManifest);
        let badge = project_made_for_build(
            self.nightly_build_revision().as_deref(),
            commit.map(|pin| pin.sha()),
            installed.map(|origin| origin.commit.as_str()),
            details.and_then(|detail| detail.build_pin.as_deref()),
        )
        .then(|| {
            div()
                .flex()
                .child(tag(tr("Made for this build"), chip::Kind::Neutral))
        });
        let package = installed_row
            .as_ref()
            .and_then(|row| self.archive_display(row))
            .and_then(|archive| archive.icon.clone());
        let summary = source.summary().to_owned();
        let title_block = head(
            self.source_tile(source, commit, allow_icon_fetch, package, 64., cx)
                .into_any_element(),
            source.name(),
            Some(trf("by {authors}", &[("authors", source.owner())])),
            badge
                .map(IntoElement::into_any_element)
                .into_iter()
                .collect(),
            Some(summary).filter(|s| !s.trim().is_empty()),
            Vec::new(),
        );
        let github = secondary("nightly-github", tr("Open on GitHub"))
            .gap(px(5.))
            .child("↗")
            .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&repo_url)));
        let actions = action_column()
            .children(missing.then_some(build))
            .children(refused_line)
            .children(conflict.then(|| note(tr(SOURCE_ID_CONFLICT))))
            .children(installed_actions)
            .child(github);
        let side = card()
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(about_row(tr("Source code"), source.repo().to_owned()))
            .child(about_row(tr("Commit"), pin))
            .child(about_row(tr("Game build"), self.discover_target_name()))
            .children(
                (offline && missing)
                    .then(|| note(tr("Connect to check the source and build this mod."))),
            )
            .children((!offline && !listed).then(|| {
                note(tr(
                    "This source listing changed. Open it again from Discover.",
                ))
            }))
            .children(
                (!busy && missing && offline && !self.mods_state.nightly.reading)
                    .then(|| retry("nightly-refresh-catalogue", cx)),
            )
            .children(failure_panel);
        let header = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(
                back(if from == Page::Installed {
                    tr("Installed")
                } else {
                    tr("Discover")
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.choose_view(from);
                    cx.notify();
                })),
            )
            .children(self.strip(cx));
        self.project_layout(
            "project",
            crate::mods::project::Layout {
                header,
                head: title_block,
                action: Some(actions),
                gallery: None,
                side: Some(side),
            },
            std::iter::empty(),
        )
    }

    pub(in crate::mods) fn nightly_log_button(
        &self,
        name: &str,
        log: std::sync::Arc<str>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let name = name.to_owned();
        secondary("nightly-build-log", tr("View build log")).on_click(cx.listener(
            move |this, _, _, cx| {
                this.mods_state.nightly.work.log = Some((name.clone(), log.clone()));
                this.open_sheet(crate::sheets::Sheet::NightlySourceLog);
                cx.notify();
            },
        ))
    }

    pub(crate) fn source_log_body(&self, cx: &mut Context<Self>) -> Option<Div> {
        let (name, log) = self.mods_state.nightly.work.log.as_ref()?;
        let title = trf("{mod} build log", &[("mod", name)]);
        let log = log.to_string();
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(
                    div()
                        .text_size(px(tokens::HEADING))
                        .font_weight(FontWeight::BOLD)
                        .child(title),
                )
                .child(
                    div()
                        .id("nightly-log-lines")
                        .max_h(px(310.))
                        .overflow_y_scroll()
                        .child(crate::code_box::code_box(
                            "nightly-log",
                            vec![log.clone().into()],
                            None,
                        )),
                )
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.))
                        .child(crate::widgets::motion_bits::copy_button(
                            "nightly-log-copy",
                            tr("Copy log"),
                            log,
                        ))
                        .child(
                            secondary("nightly-log-close", tr("Close")).on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.close_sheet();
                                    cx.notify();
                                },
                            )),
                        ),
                ),
        )
    }

    pub(crate) fn source_already_allowed(&self, question: &SourceQuestion) -> bool {
        question.sources().all(|(_, origin)| {
            source_grant(origin)
                .is_some_and(|grant| self.settings.source_mod_grants.contains(&grant))
        })
    }

    pub(crate) fn source_consent_body(&self, cx: &mut Context<Self>) -> Option<Div> {
        let question = self.source_question()?;
        let base = crate::sheets::build_name(&question.base);
        let dependency_label = |name: &str, version: &str| {
            trf("{mod} {version}", &[("mod", name), ("version", version)])
        };
        let (heading, accept) = match (question.mods.as_slice(), question.dependencies.as_slice()) {
            (_, []) => (tr("Allow unreviewed source code?").into(), tr("Allow")),
            ([(name, _)], [dependency]) => (
                trf(
                    "{mod} needs {dependency}. Add both?",
                    &[
                        ("mod", name),
                        (
                            "dependency",
                            &dependency_label(&dependency.name, &dependency.version),
                        ),
                    ],
                ),
                tr("Add both"),
            ),
            _ => (
                tr("Add these mods and their required dependencies?").into(),
                tr("Add mods"),
            ),
        };
        let mut list = div().flex().flex_col().gap(px(8.));
        for (n, (name, origin)) in question.sources().enumerate() {
            let label = if question.dependencies.is_empty() {
                name.to_owned()
            } else {
                dependency_label(name, &origin.version)
            };
            let url = origin.source_url();
            list = list.child(
                card()
                    .p(px(9.))
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(label))
                            .child(note(format!(
                                "{}/{} @ {}",
                                origin.owner, origin.repo, origin.commit
                            ))),
                    )
                    .child(
                        secondary(format!("nightly-consent-source-{}", n), tr("View source"))
                            .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url))),
                    ),
            );
        }
        for dependency in question
            .dependencies
            .iter()
            .filter(|dep| dep.origin.is_none())
        {
            list = list.child(
                card()
                    .p(px(9.))
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(dependency_label(&dependency.name, &dependency.version)),
                    )
                    .child(note(tr("Sunrise mods"))),
            );
        }
        let approve =
            primary("nightly-consent-build", accept).on_click(cx.listener(|this, _, _, cx| {
                let Some(question) = this.source_question() else {
                    return;
                };
                let grants: Option<Vec<_>> = question
                    .sources()
                    .map(|(_, origin)| source_grant(origin))
                    .collect();
                let answer = grants
                    .ok_or_else(|| tr("The source could not be verified.").to_owned())
                    .and_then(|grants| this.settings.allow_source_mods(grants))
                    .map(|()| true);
                this.answer_source_question(answer);
                cx.notify();
            }));
        Some(div().flex().flex_col().gap(px(12.))
            .child(div().text_size(px(tokens::HEADING)).font_weight(FontWeight::BOLD)
                .child(heading))
            .child(note(trf("Game build: {build}", &[("build", &base)])))
            .child(div().id("nightly-consent-mods").max_h(px(185.)).overflow_y_scroll().child(list))
            .child(note(tr("Sunrise hasn't reviewed this code. Building and playing run the author's code on your computer.")))
            .child(div().flex().justify_end().gap(px(8.))
                .child(secondary("nightly-consent-cancel", tr("Cancel")).on_click(cx.listener(|this, _, _, cx| {
                    this.close_sheet();
                    cx.notify();
                })))
                .child(approve)))
    }
}

fn project_made_for_build(
    profile: Option<&str>,
    inspected: Option<&str>,
    installed: Option<&str>,
    pin: Option<&str>,
) -> bool {
    inspected.is_some_and(|inspected| installed.is_none_or(|installed| installed == inspected))
        && made_for_build(profile, pin)
}

fn source_grant(origin: &sunrise_launcher::github_mods::Origin) -> Option<backend::SourceModGrant> {
    Some(backend::SourceModGrant {
        repo_id: origin.source.as_ref()?.repo_id,
        owner: origin.owner.to_ascii_lowercase(),
        id: origin.id.clone(),
    })
}

pub(in crate::mods) fn short_sha(commit: &str) -> &str {
    commit.get(..7).unwrap_or(commit)
}

pub(in crate::mods) fn source_error(
    error: &SourceError,
    source: Option<&Source>,
) -> Option<String> {
    Some(match error {
        SourceError::Cancelled => return None,
        SourceError::Github(error) => match error {
            sources::Failure::RepositoryUnavailable => {
                tr("Repository unavailable. It may be private, moved or deleted.").into()
            }
            sources::Failure::RefUnavailable => tr("Tracked branch/release unavailable").into(),
            sources::Failure::RateLimited {
                retry_at: Some(time),
            } => trf(
                "GitHub’s request limit was reached. Try again after {time}.",
                &[("time", &source_time(*time))],
            ),
            sources::Failure::RateLimited { retry_at: None } => {
                tr("GitHub’s request limit was reached. Try again later.").into()
            }
            sources::Failure::NoManifest => source.map_or_else(
                || tr("No Sunrise mod at this commit").into(),
                |source| {
                    trf(
                        "No Sunrise mod at this commit. Expected {manifest}.",
                        &[("manifest", source.manifest())],
                    )
                },
            ),
            sources::Failure::Offline(_) => {
                tr("You're offline. Connect to check the source.").into()
            }
            sources::Failure::Other(_) => tr("Couldn't check this source.").into(),
        },
        SourceError::DependencyGithub { name, failure } => {
            let why = source_error(&SourceError::Github(failure.clone()), None)?;
            if matches!(failure, sources::Failure::Offline(_)) {
                why
            } else {
                trf(
                    "{mod} couldn't be read from GitHub. {why}",
                    &[("mod", name), ("why", &why)],
                )
            }
        }
        SourceError::Dependency {
            required: Some(id), ..
        } => trf(
            "Needs {mod}. Install a fitting version first.",
            &[("mod", id)],
        ),
        SourceError::Dependency { required: None, .. } => {
            tr("Can't install these mods together").into()
        }
        SourceError::Catalogue(why) => why.clone(),
        SourceError::Unproved(_) => tr("Couldn't add this mod").into(),
        SourceError::Changed(_) => {
            tr("The profile or game build changed. Check the source again; nothing was changed.")
                .into()
        }
        SourceError::DoesNotFit(_) => tr("Doesn't fit this build.").into(),
        SourceError::Tools(_) => {
            tr("Couldn't download build tools. Check your connection and try again.").into()
        }
        SourceError::Build { .. } => tr("Couldn't build on this computer.").into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_badge_describes_the_installed_commit_not_a_newer_inspection() {
        let build = "1".repeat(40);
        let old = "2".repeat(40);
        let newest = "3".repeat(40);
        let stamp =
            |shown| project_made_for_build(Some(&build), Some(&newest), shown, Some(&build));
        assert!(stamp(None));
        assert!(stamp(Some(&newest)));
        assert!(!stamp(Some(&old)));
        assert!(!project_made_for_build(
            Some(&build),
            None,
            Some(&old),
            Some(&build)
        ));
    }

    #[test]
    fn source_status_keeps_local_paths_out_of_regular_copy() {
        let diagnostic = "fixture /local/private/.installed C:\\Private\\build.log";
        for error in [
            SourceError::Github(sources::Failure::Other(diagnostic.into())),
            SourceError::Unproved(diagnostic.into()),
            SourceError::Changed(diagnostic.into()),
            SourceError::Dependency {
                required: Some("api_mod".into()),
                detail: diagnostic.into(),
            },
            SourceError::DoesNotFit(diagnostic.into()),
        ] {
            assert_eq!(error.to_string(), diagnostic);
            assert!(!source_error(&error, None).unwrap().contains("Private"));
            assert!(!source_error(&error, None).unwrap().contains("/local/"));
        }
    }
}
