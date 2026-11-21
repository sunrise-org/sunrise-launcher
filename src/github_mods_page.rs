//! Installed's link form. Fetching only reads packages; the normal install worker owns mutation.
use super::*;
use crate::frame::{button, input, primary, Page, Reach as _};
use crate::mods_page::install::GithubTarget;
use crate::paint::palette;
use crate::sheets::{self, Sheet};
use gpui::rgb;
use std::{collections::BTreeSet, sync::Arc};
use sunrise_launcher::{
    browser::sunrise::{self, sources::Failure, PastedSource, PreparedSource, SourceError},
    github_mods::{self, FetchError, Origin, Packed, ResolveError, Resolved},
    native_mods::Cancellation,
};

pub(crate) struct GithubInput {
    pub(crate) target: GithubTarget,
    pub(crate) link: String,
    pub(crate) source: Resolved,
    pub(crate) selection: GithubSelection,
}

pub(crate) enum GithubSelection {
    Add(Vec<Packed>),
    Source(Vec<PreparedSource>),
    Update(Box<Packed>),
}

pub(crate) struct Feedback {
    pub(crate) profile: instances::Instance,
    pub(crate) link: String,
    pub(crate) result: Result<String, (String, String)>,
    pub(crate) retry: Option<Arc<GithubInput>>,
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

enum PickInput {
    Assets(Resolved, Vec<Packed>),
    Source(Resolved, Vec<PastedSource>),
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
        }
    }

    fn resolved(&self) -> &Resolved {
        match self {
            Self::Assets(resolved, _) | Self::Source(resolved, _) => resolved,
        }
    }
}

