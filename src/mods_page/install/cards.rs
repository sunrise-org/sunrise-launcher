//! What a registry card shows: its control, label, note, outcome, stage and ETA.
use super::*;

impl Launcher {
    pub(crate) fn registry_card(&self, id: &str) -> Option<Card<'_>> {
        self.mods.installs.card(&self.mods_home(), id)
    }

    pub(crate) fn registry_card_control(
        &self,
        id: &str,
        button: impl FnOnce(String) -> AnyElement,
    ) -> Option<AnyElement> {
        match self.registry_card(id)? {
            Card::Progress(stage) => Some(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(button(stage_label(stage)))
                    .children(stage_fraction(stage).map(|fraction| {
                        crate::widgets::progress_bar(Some((fraction, fraction)), &self.theme)
                    }))
                    .into_any_element(),
            ),
            Card::Waiting => Some(button(tr("Waiting to install…").into())),
            Card::Done(_) => None,
        }
    }

    pub(crate) fn registry_install_label(&self) -> String {
        if let Some(instance) = self
            .selected_instance()
            .filter(|_| self.instances.list.len() > 1)
        {
            trf("Install to {profile}", &[("profile", &instance.name)])
        } else {
            tr("Install").into()
        }
    }

    pub(crate) fn registry_card_note(&self, id: &str) -> Option<String> {
        self.registry_outcome(id)?.note()
    }

    pub(super) fn registry_outcome(&self, id: &str) -> Option<Outcome> {
        match self.registry_card(id) {
            Some(Card::Progress(_) | Card::Waiting) => None,
            card => {
                if let Some(hold) = self.registry_install_hold(id) {
                    return Some(Outcome {
                        failure: Some((
                            installed::hold_label(&hold),
                            installed::hold_detail(&hold),
                        )),
                        ..Outcome::default()
                    });
                }
                match card {
                    Some(Card::Done(outcome)) => Some(outcome.clone()),
                    _ => None,
                }
            }
        }
    }

    pub(crate) fn registry_card_outcome(&self, id: &str, cx: &mut Context<Self>) -> Option<Div> {
        let done = self.registry_outcome(id)?;
        let mut lines = Vec::new();
        if !done.added.is_empty() {
            lines.push(trf(
                "Also installed: {mods}",
                &[("mods", &crate::mods::names(&done.added))],
            ));
        }
        for (name, from, to) in &done.updated {
            lines.push(trf(
                "Updated {mod} from {from} to {to}",
                &[("mod", name), ("from", from), ("to", to)],
            ));
        }
        // A failed install says why in a dialog as it ends (`install_failure`); only a hold,
        // which lasts, stays on the page.
        let failure = done
            .failure
            .clone()
            .filter(|_| self.registry_install_hold(id).is_some());
        if lines.is_empty() && failure.is_none() {
            return None;
        }
        let failure = failure.map(|(label, detail)| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(8.))
                .child(label)
                .child(
                    crate::frame::button("install-outcome-detail", tr("Show why")).on_click(
                        cx.listener(move |this, _, _, cx| {
                            this.open_notice(detail.clone(), None);
                            cx.notify();
                        }),
                    ),
                )
        });
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .text_size(px(tokens::LABEL))
                .children(lines.into_iter().map(|line| div().child(line)))
                .children(failure),
        )
    }

    pub(crate) fn registry_stage(&self) -> Option<InstallProgress> {
        self.mods.installs.stage()
    }

    pub(crate) fn registry_eta(&self) -> Option<Duration> {
        self.mods
            .installs
            .running
            .as_ref()
            .and_then(|run| run.eta.remaining(Instant::now()))
    }
}

