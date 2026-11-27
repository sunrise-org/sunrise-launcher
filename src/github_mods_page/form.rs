use super::*;
use super::{actions, rows};
use crate::widgets::button::{primary, secondary};
use crate::widgets::check::{checkbox, Tick};

fn muted(text: &str) -> Div {
    div()
        .text_size(px(tokens::LABEL))
        .text_color(rgb(palette().muted))
        .child(text.to_owned())
}

fn source_option_card(
    choice: SourceChoice,
    id: &'static str,
    kind: SourceChoice,
    icon: &'static str,
    heading: &'static str,
    desc: &'static str,
    cx: &mut Context<Launcher>,
) -> Stateful<Div> {
    let selected = choice == kind;
    let accent = Hsla::from(rgb(palette().accent_ink));
    let rest_border = Hsla::from(rgb(palette().line));
    let hover_border = Hsla::from(rgb(palette().muted));
    div()
        .id(id)
        .reach()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(10.))
        .p(px(16.))
        .rounded(crate::tokens::r(tokens::R_CARD))
        .border_1()
        .border_color(if selected {
            accent.opacity(0.85)
        } else {
            rest_border
        })
        .bg(if selected {
            accent.opacity(0.14)
        } else {
            Hsla::from(rgb(palette().card_color)).opacity(0.6)
        })
        .cursor_pointer()
        .hover(move |s| s.border_color(if selected { accent } else { hover_border }))
        .child(
            div()
                .size(px(36.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(crate::tokens::r(tokens::R_CONTROL))
                .border_1()
                .border_color(rgb(palette().line))
                .bg(rgb(palette().field))
                .child(blocks::icon(icon, 18., Hsla::from(rgb(palette().text)))),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(palette().text))
                        .child(tr(heading)),
                )
                .child(
                    div()
                        .text_size(px(tokens::SMALL))
                        .text_color(rgb(palette().muted))
                        .child(tr(desc)),
                ),
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.github.stage = Stage::Choose(kind);
            cx.notify();
        }))
}