struct Pick {
    input: PickInput,
    selected: BTreeSet<String>,
    error: Option<FormError>,
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
        }
    }

    fn checked(self, target: GithubTarget, link: String, update: bool) -> GithubInput {
        let Self {
            input, selected, ..
        } = self;
        let (source, selection) = match input {
            PickInput::Assets(source, mut mods) => {
                mods.retain(|package| selected.contains(&package.id));
                let selection = if update {
                    GithubSelection::Update(Box::new(
                        mods.pop().expect("the update names one package"),
                    ))
                } else {
                    GithubSelection::Add(mods)
                };
                (source, selection)
            }
            PickInput::Source(..) => unreachable!("sources need a selected-set fit proof"),
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

#[derive(Default)]
enum Stage {
    #[default]
    Link,
    Resolving(Receiver<Result<Resolution, FormError>>, Cancellation),
    Checking(Box<SourceCheck>),
    Pick(Box<Pick>),
    Failed(FormError, bool),
    InstallFailed(Box<CheckedFailure>, bool),
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

enum Resolution {
    Ready(Box<PickInput>),
    Current,
}

#[derive(Default)]
pub(super) struct Github {
    text: line_edit::LineEdit,
    stage: Stage,
    focus: Option<FocusHandle>,
    target: Option<GithubTarget>,
    update: Option<Origin>,
    pending_failure: Option<CheckedFailure>,
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
        self.stage = Stage::Link;
        self.target = None;
        self.update = None;
        self.text.set(String::new());
    }

    pub(super) fn busy(&self) -> bool {
        matches!(self.stage, Stage::Resolving(..) | Stage::Checking(_))
    }

    fn can_resolve(&self) -> bool {
        !self.text.text().trim().is_empty()
            && match &self.stage {
                Stage::Link => true,
                Stage::Failed(error, _) => error.retry,
                _ => false,
            }
    }
}

fn resolve_link(
    link: &github_mods::Link,
    update: Option<&Origin>,
    cancellation: &Cancellation,
) -> Result<Resolution, FormError> {
    let found = github_mods::resolve(link, &github_mods::api).map_err(|error| match error {
        error @ ResolveError::NoRelease(_) => {
            FormError::new(tr("This release wasn’t found."), error.into())
        }
        ResolveError::Lookup(why) => {
            FormError::new(tr("This GitHub link could not be found."), why)
        }
        ResolveError::Transfer(why) => FormError::transport(why),
    })?;
    if update.is_some_and(|old| !github_mods::is_update(old, &found)) {
        return Ok(Resolution::Current);
    }
    if update.is_none() && found.assets.is_empty() {
        let sources = sunrise::read_pasted_sources(link, &found, cancellation)
            .map_err(pasted_source_error)?;
        return Ok(Resolution::Ready(Box::new(PickInput::Source(
            found, sources,
        ))));
    }
    let mut mods = github_mods::fetch_checked(&found).map_err(|error| match error {
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
    if let Some(old) = update {
        mods.retain(|package| package.id == old.id);
        if mods.is_empty() || found.assets.is_empty() {
            return Err(FormError::new(
                tr("The new release does not contain this mod."),
                trf(
                    "No release package was found for {mod}. The installed mod is unchanged.",
                    &[("mod", &old.id)],
                ),
            ));
        }
    }
    Ok(Resolution::Ready(Box::new(PickInput::Assets(found, mods))))
}

fn pasted_source_error(error: SourceError) -> FormError {
    eprintln!("GitHub source preflight: {error}");
    let line = match error {
        SourceError::Github(Failure::Offline(_) | Failure::RateLimited { .. }) => {
            return FormError::transport(String::new());
        }
        SourceError::DoesNotFit(_) => tr("Not available for your version").into(),
        SourceError::Github(Failure::NoManifest) => tr("No Sunrise mod at this commit.").into(),
        SourceError::Dependency {
            required: Some(id), ..
        } => trf(
            "Needs {mod}. Install a fitting version first.",
            &[("mod", &id)],
        ),
        SourceError::Dependency { required: None, .. } => {
            tr("Can't install these mods together").into()
        }
        _ => tr("Couldn't check that the mods fit").into(),
    };
    FormError::new(&line, String::new())
}

impl Launcher {
    pub(super) fn github_add_button(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        button("github-add", tr("Add from GitHub")).on_click(cx.listener(
            |this, _: &ClickEvent, window, cx| {
                this.open_github_add(None);
                let focus = this
                    .github
                    .focus
                    .get_or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true));
                window.focus(focus, cx);
                cx.notify();
            },
        ))
    }

    pub(super) fn open_github_add(&mut self, update: Option<Origin>) {
        self.github.close();
        self.open_sheet(Sheet::GithubAdd);
        match self.github_target() {
            Ok(target) => self.github.target = Some(target),
            Err(error) => {
                self.github.stage =
                    Stage::Failed(FormError::new(tr("Can't start the install"), error), false);
                return;
            }
        }
        if update.is_none()
            && self.github.pending_failure.as_ref().is_some_and(|f| {
                self.github
                    .target
                    .as_ref()
                    .is_some_and(|current| f.retry.target.same_destination(current))
            })
        {
            if let Some(failure) = self.github.pending_failure.take() {
                self.github.stage = Stage::InstallFailed(Box::new(failure), false);
                self.mods.message = None;
            }
        } else if let Some(update) = update {
            self.github.text.set(update.source_url());
            self.github.update = Some(update);
            self.github_resolve();
        }
    }

    pub(crate) fn show_github_result(&mut self, feedback: Feedback) {
        let Feedback {
            profile,
            link,
            result,
            retry,
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
                            self.github.text.set(link);
                            Stage::Failed(FormError::new(&line, detail), false)
                        }
                    };
                    self.mods.message = None;
                } else {
                    if let Some(failure) = failure {
                        self.github.pending_failure = Some(failure);
                    }
                    let message = if detail.is_empty() {
                        line
                    } else {
                        format!("{line}\n{detail}")
                    };
                    self.mods.message = Some(Err(message));
                }
            }
        }
    }

    fn github_resolve(&mut self) {
        if !self.github.can_resolve() || self.github.target.is_none() {
            return;
        }
        let update = self.github.update.clone();
        let link = update
            .as_ref()
            .map(|old| Ok(old.follows()))
            .unwrap_or_else(|| github_mods::parse(self.github.text.text()));
        let link = match link {
            Ok(link) => link,
            Err(why) => {
                self.github.stage =
                    Stage::Failed(FormError::new(tr("Enter a GitHub link."), why), false);
                return;
            }
        };
        let (tx, rx) = crate::wake::channel();
        let cancellation = Cancellation::default();
        self.github.stage = Stage::Resolving(rx, cancellation.clone());
        std::thread::spawn(move || {
            let _ = tx.send(resolve_link(&link, update.as_ref(), &cancellation));
        });
    }

    pub(super) fn github_tick(&mut self) -> bool {
        if self.ui.sheet != Some(Sheet::GithubAdd) {
            if self.github.target.is_some() {
                self.github.close();
            }
            return false;
        }
        if let Stage::Checking(check) = &self.github.stage {
            let result = match check.rx.try_recv() {
                Ok(result) => result,
                Err(std::sync::mpsc::TryRecvError::Empty) => return false,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Err(FormError::new(
                    tr("Couldn't check that the mods fit"),
                    tr("GitHub lookup stopped.").into(),
                )),
            };
            if let Stage::Checking(mut check) = std::mem::take(&mut self.github.stage) {
                match result {
                    Ok(sources) => {
                        if let Some(target) = self.github.target.take() {
                            let input = GithubInput {
                                target,
                                link: self.github.text.text().to_owned(),
                                source: check.pick.input.resolved().clone(),
                                selection: GithubSelection::Source(sources),
                            };
                            self.submit_checked_github(input);
                        }
                    }
                    Err(error) => {
                        check.pick.error = Some(error);
                        self.github.stage = Stage::Pick(check.pick);
                    }
                }
            }
            return true;
        }
        let Stage::Resolving(rx, _) = &self.github.stage else {
            return false;
        };
        match rx.try_recv() {
            Ok(Ok(Resolution::Ready(input))) => {
                let pick = Pick::new(*input);
                if pick.selected.len() == 1 {
                    self.github_submit(pick);
                } else {
                    self.github.stage = Stage::Pick(Box::new(pick));
                }
            }
            Ok(Ok(Resolution::Current)) => {
                self.close_sheet();
                self.mods.message = Some(Ok(tr("This GitHub mod is up to date.").into()));
            }
            Ok(Err(error)) => self.github.stage = Stage::Failed(error, false),
            Err(std::sync::mpsc::TryRecvError::Empty) => return false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.github.stage = Stage::Failed(
                    FormError::transport(tr("GitHub lookup stopped.").into()),
                    false,
                )
            }
        }
        true
    }

    fn github_pick(&mut self) {
        if !matches!(&self.github.stage, Stage::Pick(pick) if !pick.selected.is_empty() && pick.error.is_none())
        {
            return;
        }
        if let Stage::Pick(pick) = std::mem::take(&mut self.github.stage) {
            self.github_submit(*pick);
        }
    }

    fn github_submit(&mut self, mut pick: Pick) {
        if let PickInput::Source(_, choices) = &pick.input {
            let Some(target) = self.github.target.as_ref() else {
                return;
            };
            let target = match target.source() {
                Ok(target) => target,
                Err(error) => {
                    pick.error = Some(FormError::new(tr("Not available for your version"), error));
                    self.github.stage = Stage::Pick(Box::new(pick));
                    return;
                }
            };
            let selected = choices
                .iter()
                .filter(|package| pick.selected.contains(&package.origin().id))
                .cloned()
                .collect();
            let cancellation = Cancellation::default();
            let (tx, rx) = crate::wake::channel();
            self.github.stage = Stage::Checking(Box::new(SourceCheck {
                pick: Box::new(pick),
                rx,
                cancellation: cancellation.clone(),
            }));
            std::thread::spawn(move || {
                let result = sunrise::prepare_pasted_sources(
                    target.instance(),
                    target.identity(),
                    target.base(),
                    selected,
                    &cancellation,
                )
                .map_err(pasted_source_error);
                let _ = tx.send(result);
            });
        } else if let Some(target) = self.github.target.take() {
            let input = pick.checked(
                target,
                self.github.text.text().to_owned(),
                self.github.update.is_some(),
            );
            self.submit_checked_github(input);
        }
    }

    fn submit_checked_github(&mut self, input: GithubInput) {
        self.close_sheet();
        if let Err(error) = self.queue_github(Arc::new(input)) {
            self.mods.message = Some(Err(error));
        }
    }

    fn github_retry(&mut self) {
        let Stage::InstallFailed(failure, _) = &self.github.stage else {
            return;
        };
        let retry = failure.retry.clone();
        self.close_sheet();
        if let Err(error) = self.queue_github(retry) {
            self.mods.message = Some(Err(error));
        }
    }

    pub(crate) fn github_add_sheet(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let focus = self
            .github
            .focus
            .get_or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true))
            .clone();
        let muted = |text: &str| {
            div()
                .text_size(px(tokens::LABEL))
                .text_color(rgb(palette().muted))
                .child(text.to_owned())
        };
        let body = div().flex().flex_col();
        let picker = match &self.github.stage {
            Stage::Pick(pick) => Some((pick.as_ref(), false)),
            Stage::Checking(check) => Some((check.pick.as_ref(), true)),
            _ => None,
        };
        let (body, action) = if let Some((pick, checking)) = picker {
            let rows = div()
                .id("github-add-list")
                .max_h(px(184.))
                .overflow_y_scroll()
                .rounded(crate::tokens::r(tokens::R_CARD))
                .border_1()
                .border_color(rgb(palette().divider))
                .children(pick.input.rows().into_iter().enumerate().map(
                    |(i, (id, name, version))| {
                        let id = id.to_owned();
                        div()
                            .id(("github-add-choice", i))
                            .reach()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .px(px(12.))
                            .h(px(48.))
                            .cursor_pointer()
                            .when(i > 0, |r| {
                                r.border_t_1().border_color(rgb(palette().divider))
                            })
                            .hover(|s| s.bg(rgb(palette().hover)))
                            .child(
                                crate::mods::checkbox(
                                    ("github-add-check", i),
                                    pick.selected.contains(&id),
                                )
                                .tab_stop(false),
                            )
                            .child(crate::mod_icons::tile(
                                None::<gpui::ImageSource>,
                                &id,
                                name,
                                26.,
                            ))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(name.to_owned()),
                            )
                            .child(muted(version).flex_none())
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                if let Stage::Pick(pick) = &mut this.github.stage {
                                    pick.toggle(&id);
                                }
                                cx.notify();
                            }))
                    },
                ));
            let button = primary(
                "github-add-install",
                if checking {
                    tr("Checking fit…")
                } else {
                    tr("Install")
                },
            );
            let button = if checking || pick.error.is_some() || pick.selected.is_empty() {
                widgets::disabled(button)
            } else {
                button.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.github_pick();
                    cx.notify();
                }))
            };
            (
                body.child(sheets::title(tr("Choose mods to install").into()))
                    .child(muted(&pick.input.resolved().label()).truncate())
                    .child(muted(tr(github_mods::WARNING)))
                    .child(rows)
                    .children(pick.error.as_ref().map(|error| {
                        div()
                            .text_size(px(tokens::LABEL))
                            .text_color(rgb(palette().danger))
                            .child(error.line.clone())
                    })),
                button,
            )
        } else {
            let resolving = self.github.busy();
            let checked_failure = match &self.github.stage {
                Stage::InstallFailed(failure, _) => Some(failure.as_ref()),
                _ => None,
            };
            let mut body = if let Some(failure) = checked_failure {
                let selected = match &failure.retry.selection {
                    GithubSelection::Add(packages) => {
                        packages.iter().map(|p| p.name.as_str()).collect::<Vec<_>>()
                    }
                    GithubSelection::Source(packages) => {
                        packages.iter().map(PreparedSource::name).collect()
                    }
                    GithubSelection::Update(_) => unreachable!("only Add is retryable"),
                };
                body.child(sheets::title(tr("Add from GitHub").into()))
                    .child(
                        muted(&format!(
                            "{} @ {:.7}",
                            failure.retry.source.name(),
                            failure.retry.source.commit,
                        ))
                        .truncate(),
                    )
                    .child(muted(&selected.join(", ")).truncate())
            } else {
                let focused = !resolving && focus.is_focused(window);
                let mut field_theme = crate::mods::field_theme(self.theme);
                field_theme.glass = true;
                let field = input(focused)
                    .id("github-add-field")
                    .h(px(tokens::ACTION))
                    .px(px(12.))
                    .text_size(px(tokens::BODY))
                    .child(line_edit::content_with(
                        &self.github.text,
                        &field_theme,
                        focused,
                        false,
                        false,
                        "https://github.com/owner/repo".into(),
                    ));
                let field = if resolving {
                    field.opacity(0.55)
                } else {
                    let field = field
                        .track_focus(&focus)
                        .seen()
                        .cursor_text()
                        .on_click(cx.listener(move |_, _: &ClickEvent, window, cx| {
                            window.focus(&focus, cx);
                            cx.notify();
                        }))
                        .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                            match line_edit::handle(&mut this.github.text, e, cx) {
                                line_edit::Key::Pass | line_edit::Key::Escape => return,
                                line_edit::Key::Enter => this.github_resolve(),
                                line_edit::Key::Edited
                                | line_edit::Key::Paste
                                | line_edit::Key::Cut(_) => {
                                    this.github.stage = Stage::Link;
                                    this.github.update = None;
                                }
                                _ => {}
                            }
                            cx.stop_propagation();
                            cx.notify();
                        }));
                    line_edit::mouse(field, cx, |this| &mut this.github.text)
                };
                body.child(sheets::title(tr("Add from GitHub").into()))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .child(muted(tr(
                                "Paste a link to a repository, release, tag, branch or commit.",
                            )))
                            .child(field)
                            .child(muted(tr(github_mods::WARNING))),
                    )
            };
            if resolving {
                body = body.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .child(muted(tr("Checking GitHub…")))
                        .child(widgets::progress_bar(None, &self.theme).flex_none()),
                );
            }
            let error = match &self.github.stage {
                Stage::Failed(error, expanded) => Some((error, expanded)),
                Stage::InstallFailed(failure, expanded) => Some((&failure.error, expanded)),
                _ => None,
            };
            if let Some((error, expanded)) = error {
                body = body.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(8.))
                        .child(
                            div()
                                .text_size(px(tokens::LABEL))
                                .text_color(rgb(palette().danger))
                                .child(error.line.clone()),
                        )
                        .children((!error.detail.is_empty()).then(|| {
                            button("github-add-details", tr("Details")).on_click(cx.listener(
                                |this, _: &ClickEvent, _, cx| {
                                    match &mut this.github.stage {
                                        Stage::Failed(_, expanded)
                                        | Stage::InstallFailed(_, expanded) => *expanded ^= true,
                                        _ => (),
                                    }
                                    cx.notify();
                                },
                            ))
                        })),
                );
                if *expanded {
                    body = body.child(
                        div()
                            .id("github-add-error-detail")
                            .max_h(px(120.))
                            .overflow_y_scroll()
                            .p(px(10.))
                            .rounded(crate::tokens::r(tokens::R_SMALL))
                            .bg(rgb(palette().background))
                            .text_size(px(tokens::LABEL))
                            .text_color(rgb(palette().muted))
                            .child(crate::selectable::selectable(error.detail.clone())),
                    );
                }
            }
            let next = primary(
                "github-add-continue",
                if checked_failure.is_some()
                    || matches!(&self.github.stage, Stage::Failed(error, _) if error.retry)
                {
                    tr("Try again")
                } else {
                    tr("Continue")
                },
            );
            let next = if checked_failure.is_some() {
                next.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.github_retry();
                    cx.notify();
                }))
            } else if self.github.target.is_some() && self.github.can_resolve() {
                next.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.github_resolve();
                    cx.notify();
                }))
            } else {
                widgets::disabled(next)
            };
            (body, next)
        };
        let body = body.child(
            div()
                .flex()
                .justify_end()
                .gap(px(8.))
                .child(
                    button("github-add-cancel", tr("Cancel")).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.close_sheet();
                            cx.notify();
                        },
                    )),
                )
                .child(action),
        );
        sheets::frame("github-add-sheet", 500., body, cx)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use sunrise_launcher_smod::{index, Package};

    pub(crate) fn checked_subset(target: GithubTarget) -> Arc<GithubInput> {
        let release = "https://github.com/fixture/add/releases/download/v1";
        let archive = |id: &str| {
            let manifest = format!("id='{id}'\nname='{id}'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\n[native]\nsunrise='{}'\nmixins='{}'\ngame='26.3'\n", "b".repeat(40), "c".repeat(40));
            let cargo = format!("[package]\nname='{id}'\nversion='1.0.0'\n");
            Package::write_files(
                &manifest,
                &[("Cargo.toml", cargo.as_bytes()), ("src/lib.rs", b"")],
            )
            .unwrap()
        };
        let packages = [archive("one"), archive("two")];
        let found = Resolved {
            owner: "fixture".into(),
            repo: "add".into(),
            tag: Some("v1".into()),
            commit: "a".repeat(40),
            assets: ["one", "two"]
                .into_iter()
                .zip(&packages)
                .map(|(id, bytes)| github_mods::Asset {
                    name: format!("{id}.smod"),
                    url: format!("{release}/{id}.smod"),
                    sha256: Some(index::sha256(bytes)),
                })
                .collect(),
        };
        let checked = github_mods::fetch_with(&found, |url| {
            let i = found
                .assets
                .iter()
                .position(|asset| asset.url == url)
                .unwrap();
            Ok(packages[i].clone())
        })
        .unwrap();
        let mut pick = Pick::new(PickInput::Assets(found, checked));
        assert_eq!(pick.selected.len(), 2);
        pick.toggle("one");
        Arc::new(pick.checked(target, "https://github.com/fixture/add".into(), false))
    }
}
