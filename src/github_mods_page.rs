//! "Add from source" dialog (repository folder/link or local `.smod` file) and its prominent
//! unreviewed-code warning. Reading and checking happen in the background; nothing is installed
//! until the player explicitly confirms the warning.
use super::*;

mod actions;
mod check;
mod form;
mod local_source;
mod rows;
mod unreviewed;
use crate::blocks;
use crate::frame::{button, input, primary, Page, Reach as _};
use crate::mods::ManualBadge;
use crate::mods_page::install::GithubTarget;
use crate::paint::palette;
use crate::sheets::{self, Sheet};
use gpui::rgb;
use std::{collections::BTreeSet, path::PathBuf, sync::Arc};
use sunrise_launcher::{
    browser::sunrise::{self, sources::Failure, PastedSource, PreparedSource, SourceError},
    github_mods::{self, FetchError, LocalPackage, Packed, ResolveError, Resolved},
    native_mods::Cancellation,
};
pub(crate) use unreviewed::*;

pub(crate) struct GithubInput {
    pub(crate) target: GithubTarget,
    pub(crate) link: String,
    pub(crate) source: Resolved,
    pub(crate) selection: GithubSelection,
}

pub(crate) enum GithubSelection {
    Add(Vec<Packed>),
    Source(Vec<PreparedSource>),
}

pub(crate) struct LocalInput {
    pub(crate) target: GithubTarget,
    pub(crate) label: String,
    pub(crate) names: Vec<String>,
    pub(crate) packages: Vec<LocalPackage>,
}