impl Launcher {
    pub(crate) fn github_add_sheet(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let focus = self
            .github
            .focus
            .get_or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true))
            .clone();
        if self.github.repo_needs_focus {
            self.github.repo_needs_focus = false;
            window.focus(&focus, cx);
        }
        if matches!(self.github.stage, Stage::Warn(_)) {
            return self.unreviewed_warning_sheet(window, cx);
        }
        if let Stage::Choose(choice) = self.github.stage {
            let body = self.choose_body(choice, window, cx);
            return sheets::frame("github-add-sheet", 560., body, cx);
        }
        if matches!(self.github.stage, Stage::ReadingLocal { .. }) {
            let body = self.reading_body(cx);
            return sheets::frame("github-add-sheet", 500., body, cx);
        }
        if matches!(
            self.github.stage,
            Stage::Failed {
                from_choose: true,
                ..
            }
        ) {
            let body = self.failed_body(cx);
            return sheets::frame("github-add-sheet", 500., body, cx);
        }

        let picker = match &self.github.stage {
            Stage::Pick(pick) => Some((pick.as_ref(), false)),
            Stage::Checking(check) => Some((check.pick.as_ref(), true)),
            _ => None,
        };
        let (body, back_action, action) = if let Some((pick, checking)) = picker {
            self.pick_body(pick, checking, cx)
        } else {
            let (body, resolving, checked) = self.source_body(focus, window, cx);
            self.source_footer(body, resolving, checked, cx)
        };
        let body = self.footer(body, back_action, action, cx);
        sheets::frame("github-add-sheet", 520., body, cx)
    }

    fn target_profile_name(&self) -> String {
        self.github
            .target
            .as_ref()
            .map(|t| t.profile_name().to_owned())
            .or_else(|| self.selected_instance().map(|i| i.name.clone()))
            .unwrap_or_else(|| tr("this profile").into())
    }

    fn choose_body(
        &mut self,
        choice: SourceChoice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let narrow = rows::is_narrow(f32::from(window.viewport_size().width));
        let profile_name = self.target_profile_name();
        let choices = div()
            .flex()
            .when(narrow, |d| d.flex_col())
            .gap(px(12.))
            .child(source_option_card(
                choice,
                "add-source-opt-repo",
                SourceChoice::Repository,
                "folder",
                "Repository (local folder or link)",
                "Build a mod from a GitHub link or a folder on this computer. Marked From source.",
                cx,
            ))
            .child(source_option_card(
                choice,
                "add-source-opt-local",
                SourceChoice::LocalFile,
                "package",
                "Local .smod file",
                "Install a .smod package file already saved on this computer. Marked Local.",
                cx,
            ));
        let warn_color = Hsla::from(rgb(palette().warning));
        let notice = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(12.))
            .py(px(10.))
            .rounded(crate::tokens::r(tokens::R_CONTROL))
            .border_1()
            .border_color(warn_color.opacity(0.35))
            .bg(warn_color.opacity(0.08))
            .child(blocks::icon("warning", 15., warn_color))
            .child(
                div()
                    .text_size(px(tokens::SMALL))
                    .text_color(rgb(palette().muted))
                    .child(tr(
                        "Neither option is reviewed by Sunrise, and neither updates automatically.",
                    )),
            );
        let body =
            div()
                .flex()
                .flex_col()
                .child(sheets::title(tr("Add from source").into()))
                .child(muted(&trf(
                    "Choose where to install an unreviewed mod for {profile}.",
                    &[("profile", &profile_name)],
                )))
                .child(choices)
                .child(notice)
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.))
                        .child(
                            secondary("add-source-cancel", tr("Cancel")).on_click(cx.listener(
                                |this, _: &ClickEvent, _, cx| {
                                    actions::cancel(this);
                                    cx.notify();
                                },
                            )),
                        )
                        .child(primary("add-source-continue", tr("Continue")).on_click(
                            cx.listener(move |this, _: &ClickEvent, _, cx| {
                                match choice {
                                    SourceChoice::Repository => actions::start_repository(this),
                                    SourceChoice::LocalFile => actions::read_local(this, cx),
                                }
                                cx.notify();
                            }),
                        )),
                );
        body
    }

    fn reading_body(&mut self, cx: &mut Context<Self>) -> Div {
        let body = div()
            .flex()
            .flex_col()
            .child(sheets::title(tr("Add from source").into()))
            .child(muted(tr("Reading mod files…")))
            .child(widgets::feedback::progress_bar(None))
            .child(div().flex().justify_end().child(
                secondary("github-add-cancel", tr("Cancel")).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        actions::cancel(this);
                        cx.notify();
                    },
                )),
            ));
        body
    }

    fn failed_body(&mut self, cx: &mut Context<Self>) -> Div {
        if let Stage::Failed {
            error,
            expanded,
            from_choose: true,
        } = &self.github.stage
        {
            let can_back = self.github.target.is_some();
            let expanded = *expanded;
            let line = error.line.clone();
            let detail = error.detail.clone();
            let mut body = div()
                .flex()
                .flex_col()
                .child(sheets::title(tr("Add from source").into()))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(8.))
                        .child(
                            div()
                                .text_size(px(tokens::LABEL))
                                .text_color(rgb(palette().danger))
                                .child(line),
                        )
                        .children((!detail.is_empty()).then(|| {
                            secondary(
                                "github-add-details",
                                crate::mods::refused::details_label(expanded),
                            )
                            .on_click(cx.listener(
                                |this, _: &ClickEvent, _, cx| {
                                    if let Stage::Failed { expanded, .. } = &mut this.github.stage {
                                        *expanded ^= true;
                                    }
                                    cx.notify();
                                },
                            ))
                        })),
                );
            if expanded && !detail.is_empty() {
                body = body.child(crate::mods::refused::details_box(
                    "github-add-error-detail",
                    detail,
                ));
            }
            let body = body.child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .children(can_back.then(|| {
                        secondary("add-source-back", tr("Back")).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.github.stage = Stage::Choose(SourceChoice::LocalFile);
                                cx.notify();
                            },
                        ))
                    }))
                    .child(
                        primary("github-add-cancel", tr("Cancel")).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                actions::cancel(this);
                                cx.notify();
                            },
                        )),
                    ),
            );
            body
        } else {
            unreachable!("failed_body is only called from a failed choose step")
        }
    }

    fn pick_body(
        &self,
        pick: &Pick,
        checking: bool,
        cx: &mut Context<Self>,
    ) -> (Div, Option<Stateful<Div>>, Stateful<Div>) {
        let body = div().flex().flex_col();
        let rows =
            div()
                .id("github-add-list")
                .max_h(px(184.))
                .overflow_y_scroll()
                .rounded(crate::tokens::r(tokens::R_CARD))
                .border_1()
                .border_color(palette().rule())
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
                            .when(i > 0, |r| r.border_t_1().border_color(palette().rule()))
                            .hover(|s| s.bg(rgb(palette().hover)))
                            .child(
                                checkbox(
                                    ("github-add-check", i),
                                    if pick.selected.contains(&id) {
                                        Tick::On
                                    } else {
                                        Tick::Off
                                    },
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
        let next = primary(
            "github-add-install",
            if checking {
                tr("Checking fit…")
            } else {
                tr("Continue")
            },
        );
        let next = if checking || pick.error.is_some() || pick.selected.is_empty() {
            widgets::disabled(next)
        } else {
            next.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                actions::pick(this);
                cx.notify();
            }))
        };
        (
            body.child(sheets::title(tr("Choose mods to install").into()))
                .child(muted(&pick.input.subtitle()).truncate())
                .child(muted(tr(github_mods::WARNING)))
                .child(rows)
                .children(pick.error.as_ref().map(|error| {
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .child(
                            div()
                                .text_size(px(tokens::LABEL))
                                .text_color(rgb(palette().danger))
                                .child(error.line.clone()),
                        )
                        .children((!error.detail.is_empty()).then(|| {
                            div().flex().child(
                                secondary(
                                    "github-pick-details",
                                    crate::mods::refused::details_label(pick.details),
                                )
                                .on_click(cx.listener(
                                    |this, _: &ClickEvent, _, cx| {
                                        if let Stage::Pick(pick) = &mut this.github.stage {
                                            pick.details ^= true;
                                        }
                                        cx.notify();
                                    },
                                )),
                            )
                        }))
                        .children((pick.details && !error.detail.is_empty()).then(|| {
                            crate::mods::refused::details_box(
                                "github-pick-error-detail",
                                error.detail.clone(),
                            )
                        }))
                })),
            None,
            next,
        )
    }

    fn source_field(
        &self,
        focus: FocusHandle,
        window: &mut Window,
        resolving: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let focused = !resolving && focus.is_focused(window);
        let mut field_theme = crate::mods::field_theme(self.theme);
        field_theme.glass = true;
        let field = crate::widgets::field::field_box(focused, false)
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
                        line_edit::Key::Enter => actions::resolve(this),
                        line_edit::Key::Edited | line_edit::Key::Paste | line_edit::Key::Cut(_) => {
                            this.github.stage = Stage::Repository;
                        }
                        _ => {}
                    }
                    cx.stop_propagation();
                    cx.notify();
                }));
            line_edit::mouse(field, cx, |this| &mut this.github.text)
        };
        let choose_folder = {
            let btn = secondary("add-source-pick-folder", tr("Choose local folder…"));
            if resolving {
                widgets::disabled(btn)
            } else {
                btn.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    actions::pick_folder(this, cx);
                    cx.notify();
                }))
            }
        };
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(muted(tr(
                "Paste a GitHub link to a repository, release, tag, branch or commit, or choose a mod folder on this computer.",
            )))
            .child(field)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(choose_folder)
                    .child(muted(tr("Marked From source · no auto-updates"))),
            )
    }

    fn source_body(
        &self,
        focus: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Div, bool, bool) {
        let resolving = self.github.busy();
        let checked_failure = match &self.github.stage {
            Stage::InstallFailed(failure, _) => Some(failure.as_ref()),
            _ => None,
        };
        let body = div().flex().flex_col();
        let mut body = if let Some(failure) = checked_failure {
            let selected = match &failure.retry.selection {
                GithubSelection::Add(packages) => {
                    packages.iter().map(|p| p.name.as_str()).collect::<Vec<_>>()
                }
                GithubSelection::Source(packages) => {
                    packages.iter().map(PreparedSource::name).collect()
                }
            };
            body.child(sheets::title(tr("Add from source").into()))
                .child(muted(&failure.retry.source.label()).truncate())
                .child(muted(&selected.join(", ")).truncate())
        } else {
            body.child(sheets::title(tr("Add from source").into()))
                .child(self.source_field(focus, window, resolving, cx))
        };
        if resolving {
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(muted(tr("Checking GitHub…")))
                    .child(widgets::feedback::progress_bar(None)),
            );
        }
        let error = match &self.github.stage {
            Stage::Failed {
                error, expanded, ..
            } => Some((error, expanded)),
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
                        secondary(
                            "github-add-details",
                            crate::mods::refused::details_label(*expanded),
                        )
                        .on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                match &mut this.github.stage {
                                    Stage::Failed { expanded, .. }
                                    | Stage::InstallFailed(_, expanded) => *expanded ^= true,
                                    _ => (),
                                }
                                cx.notify();
                            },
                        ))
                    })),
            );
            if *expanded {
                body = body.child(crate::mods::refused::details_box(
                    "github-add-error-detail",
                    error.detail.clone(),
                ));
            }
        }
        (body, resolving, checked_failure.is_some())
    }

    fn source_footer(
        &self,
        body: Div,
        resolving: bool,
        checked: bool,
        cx: &mut Context<Self>,
    ) -> (Div, Option<Stateful<Div>>, Stateful<Div>) {
        let back = (!checked && !resolving).then(|| {
            secondary("add-source-back", tr("Back")).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    actions::back_to_repository(this);
                    cx.notify();
                },
            ))
        });
        let next = primary(
            "github-add-continue",
            if checked
                || matches!(
                    &self.github.stage,
                    Stage::Failed { error, .. } if error.retry
                )
            {
                tr("Try again")
            } else {
                tr("Continue")
            },
        );
        let next = if checked {
            next.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                actions::retry(this);
                cx.notify();
            }))
        } else if self.github.target.is_some() && self.github.can_resolve() {
            next.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                actions::resolve(this);
                cx.notify();
            }))
        } else {
            widgets::disabled(next)
        };
        (body, back, next)
    }

    fn footer(
        &self,
        body: Div,
        back_action: Option<Stateful<Div>>,
        action: Stateful<Div>,
        cx: &mut Context<Self>,
    ) -> Div {
        body.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .children(back_action)
                .child(div().flex_1())
                .child(
                    secondary("github-add-cancel", tr("Cancel")).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            actions::cancel(this);
                            cx.notify();
                        },
                    )),
                )
                .child(action),
        )
    }
}