pub(crate) fn stage_label(step: InstallProgress) -> String {
    match step {
        InstallProgress::PreparingBuild => tr("Getting the build ready…").into(),
        InstallProgress::Checking => tr("Checking the mod set…").into(),
        InstallProgress::Waiting => tr("Waiting for another build to finish…").into(),
        InstallProgress::Downloading(f) => trf(
            "Downloading {percent}%",
            &[("percent", &format!("{:.0}", f * 100.))],
        ),
        InstallProgress::DownloadingBuild(f) => trf(
            "Downloading the build {percent}%",
            &[("percent", &format!("{:.0}", f * 100.))],
        ),
        InstallProgress::Minecraft(step, files) => {
            use sunrise_launcher::backend::PrepareStep;
            let count = files.map(|(done, total)| (done.to_string(), total.to_string()));
            match (step, count) {
                (PrepareStep::Files, None) => tr("Finding Minecraft's files…").into(),
                (PrepareStep::Files, Some((done, total))) => trf(
                    "Downloading Minecraft's files ({done} of {total})",
                    &[("done", &done), ("total", &total)],
                ),
                (PrepareStep::Java, None) => tr("Finding Java…").into(),
                (PrepareStep::Java, Some((done, total))) => trf(
                    "Downloading Java ({done} of {total} files)",
                    &[("done", &done), ("total", &total)],
                ),
                (PrepareStep::Sounds, _) => tr("Adding the sounds…").into(),
                (PrepareStep::Prepare, _) => tr("Preparing Minecraft's files…").into(),
            }
        }
        InstallProgress::Preparing => tr("Getting ready…").into(),
        InstallProgress::Rust(fraction) => trf(
            "Downloading build tools {percent}%",
            &[("percent", &format!("{:.0}", fraction * 100.0))],
        ),
        InstallProgress::Tools { total, .. } => trf(
            "Getting build tools ({size})",
            &[("size", &sunrise_launcher::storage::format_bytes(total))],
        ),
        InstallProgress::Building(Some(fraction)) => trf(
            "Building {percent}%",
            &[(
                "percent",
                &format!("{:.0}", (fraction * 100.0).clamp(0.0, 100.0)),
            )],
        ),
        InstallProgress::Building(None) => tr("Building").into(),
        InstallProgress::Publishing => tr("Finishing…").into(),
        InstallProgress::Reviewing => tr("Review changes…").into(),
    }
}

pub(crate) fn stage_fraction(step: InstallProgress) -> Option<f32> {
    match step {
        InstallProgress::Downloading(f)
        | InstallProgress::DownloadingBuild(f)
        | InstallProgress::Rust(f) => Some(f),
        InstallProgress::Minecraft(_, Some((done, total))) if total > 0 => {
            Some(done as f32 / total as f32)
        }
        InstallProgress::Building(f) => f,
        InstallProgress::Tools { done, total } if total > 0 => Some(done as f32 / total as f32),
        _ => None,
    }
}

/// What a registry install that finished but left mods off or on their old version says:
/// a line about `card`'s own mod, or the batch, and each change's reason.
pub(super) fn notice_failure(notice: &BuildNotice, card: &str) -> (String, String) {
    let this = notice.changes.iter().find(|change| change.project == card);
    let label = match this {
        Some(change) if change.predecessor.is_some() => tr("Kept the working version").into(),
        Some(change) => installed::hold_label(&change.reason),
        None => tr("Some mods stayed off or kept their old version").into(),
    };
    let detail = notice
        .changes
        .iter()
        .map(|change| {
            trf(
                "{mod}: {disposition}. {reason}",
                &[
                    ("mod", &change.title),
                    ("disposition", &change.disposition()),
                    ("reason", &installed::hold_detail(&change.reason)),
                ],
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    (label, detail)
}

pub(super) fn failure_line(error: &InstallError) -> &'static str {
    match error {
        InstallError::Refused(_) => tr("Can't install these mods together"),
        InstallError::Host(_) => tr("Native mods can't build on this computer"),
        InstallError::Unproved(_) => tr("Couldn't check that the mods fit"),
        InstallError::NotForVersion(_) => tr("Not available for your version"),
        InstallError::Source(SourceError::DoesNotFit(_)) => tr("Not available for your version"),
        InstallError::Source(SourceError::Dependency { .. } | SourceError::Catalogue(_)) => {
            tr("Can't install these mods together")
        }
        InstallError::Source(
            SourceError::Unproved(_)
            | SourceError::Github(_)
            | SourceError::DependencyGithub { .. },
        ) => tr("Fit could not be verified"),
        InstallError::Source(SourceError::Tools(_)) => {
            tr("Couldn't download build tools. Check your connection and try again.")
        }
        InstallError::Source(SourceError::Changed(_)) => {
            tr("The profile or game build changed. Check the source again; nothing was changed.")
        }
        InstallError::Source(SourceError::Cancelled) => tr("Install cancelled."),
        InstallError::Source(SourceError::Build { .. }) => tr("Couldn't build on this computer."),
        InstallError::GithubBuild(_) => tr("Could not build the selected mods."),
        InstallError::Download(_) => tr("Download failed. Try again"),
        InstallError::Compile(_) => tr("Couldn't build on your computer"),
        InstallError::PublisherChanged(_) => {
            tr("This mod's publisher changed. Confirm before installing.")
        }
        InstallError::AmbiguousProvider(_) => {
            tr("Several mods provide a required dependency. Choose one.")
        }
        InstallError::ProviderConflict(_) => {
            tr("Another mod is already installed for that slot. Switch provider instead.")
        }
        InstallError::Cancelled => tr("Install cancelled."),
    }
}
