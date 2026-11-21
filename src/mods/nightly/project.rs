//! The pinned source detail. GitHub content is data; it never selects the profile or executes code.
use super::*;
use crate::mods_page::install::{stage_label, SourceQuestion};
use sunrise_launcher::browser::sunrise::SourceError;

impl Launcher {
    pub(in crate::mods) fn nightly_project(&mut self, cx: &mut Context<Self>) -> AnyElement {
        self.ui.mods.build_button_drawn = false;
        let state = &self.ui.mods.nightly;
        let open = state.open.as_ref().expect("a Nightly detail is open");
        let source = &open.source;
        let inspection = open.inspection.as_ref();
        let details = inspection.map(|i| &i.data);
        let readme = details.and_then(|d| d.readme.as_ref());
        let commit = details.and_then(|d| d.commit.as_ref());
        let failure = inspection.and_then(|i| i.candidate.as_ref().err());
        let failed_job = self
            .source_outcome(source.id())
            .and_then(|outcome| outcome.as_ref().err());
        let offline = !self.nightly_catalogue_current();
        let current = self
            .source_activity()
            .filter(|a| a.ids.iter().any(|id| id == source.id()));
        let retry = |id, cx: &mut Context<Self>| {
            let source = source.clone();
            button(id, tr("Try again")).on_click(cx.listener(move |this, _, _, cx| {
                this.inspect_nightly(source.clone(), true);
                cx.notify();
            }))
        };
        let worked = commit.is_some_and(|pin| {
            self.ui.mods.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot
                    .rows
                    .local
                    .iter()
                    .filter_map(|local| local.github.as_ref())
                    .any(|origin| {
                        origin.id == source.id()
                            && origin.commit == pin.sha()
                            && origin
                                .source
                                .as_ref()
                                .is_some_and(|receipt| receipt.repo_id == source.repo_id())
                            && origin.owner.eq_ignore_ascii_case(source.owner())
                    })
            })
        });
        let build_label = if let Some(activity) = current.as_ref() {
            if activity.progress == sunrise::InstallProgress::Publishing {
                tr("Finishing…").into()
            } else {
                stage_label(activity.progress)
            }
        } else if worked {
            tr("Installed").into()
        } else if matches!(failed_job, Some(SourceError::Tools(_))) {
            tr("Retry download").into()
        } else {
            tr("Build from source…").into()
        };
        let build = primary("nightly-build-source", build_label)
            .w_full()
            .h(px(tokens::ACTION));
        let listed = state
            .catalogue
            .as_ref()
            .is_some_and(|list| list.sources.contains(source));
        let can_build = !offline && listed && inspection.is_some_and(|i| i.candidate.is_ok());
        let busy = self.mutating()
            || self.mods_locked().is_some()
            || self
                .ui
                .mods
                .snapshot
                .as_ref()
                .is_none_or(|snap| !snap.ready());
        let build = if busy || worked {
            dim(build)
        } else if can_build {
            build.on_click(cx.listener(|this, _, _, cx| {
                this.build_nightly();
                cx.notify();
            }))
        } else if !offline && failed_job.is_some() {
            let source = source.clone();
            build.on_click(cx.listener(move |this, _, _, cx| {
                this.inspect_nightly(source.clone(), true);
                cx.notify();
            }))
        } else {
            dim(build)
        };
        let failure_panel = failed_job.or(failure).and_then(|error| {
            let dependency = match error {
                SourceError::Dependency { required: Some(id), .. } => state.catalogue.as_ref()
                    .and_then(|list| list.sources.iter().find(|source| source.id() == id)).cloned(),
                _ => None,
            };
            let log = match error {
                SourceError::Build { log: Some(log), .. } => Some(log.clone()),
                _ => None,
            };
            let words = source_error(error, Some(source))?;
            Some(div().flex().flex_col().items_start().gap(px(7.))
                .child(note(words))
                .children(dependency.map(|source| button("nightly-find-dependency", tr("Find dependency"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.inspect_nightly(source.clone(), false);
                        cx.notify();
                    }))))
                .children(log.map(|log| self.nightly_log_button(source.name(), log, cx)))
                .children(matches!(error, SourceError::Github(_) | SourceError::Changed(_))
                    .then(|| if matches!(error, SourceError::Github(sources::Failure::RateLimited { retry_at: Some(at) }) if *at > SystemTime::now()) {
                        dim(button("nightly-detail-retry", tr("Try again")))
                    } else { retry("nightly-detail-retry", cx) })))
        });
        let tracking = match source.tracker() {
            sources::Tracker::Branch(branch) => branch.as_str(),
            sources::Tracker::Release => commit
                .and_then(|pin| pin.release_tag())
                .unwrap_or(tr("Release")),
        };
        let pin = commit.map_or_else(
            || tr("Not checked").to_owned(),
            |pin| {
                trf(
                    "{track} @ {commit}",
                    &[("track", tracking), ("commit", short_sha(pin.sha()))],
                )
            },
        );
        let repo_url = format!("https://github.com/{}", source.repo());
        let title = div()
            .flex()
            .items_center()
            .min_w_0()
            .gap(px(12.))
            .child(tile(None::<PathBuf>, source.id(), source.name(), 56.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(tokens::DISPLAY))
                            .font_weight(FontWeight::BOLD)
                            .child(crate::selectable::selectable(source.name().to_owned())),
                    )
                    .child(muted(source.repo().to_owned())),
            );
        let panel = card()
            .p(px(14.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(source.name().to_owned()),
            )
            .child(build)
            .child(super::super::project::about_row(tr("Source"), pin))
            .child(super::super::project::about_row(
                tr("Game build"),
                self.nightly_target_name(),
            ))
            .children(offline.then(|| note(tr("Connect to check the source and build this mod."))))
            .children((!offline && !listed).then(|| {
                note(tr(
                    "This source listing changed. Open it again from Discover.",
                ))
            }))
            .children(
                (!busy && offline && !self.ui.mods.nightly.reading)
                    .then(|| retry("nightly-refresh-catalogue", cx)),
            )
            .children(failure_panel)
            .child(
                div().flex().justify_end().child(
                    button("nightly-github", tr("Open on GitHub"))
                        .gap(px(5.))
                        .child("↗")
                        .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&repo_url))),
                ),
            );
        let mut left = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(px(12.))
            .child(title);
        let (above, beside) = if self.content_width() < 610. {
            (Some(panel), None)
        } else {
            (
                None,
                Some(
                    panel
                        .w(px(if self.content_width() < 860. {
                            228.
                        } else {
                            315.
                        }))
                        .flex_none(),
                ),
            )
        };
        left = left.children(above);
        let saved_readme = readme.is_some()
            && matches!(
                details.and_then(|details| details.error.as_ref()),
                Some(sources::Failure::Offline(_) | sources::Failure::RateLimited { .. })
            );
        left = left.child(muted(if saved_readme {
            tr("README from the repository · saved copy")
        } else {
            tr("README from the repository")
        }));
        let body = if let Some(readme) = readme {
            let entity = cx.entity().downgrade();
            let compact = self.content_width() < 860.;
            crate::markdown::render_pinned(readme, &self.theme, |image| {
                let placeholder = || note(image.alt().to_owned());
                match state.work.images.get(image) {
                    Some(Some(Some(path))) => gpui::img(path.clone())
                        .max_w_full()
                        .max_h(px(if compact { 180. } else { 240. }))
                        .object_fit(gpui::ObjectFit::ScaleDown)
                        .into_any_element(),
                    Some(_) => placeholder().into_any_element(),
                    None => {
                        let image = image.clone();
                        let entity = entity.clone();
                        div()
                            .relative()
                            .min_h(px(24.))
                            .child(placeholder())
                            .child(
                                gpui::canvas(
                                    move |bounds, window, cx| {
                                        if window.content_mask().bounds.intersects(&bounds) {
                                            let image = image.clone();
                                            let entity = entity.clone();
                                            cx.defer(move |cx| {
                                                entity
                                                    .update(cx, |this, _| {
                                                        this.request_nightly_image(image)
                                                    })
                                                    .ok();
                                            });
                                        }
                                    },
                                    |_, _, _, _| {},
                                )
                                .absolute()
                                .inset_0(),
                            )
                            .into_any_element()
                    }
                }
            })
        } else if inspection.is_none() {
            note(tr("Checking the pinned source…")).into_any_element()
        } else {
            card()
                .p(px(14.))
                .flex()
                .flex_col()
                .gap(px(6.))
                .items_start()
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(tr("README unavailable")),
                )
                .child(note(tr("You can still inspect the source on GitHub.")))
                .children(
                    (details.is_some_and(|d| d.error.is_none()) && !busy)
                        .then(|| retry("nightly-readme-retry", cx)),
                )
                .into_any_element()
        };
        left = left.child(body);
        div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .p(px(24.))
            .gap(px(12.))
            .children(self.strip(cx))
            .child(
                div()
                    .id("nightly-project-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&crate::frame::scrolls("nightly-project"))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(14.))
                            .child(super::super::project::back(tr("Nightly mods")).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.ui.mods.nightly.open = None;
                                    this.choose_view(Page::Discover);
                                    cx.notify();
                                }),
                            ))
                            .child(
                                div()
                                    .flex()
                                    .items_start()
                                    .gap(px(18.))
                                    .child(left)
                                    .children(beside),
                            ),
                    ),
            )
            .into_any_element()
    }

    pub(in crate::mods) fn nightly_log_button(
        &self,
        name: &str,
        log: std::sync::Arc<str>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let name = name.to_owned();
        button("nightly-build-log", tr("View build log")).on_click(cx.listener(
            move |this, _, _, cx| {
                this.ui.mods.nightly.work.log = Some((name.clone(), log.clone()));
                this.open_sheet(crate::sheets::Sheet::NightlySourceLog);
                cx.notify();
            },
        ))
    }

    pub(crate) fn source_log_body(&self, cx: &mut Context<Self>) -> Option<Div> {
        let (name, log) = self.ui.mods.nightly.work.log.as_ref()?;
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
                        .child(
                            button("nightly-log-copy", tr("Copy log")).on_click(cx.listener(
                                move |_, _, _, cx| {
                                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                        log.clone(),
                                    ));
                                },
                            )),
                        )
                        .child(
                            button("nightly-log-close", tr("Close")).on_click(cx.listener(
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
        self.settings.native_mods_allowed_at.is_some()
            && question.mods.iter().all(|(_, origin)| {
                source_grant(origin)
                    .is_some_and(|grant| self.settings.source_mod_grants.contains(&grant))
            })
    }

    pub(crate) fn source_consent_body(&self, cx: &mut Context<Self>) -> Option<Div> {
        let question = self.source_question()?;
        let base = crate::sheets::build_name(&question.base);
        let mut list = div().flex().flex_col().gap(px(8.));
        for (n, (name, origin)) in question.mods.iter().enumerate() {
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
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(name.clone()))
                            .child(note(format!(
                                "{}/{} @ {}",
                                origin.owner, origin.repo, origin.commit
                            ))),
                    )
                    .child(
                        button(("nightly-consent-source", n), tr("View source"))
                            .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url))),
                    ),
            );
        }
        let approve = primary("nightly-consent-build", tr("Build")).on_click(cx.listener(
            |this, _, _, cx| {
                let Some(question) = this.source_question() else {
                    return;
                };
                let grants: Option<Vec<_>> = question
                    .mods
                    .iter()
                    .map(|(_, origin)| source_grant(origin))
                    .collect();
                let answer = grants
                    .ok_or_else(|| tr("The source could not be verified.").to_owned())
                    .and_then(|grants| this.settings.allow_source_mods(grants))
                    .map(|()| true);
                this.answer_source_question(answer);
                cx.notify();
            },
        ));
        Some(div().flex().flex_col().gap(px(12.))
            .child(div().text_size(px(tokens::HEADING)).font_weight(FontWeight::BOLD)
                .child(tr("Build unreviewed source code?")))
            .child(note(trf("Game build: {build}", &[("build", &base)])))
            .child(div().id("nightly-consent-mods").max_h(px(185.)).overflow_y_scroll().child(list))
            .child(note(tr("Sunrise has not reviewed this code. Building runs the author’s code and build scripts on this computer. Native mods can also run code when you play, with the same access as other programs you run. Only continue if you trust this mod.")))
            .child(div().flex().justify_end().gap(px(8.))
                .child(button("nightly-consent-cancel", tr("Cancel")).on_click(cx.listener(|this, _, _, cx| {
                    this.close_sheet();
                    cx.notify();
                })))
                .child(approve)))
    }
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
        SourceError::Dependency {
            required: Some(id), ..
        } => trf(
            "Needs {mod}. Install a fitting version first.",
            &[("mod", id)],
        ),
        SourceError::Dependency { required: None, .. } => {
            tr("Can't install these mods together").into()
        }
        SourceError::Unproved(_) => tr("Fit could not be verified").into(),
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