pub(crate) struct Feedback {
    pub(crate) profile: instances::Instance,
    pub(crate) link: String,
    pub(crate) result: Result<String, (String, String)>,
    pub(crate) retry: Option<Arc<GithubInput>>,
    pub(crate) local: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SourceChoice {
    #[default]
    Repository,
    LocalFile,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WarningAction {
    Cancel,
    Confirm,
}

impl WarningAction {
    /// Cancel is always the default on the unreviewed-code warning.
    const DEFAULT: Self = Self::Cancel;
}

struct WarningProvenance {
    icon: &'static str,
    headline: String,
    detail: String,
}

enum PendingConfirm {
    Github(GithubInput),
    Local {
        target: GithubTarget,
        packages: Vec<LocalPackage>,
        folder: Option<PathBuf>,
    },
}

impl PendingConfirm {
    fn badge(&self) -> ManualBadge {
        match self {
            Self::Github(_)
            | Self::Local {
                folder: Some(_), ..
            } => ManualBadge::FromSource,
            Self::Local { folder: None, .. } => ManualBadge::Local,
        }
    }

    fn count(&self) -> usize {
        match self {
            Self::Github(input) => match &input.selection {
                GithubSelection::Add(packages) => packages.len(),
                GithubSelection::Source(packages) => packages.len(),
            },
            Self::Local { packages, .. } => packages.len(),
        }
    }

    fn any_native(&self) -> bool {
        match self {
            Self::Github(_) => true,
            Self::Local { packages, .. } => packages.iter().any(|p| p.native),
        }
    }

    fn unsigned_local_file(&self) -> bool {
        matches!(
            self,
            Self::Local {
                folder: None,
                packages,
                ..
            } if !matches!(
                packages.first().map(|p| &p.attestation),
                Some(github_mods::LocalAttestation::Verified { .. })
            )
        )
    }

    fn provenance(&self) -> WarningProvenance {
        match self {
            Self::Local {
                folder: None,
                packages,
                ..
            } => {
                let pkg = &packages[0];
                let path = pkg.source_path.display().to_string();
                match &pkg.attestation {
                    github_mods::LocalAttestation::Verified {
                        repository,
                        workflow,
                    } => WarningProvenance {
                        icon: "github",
                        headline: tr("Signed local file, not reviewed").into(),
                        detail: trf(
                            "File: {file} · Built by {repo} ({workflow}) · Sunrise has not checked this code.",
                            &[("file", &path), ("repo", repository), ("workflow", workflow)],
                        ),
                    },
                    github_mods::LocalAttestation::Failed(reason) => WarningProvenance {
                        icon: "warning",
                        headline: tr("Attestation check failed, not reviewed").into(),
                        detail: trf(
                            "File: {file} · The bundled attestation could not be verified ({reason}).",
                            &[("file", &path), ("reason", reason)],
                        ),
                    },
                    github_mods::LocalAttestation::Unsigned => WarningProvenance {
                        icon: "warning",
                        headline: tr("Unsigned local file, not reviewed").into(),
                        detail: trf(
                            "File: {file} · Sunrise cannot verify who built this file or where it came from.",
                            &[("file", &path)],
                        ),
                    },
                }
            }
            Self::Local {
                folder: Some(folder),
                packages,
                ..
            } => {
                let mods = packages
                    .iter()
                    .map(|p| p.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                let folder_name = folder.display().to_string();
                WarningProvenance {
                    icon: "folder",
                    headline: tr("Local repository folder, not reviewed").into(),
                    detail: trf(
                        "Folder: {folder} · Mods: {mods} · Sunrise has not checked this code.",
                        &[("folder", &folder_name), ("mods", &mods)],
                    ),
                }
            }
            Self::Github(input) => {
                let repo = input.source.name();
                let detail = match &input.selection {
                    GithubSelection::Add(packages) => {
                        let mods = packages
                            .iter()
                            .map(|p| p.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ");
                        let assets: Vec<(&str, &str)> = packages
                            .iter()
                            .filter_map(|p| p.release_asset(&input.source))
                            .collect();
                        if !assets.is_empty() && assets.len() == packages.len() {
                            let tag = assets[0].0;
                            let files = assets
                                .iter()
                                .map(|(_, asset)| *asset)
                                .collect::<Vec<_>>()
                                .join(", ");
                            trf(
                                "Mods: {mods} · Release {tag} ({files}) · Sunrise has not checked this code.",
                                &[("mods", &mods), ("tag", tag), ("files", &files)],
                            )
                        } else if let Some(tag) = input.source.tag.as_deref() {
                            trf(
                                "Mods: {mods} · Release {tag} · Sunrise has not checked this code.",
                                &[("mods", &mods), ("tag", tag)],
                            )
                        } else {
                            let sha: String = input.source.commit.chars().take(7).collect();
                            trf(
                                "Mods: {mods} · Commit {sha} · Sunrise has not checked this code.",
                                &[("mods", &mods), ("sha", &sha)],
                            )
                        }
                    }
                    GithubSelection::Source(packages) => {
                        let mods = packages
                            .iter()
                            .map(PreparedSource::name)
                            .collect::<Vec<_>>()
                            .join(", ");
                        let sha: String = input.source.commit.chars().take(7).collect();
                        trf(
                            "Mods: {mods} · Commit {sha} · Sunrise has not checked this code.",
                            &[("mods", &mods), ("sha", &sha)],
                        )
                    }
                };
                WarningProvenance {
                    icon: "github",
                    headline: trf("From github.com/{repo}, not reviewed", &[("repo", &repo)]),
                    detail,
                }
            }
        }
    }
}

struct FormError {
    line: String,
    detail: String,
    retry: bool,
}

impl FormError {
    fn new(line: &str, detail: String) -> Self {
        Self {
            line: line.into(),
            detail,
            retry: false,
        }
    }

    fn transport(detail: String) -> Self {
        Self {
            line: tr("Could not reach GitHub. Try again.").into(),
            detail,
            retry: true,
        }
    }
}

enum LocalRead {
    File(Box<LocalPackage>),
    Folder(PathBuf, Vec<LocalPackage>),
}

enum PickInput {
    Assets(Resolved, Vec<Packed>),
    Source(Resolved, Vec<PastedSource>),
    LocalFolder(PathBuf, Vec<LocalPackage>),
}

impl PickInput {
    fn rows(&self) -> Vec<(&str, &str, &str)> {
        match self {
            Self::Assets(_, mods) => mods
                .iter()
                .map(|p| (p.id.as_str(), p.name.as_str(), p.version.as_str()))
                .collect(),
            Self::Source(_, mods) => mods
                .iter()
                .map(|p| {
                    (
                        p.origin().id.as_str(),
                        p.name(),
                        p.origin().version.as_str(),
                    )
                })
                .collect(),
            Self::LocalFolder(_, mods) => mods
                .iter()
                .map(|p| (p.id.as_str(), p.name.as_str(), p.version.as_str()))
                .collect(),
        }
    }

    fn subtitle(&self) -> String {
        match self {
            Self::Assets(resolved, _) | Self::Source(resolved, _) => resolved.label(),
            Self::LocalFolder(path, _) => path.display().to_string(),
        }
    }

    fn resolved(&self) -> Option<&Resolved> {
        match self {
            Self::Assets(resolved, _) | Self::Source(resolved, _) => Some(resolved),
            Self::LocalFolder(_, _) => None,
        }
    }
}

struct Pick {
    input: PickInput,
    selected: BTreeSet<String>,
    error: Option<FormError>,
    /// The error's own words show (Show details).
    details: bool,
}

impl Pick {
    fn new(input: PickInput) -> Self {
        let selected = input
            .rows()
            .into_iter()
            .map(|(id, _, _)| id.into())
            .collect();
        Self {
            input,
            selected,
            error: None,
            details: false,
        }
    }

    fn checked(self, target: GithubTarget, link: String) -> GithubInput {
        let Self {
            input, selected, ..
        } = self;
        let (source, selection) = match input {
            PickInput::Assets(source, mut mods) => {
                mods.retain(|package| selected.contains(&package.id));
                (source, GithubSelection::Add(mods))
            }
            PickInput::Source(..) => unreachable!("sources need a selected-set fit proof"),
            PickInput::LocalFolder(..) => unreachable!("local folders install locally"),
        };
        GithubInput {
            target,
            link,
            source,
            selection,
        }
    }

    fn toggle(&mut self, id: &str) {
        self.error = None;
        if !self.selected.remove(id) {
            self.selected.insert(id.to_owned());
        }
    }
}

enum Stage {
    Choose(SourceChoice),
    Repository,
    Resolving(Receiver<Result<PickInput, FormError>>, Cancellation),
    ReadingLocal {
        rx: Receiver<Result<LocalRead, FormError>>,
        from_choose: bool,
    },
    Checking(Box<SourceCheck>),
    Pick(Box<Pick>),
    Warn(Box<PendingConfirm>),
    Failed {
        error: FormError,
        expanded: bool,
        from_choose: bool,
    },
    InstallFailed(Box<CheckedFailure>, bool),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PickerKind {
    LocalFile,
    LocalFolder,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PendingPicker {
    token: u64,
    kind: PickerKind,
}

impl Default for Stage {
    fn default() -> Self {
        Self::Choose(SourceChoice::Repository)
    }
}

struct SourceCheck {
    pick: Box<Pick>,
    rx: Receiver<Result<Vec<PreparedSource>, FormError>>,
    cancellation: Cancellation,
}

struct CheckedFailure {
    error: FormError,
    retry: Arc<GithubInput>,
}

#[derive(Default)]
pub(super) struct Github {
    text: line_edit::LineEdit,
    stage: Stage,
    focus: Option<FocusHandle>,
    cancel_focus: Option<FocusHandle>,
    warn_needs_focus: bool,
    repo_needs_focus: bool,
    target: Option<GithubTarget>,
    pending_failure: Option<CheckedFailure>,
    next_picker: u64,
    pending_picker: Option<PendingPicker>,
}

impl Github {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn close(&mut self) {
        // Dropping a pending lookup's receiver releases its packages even if it finishes later.
        match &self.stage {
            Stage::Resolving(_, cancellation) => cancellation.stop(),
            Stage::Checking(check) => check.cancellation.stop(),
            _ => (),
        }
        self.stage = Stage::Choose(SourceChoice::Repository);
        self.target = None;
        self.warn_needs_focus = false;
        self.repo_needs_focus = false;
        self.pending_picker = None;
        self.text.set(String::new());
    }

    fn start_picker(&mut self, kind: PickerKind) -> u64 {
        self.next_picker = self.next_picker.wrapping_add(1);
        let token = self.next_picker;
        self.pending_picker = Some(PendingPicker { token, kind });
        token
    }

    fn clear_picker(&mut self) {
        self.pending_picker = None;
    }

    fn finish_picker(
        &mut self,
        token: u64,
        kind: PickerKind,
        outcome: Result<Option<PathBuf>, String>,
    ) -> Option<PathBuf> {
        let stage_matches = match kind {
            PickerKind::LocalFile => matches!(self.stage, Stage::Choose(_)),
            PickerKind::LocalFolder => matches!(self.stage, Stage::Repository),
        };
        if self.pending_picker != Some(PendingPicker { token, kind }) || !stage_matches {
            return None;
        }
        self.pending_picker = None;
        match outcome {
            Ok(path) => path,
            Err(why) => {
                self.stage = Stage::Failed {
                    error: FormError::new(tr("Couldn't open the file picker."), why),
                    expanded: false,
                    from_choose: kind == PickerKind::LocalFile,
                };
                None
            }
        }
    }

    pub(super) fn busy(&self) -> bool {
        matches!(
            self.stage,
            Stage::Resolving(..) | Stage::ReadingLocal { .. } | Stage::Checking(_)
        )
    }

    fn can_resolve(&self) -> bool {
        !self.text.text().trim().is_empty()
            && match &self.stage {
                Stage::Repository => true,
                Stage::Failed {
                    error,
                    from_choose: false,
                    ..
                } => error.retry,
                _ => false,
            }
    }
}

fn resolve_link(
    link: &github_mods::Link,
    cancellation: &Cancellation,
) -> Result<PickInput, FormError> {
    let found = github_mods::resolve(link, &github_mods::api).map_err(|error| match error {
        error @ ResolveError::NoRelease(_) => {
            FormError::new(tr("This release wasn’t found."), error.into())
        }
        ResolveError::Lookup(why) => {
            FormError::new(tr("This GitHub link could not be found."), why)
        }
        ResolveError::Transfer(why) => FormError::transport(why),
    })?;
    if found.assets.is_empty() {
        let sources = sunrise::read_pasted_sources(link, &found, cancellation)
            .map_err(|error| pasted_source_error(&found.repo, error))?;
        return Ok(PickInput::Source(found, sources));
    }
    let mods = github_mods::fetch_checked(&found).map_err(|error| match error {
        FetchError::NoMods { release, detail } => FormError::new(
            if release {
                tr("No Sunrise mods were found in this release.")
            } else {
                tr("No Sunrise mods were found at this link.")
            },
            detail,
        ),
        FetchError::Package(why) => {
            FormError::new(tr("The GitHub mod package was not valid."), why)
        }
        FetchError::Transfer(why) => FormError::transport(why),
    })?;
    Ok(PickInput::Assets(found, mods))
}

/// A source the form couldn't take, said as every refusal is (`mods::refused`): what it
/// means for the mod people know as `name`, with the checker's words behind Show details.
fn pasted_source_error(name: &str, error: SourceError) -> FormError {
    sunrise_launcher::elog!("GitHub source preflight: {error}");
    if let SourceError::Github(Failure::Offline(_) | Failure::RateLimited { .. })
    | SourceError::DependencyGithub {
        failure: Failure::Offline(_) | Failure::RateLimited { .. },
        ..
    } = error
    {
        return FormError::transport(String::new());
    }
    match crate::mods::refused::Refusal::of(name, &error) {
        Some(refusal) => FormError::new(&refusal.said, refusal.details),
        None => FormError::new(tr("Install cancelled."), String::new()),
    }
}

impl Launcher {
    pub(crate) fn add_from_source_button(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        div()
            .id("add-from-source")
            .tab_index(0)
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .gap(px(6.))
            .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
            .border_1()
            .border_color(rgb(palette().line))
            .px(px(12.))
            .h(px(tokens::CONTROL))
            .text_color(rgb(palette().text))
            .text_size(px(tokens::LABEL))
            .font_weight(FontWeight::SEMIBOLD)
            .cursor_pointer()
            .hover(|s| s.bg(rgb(palette().hover)))
            .focus_visible(|s| s.border_color(rgb(palette().focus)))
            .seen()
            .child(blocks::icon("plus", 14., rgb(palette().text).into()))
            .child(tr("Add from source"))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.open_add_from_source();
                cx.notify();
            }))
    }

    pub(crate) fn open_add_from_source(&mut self) {
        self.github.close();
        self.open_sheet(Sheet::GithubAdd);
        match self.github_target() {
            Ok(target) => self.github.target = Some(target),
            Err(error) => {
                self.github.stage = Stage::Failed {
                    error: FormError::new(tr("Can't start the install"), error),
                    expanded: false,
                    from_choose: true,
                };
                return;
            }
        }
        if self.github.pending_failure.as_ref().is_some_and(|f| {
            self.github
                .target
                .as_ref()
                .is_some_and(|current| f.retry.target.same_destination(current))
        }) {
            if let Some(failure) = self.github.pending_failure.take() {
                self.github.stage = Stage::InstallFailed(Box::new(failure), false);
                self.mods.message = None;
            }
        }
    }

    pub(crate) fn show_github_result(&mut self, feedback: Feedback) {
        let Feedback {
            profile,
            link,
            result,
            retry,
            local,
        } = feedback;
        let same_profile = self.selected_instance().is_some_and(|current| {
            current.home == profile.home
                && current.id == profile.id
                && current.incarnation == profile.incarnation
        });
        match result {
            Ok(message) if same_profile => self.mods.message = Some(Ok(message)),
            Ok(_) => (),
            Err((line, detail)) => {
                let failure = retry.map(|retry| CheckedFailure {
                    error: FormError::new(&line, detail.clone()),
                    retry,
                });
                if !same_profile {
                    if let Some(failure) = failure {
                        self.github.pending_failure = Some(failure);
                    }
                    return;
                }
                if self.ui.page == Page::Installed
                    && self.ui.sheet.is_none()
                    && !self.blocking_dialog()
                {
                    self.github.close();
                    self.open_sheet(Sheet::GithubAdd);
                    self.github.target = self.github_target().ok();
                    self.github.stage = match failure {
                        Some(failure)
                            if self.github.target.as_ref().is_some_and(|current| {
                                failure.retry.target.same_destination(current)
                            }) =>
                        {
                            Stage::InstallFailed(Box::new(failure), false)
                        }
                        _ => {
                            if !local {
                                self.github.text.set(link);
                            }
                            Stage::Failed {
                                error: FormError::new(&line, detail),
                                expanded: false,
                                from_choose: local,
                            }
                        }
                    };
                    self.mods.message = None;
                } else {
                    if let Some(failure) = failure {
                        self.github.pending_failure = Some(failure);
                    }
                    // The notice says what it means; the checker's words stay with the form,
                    // behind Show details, when the player opens it again.
                    self.mods.message = Some(Err(line));
                }
            }
        }
    }

    fn enter_warning(&mut self, pending: PendingConfirm) {
        self.github.stage = Stage::Warn(Box::new(pending));
        self.github.warn_needs_focus = true;
    }

    fn handle_warning_action(&mut self, action: WarningAction) {
        match action {
            WarningAction::Cancel => self.close_sheet(),
            WarningAction::Confirm => self.confirm_unreviewed(),
        }
    }

    fn confirm_unreviewed(&mut self) {
        let Stage::Warn(pending) = std::mem::take(&mut self.github.stage) else {
            return;
        };
        // A build takes the confirmed files without asking again. A source's dependencies
        // aren't among them: a build asks about those.
        let digests: Vec<String> = match &*pending {
            PendingConfirm::Github(input) => match &input.selection {
                GithubSelection::Add(packages) => {
                    packages.iter().map(|p| p.sha256.clone()).collect()
                }
                GithubSelection::Source(packages) => {
                    packages.iter().map(|p| p.sha256().to_owned()).collect()
                }
            },
            PendingConfirm::Local { packages, .. } => {
                packages.iter().map(|p| p.sha256.clone()).collect()
            }
        };
        if let Err(error) = backend::allow_native_files(digests) {
            self.github.stage = Stage::Failed {
                error: FormError::new(&error, String::new()),
                expanded: false,
                from_choose: false,
            };
            return;
        }
        match *pending {
            PendingConfirm::Github(input) => self.submit_checked_github(input),
            PendingConfirm::Local {
                target,
                packages,
                folder,
            } => {
                let label = folder
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .or_else(|| {
                        packages
                            .first()
                            .map(|p| p.source_path.display().to_string())
                    })
                    .unwrap_or_default();
                let names = packages.iter().map(|p| p.name.clone()).collect();
                let input = Arc::new(LocalInput {
                    target,
                    label,
                    names,
                    packages,
                });
                if let Err(error) = self.queue_local(input) {
                    self.github.stage = Stage::Failed {
                        error: FormError::new(&error, String::new()),
                        expanded: false,
                        from_choose: folder.is_none(),
                    };
                } else {
                    self.close_sheet();
                }
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
