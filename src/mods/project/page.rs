use super::look::{action_column, card, head, muted, note, section};
use super::*;
use crate::widgets::button::{primary, secondary};
use crate::widgets::chip::{self, tag};

impl Launcher {
    pub(super) fn v2_project_page(
        &mut self,
        header: Div,
        page: Div,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use super::model::SourceLink;
        use sunrise_launcher::browser::details::category_label;

        let p = palette();
        let open_id = self.mods_state.open.as_ref().map(|o| o.id.clone());
        // Opened from Installed or Discover alike, an installed mod keeps its own controls here.
        let (rows, _) = self.labeled_installed_rows();
        let installed_row = rows.iter().find(|row| {
            row.v2_installed()
                && row
                    .entry
                    .as_ref()
                    .is_some_and(|e| Some(&e.project) == open_id.as_ref())
        });
        let busy = self.mods_busy()
            || self.mods_locked().is_some()
            || self
                .mods_state
                .snapshot
                .as_ref()
                .is_none_or(|snapshot| !snapshot.ready());
        let cells = installed_row.map(|row| {
            model::cells(
                row,
                self.mods_state.errors.get(&row.id).map(String::as_str),
                self.build_here()
                    .filter(|_| row.on)
                    .and_then(|run| run.state_of(&row.id, &row.version)),
            )
        });
        let controls = installed_row
            .zip(cells.as_ref())
            .map(|(row, cells)| self.installed_project_controls(row, cells, busy, cx));
        let vp = match self
            .mods_state
            .open
            .as_ref()
            .and_then(|o| o.v2_page.as_ref())
        {
            None => {
                return page
                    .child(header)
                    .child(muted(tr("Loading…")))
                    .into_any_element();
            }
            Some(Err(error)) => {
                return page
                    .child(header)
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(tr("This mod can't be shown")),
                    )
                    .child(note(error.clone()))
                    // An installed mod the registry no longer lists can still be removed here.
                    .children(controls.map(|controls| controls.max_w(px(320.))))
                    .into_any_element();
            }
            Some(Ok(vp)) => (**vp).clone(),
        };

        let profile_name = self
            .selected_instance()
            .map(|i| i.name.clone())
            .unwrap_or_else(|| tr("Default").to_owned());
        let target_label = data::target_label(
            &profile_name,
            self.selected_exact_build()
                .as_deref()
                .map(crate::sheets::build_name)
                .as_deref(),
        );

        let installed = self.discover_installed();
        let entry = installed.and_then(|rows| rows.registry_entry(&vp.id));
        let installed_version = entry.map(|entry| entry.number.clone());
        let pending = installed.is_some_and(|rows| rows.registry_pending(&vp.id));
        let package = installed
            .zip(entry)
            .and_then(|(rows, entry)| rows.archives.get(&entry.file)?.icon.clone());

        let picture = mod_artwork(
            vp.icon
                .as_deref()
                .and_then(|rel| self.v2_mod_image(&vp.id, rel, images::ICON)),
            package,
        );

        let credit = if vp.authors.is_empty() {
            String::new()
        } else {
            trf("by {authors}", &[("authors", &vp.authors)])
        };
        let categories = vp.categories.iter().map(|k| {
            div()
                .px(px(8.))
                .py(px(2.))
                .rounded(crate::tokens::r(crate::tokens::R_SMALL))
                .border_1()
                .border_color(p.rule())
                .bg(c(p.field))
                .text_size(px(tokens::CAPTION))
                .text_color(c(p.muted))
                .child(category_label(k))
        });

        let mut chips: Vec<AnyElement> =
            vec![tag(vp.make.label(), chip::Kind::Neutral).into_any_element()];
        chips.extend(categories.map(IntoElement::into_any_element));
        let facts: Vec<String> = (!vp.updated.is_empty())
            .then(|| format!("{} {}", tr("Updated"), vp.updated))
            .into_iter()
            .collect();
        let summary = Some(vp.summary.clone()).filter(|s| !s.trim().is_empty());
        let by = (!credit.is_empty()).then(|| credit.clone());
        let header_block = head(
            tile(picture, &vp.id, &vp.name, 64.).into_any_element(),
            &vp.name,
            by,
            chips,
            summary,
            facts,
        );

        // Captioned screenshot gallery: fetched ONLY here when the mod page opens.
        let gallery = self.v2_gallery(&vp.id, &vp.screenshots, cx);

        // Body: Markdown description + fitting versions section.
        let mut body: Vec<AnyElement> = Vec::new();
        if !vp.description.trim().is_empty() {
            body.push(prose(self.theme, &vp.description));
        }
        if !vp.dependencies.is_empty() {
            let mut list = card().flex().flex_col();
            for (n, dep) in vp.dependencies.iter().enumerate() {
                list = list.child(self.v2_dependency_line(n, dep, cx));
            }
            body.push(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .pt(px(8.))
                    .child(section(tr("Dependencies")))
                    .child(list)
                    .into_any_element(),
            );
        }

        let versions_heading = div()
            .flex()
            .flex_wrap()
            .items_baseline()
            .justify_between()
            .gap(px(8.))
            .pt(px(8.))
            .child(section(trf(
                "Versions that fit {target}",
                &[("target", &target_label)],
            )))
            .when(!vp.other_versions.is_empty(), |d| {
                let count = vp.other_versions.len();
                let label = if count == 1 {
                    tr("1 release for other builds hidden").to_owned()
                } else {
                    trf(
                        "{count} releases for other builds hidden",
                        &[("count", &count.to_string())],
                    )
                };
                d.child(muted(label))
            });

        let versions_card = if !vp.fitting_versions.is_empty() {
            let mut list = card().flex().flex_col();
            for (idx, line) in vp.fitting_versions.iter().enumerate() {
                let is_installed_ver = installed_version.as_deref() == Some(line.version.as_str());
                let badge_el = if is_installed_ver {
                    super::discover::install_fact(&line.version, pending)
                } else if idx == 0 {
                    div()
                        .px(px(8.))
                        .py(px(2.))
                        .rounded(crate::tokens::r(crate::tokens::R_SMALL))
                        .bg(c(p.success).opacity(0.16))
                        .border_1()
                        .border_color(c(p.success).opacity(0.35))
                        .text_size(px(tokens::CAPTION))
                        .text_color(c(p.success))
                        .child(line.verdict.fit_label())
                } else {
                    div()
                        .text_size(px(tokens::SMALL))
                        .text_color(c(p.muted))
                        .child(line.verdict.fit_label())
                };
                list = list.child(
                    self.release_row(("release-row", idx), &vp.id, line, cx)
                        .when(idx > 0, |d| d.border_t_1().border_color(p.rule()))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(2.))
                                .child(
                                    div()
                                        .flex()
                                        .items_baseline()
                                        .gap(px(12.))
                                        .child(
                                            div()
                                                .font_family(crate::theme::MONO)
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_size(px(tokens::LABEL))
                                                .child(line.version.clone()),
                                        )
                                        .child(muted(line.published.clone()))
                                        .child(muted(model::size(line.size))),
                                )
                                .child(made_for(line)),
                        )
                        .child(badge_el),
                );
            }
            list
        } else {
            let mut list = card().flex().flex_col();
            if let Some(reason) = vp.fit.reason() {
                list = list.child(
                    div()
                        .px(px(14.))
                        .py(px(12.))
                        .text_size(px(tokens::LABEL))
                        .text_color(c(p.muted))
                        .child(reason),
                );
            }
            for (idx, line) in vp.other_versions.iter().enumerate() {
                let reason = line.verdict.reason().unwrap_or_default();
                list = list.child(
                    self.release_row(("other-release-row", idx), &vp.id, line, cx)
                        .border_t_1()
                        .border_color(p.rule())
                        .opacity(0.75)
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(2.))
                                .child(
                                    div()
                                        .flex()
                                        .items_baseline()
                                        .gap(px(12.))
                                        .child(
                                            div()
                                                .font_family(crate::theme::MONO)
                                                .text_size(px(tokens::LABEL))
                                                .child(line.version.clone()),
                                        )
                                        .child(muted(line.published.clone())),
                                )
                                .child(made_for(line)),
                        )
                        .child(muted(reason)),
                );
            }
            list
        };
        body.push(
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(versions_heading)
                .child(versions_card)
                .into_any_element(),
        );

        // Right sticky side panel: action card + publisher provenance + pin callout + About card.
        // An update runs under the installed mod's id, which can differ from the project's.
        let installed_id = installed_row
            .map(|row| row.id.as_str())
            .filter(|id| *id != vp.id);
        let working_id = installed_id
            .filter(|id| {
                matches!(
                    self.registry_card(id),
                    Some(crate::mods_page::install::Card::Progress(_))
                        | Some(crate::mods_page::install::Card::Waiting)
                )
            })
            .unwrap_or(&vp.id);
        let working = self.registry_card_control(working_id, |label| {
            dim(secondary("alt-project-add", label).w_full()).into_any_element()
        });
        let update = installed_row
            .zip(cells.as_ref())
            .filter(|(_, cells)| cells.action == Some(model::RowAction::Update))
            .and_then(|(row, _)| row.update.as_ref().map(|to| (row, to)));
        let primary_control: AnyElement = if let Some(control) = working {
            control
        } else if let Some((row, to)) = update {
            let b = primary(
                "alt-project-update",
                trf(
                    "Update {from} → {to}",
                    &[("from", &row.version), ("to", to)],
                ),
            )
            .w_full();
            if busy {
                dim(b).into_any_element()
            } else {
                let row = row.clone();
                b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.update_one(&row);
                    cx.notify();
                }))
                .into_any_element()
            }
        } else if let Some(version) = &installed_version {
            super::discover::install_fact(version, pending).into_any_element()
        } else if vp.will_install.is_some() {
            let id = vp.id.clone();
            primary("alt-project-add", self.registry_install_label())
                .w_full()
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.add_v2(id.clone());
                    cx.notify();
                }))
                .into_any_element()
        } else {
            dim(secondary("alt-project-add", tr("Install")))
                .w_full()
                .into_any_element()
        };

        let meta_sub = match &vp.will_install {
            Some(v) => trf(
                "{version} · {size}",
                &[("version", &v.version), ("size", &model::size(v.size))],
            ),
            None => vp.fit.reason().unwrap_or_default(),
        };
        let repo_href = vp.repository_url.as_ref().map(|link| match link {
            SourceLink::GitHub(url) | SourceLink::Other(url) => url.clone(),
        });
        let published_from_text = trf(
            "Published from github.com/{repo}",
            &[("repo", &vp.repository_name)],
        );
        let published_row = {
            let row = div()
                .id("alt-project-publisher")
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(tokens::SMALL))
                .text_color(c(p.muted))
                .child(crate::glass_ui::icon("github", 14., c(p.muted)))
                .child(div().min_w_0().truncate().child(published_from_text));
            if let Some(url) = repo_href {
                row.cursor_pointer()
                    .hover(|s| s.text_color(c(p.accent_ink)))
                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                        Self::open_url(&url, cx);
                    }))
            } else {
                row
            }
        };

        let link_url = |link: &Option<SourceLink>| -> Option<String> {
            link.as_ref().map(|l| match l {
                SourceLink::GitHub(url) | SourceLink::Other(url) => url.clone(),
            })
        };
        let source_url = link_url(&vp.source).or_else(|| link_url(&vp.repository_url));
        let homepage_url = link_url(&vp.homepage);
        let issues_url = link_url(&vp.issues);

        // The action column: the one action, what it will change, then the secondary link.
        let action_col = action_column()
            .child(muted(meta_sub))
            .children(vp.will_install.as_ref().map(|line| {
                div()
                    .text_size(px(tokens::CAPTION))
                    .text_color(c(p.success))
                    .child(line.verdict.fit_label())
            }))
            .child(primary_control)
            .children(self.registry_card_outcome(&vp.id, cx))
            .children(installed_id.and_then(|id| self.registry_card_outcome(id, cx)))
            .when(vp.will_install.is_none(), |d| {
                d.children(vp.fit.reason().map(note))
            })
            .children(controls)
            .children(source_url.clone().map(|url| {
                secondary("alt-v2-source-open", tr("Open source")).on_click(cx.listener(
                    move |_, _: &ClickEvent, _, cx| {
                        Self::open_url(&url, cx);
                    },
                ))
            }));

        let reviewed = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(px(tokens::SMALL))
            .text_color(c(p.muted))
            .child(crate::glass_ui::icon("tick-02", 14., c(p.muted)))
            .child(tr("Reviewed by Sunrise"));

        let mut about = card()
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(
                div()
                    .pb(px(6.))
                    .text_size(px(tokens::CAPTION))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(c(p.muted))
                    .child(tr("ABOUT")),
            )
            .child(about_row(tr("In-game ID"), vp.mod_id.clone()))
            .when(!vp.updated.is_empty(), |d| {
                d.child(about_row(tr("Updated"), vp.updated.clone()))
            })
            .when(!vp.license.is_empty(), |d| {
                d.child(about_row(tr("Licence"), vp.license.clone()))
            })
            .child(div().h(px(1.)).my(px(8.)).bg(p.rule()))
            .child(reviewed)
            .child(published_row);

        if let Some(url) = homepage_url {
            about = about.child(about_link("alt-v2-homepage", tr("Homepage")).on_click(
                cx.listener(move |_, _: &ClickEvent, _, cx| {
                    Self::open_url(&url, cx);
                }),
            ));
        }
        if let Some(url) = issues_url {
            about = about.child(
                about_link("alt-v2-issues", tr("Issues")).on_click(cx.listener(
                    move |_, _: &ClickEvent, _, cx| {
                        Self::open_url(&url, cx);
                    },
                )),
            );
        }

        let side = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(about)
            .child(card().p(px(16.)).child(self.rating_block(&vp.id, cx)));

        self.project_layout(
            "project",
            super::Layout {
                header,
                head: header_block,
                action: Some(action_col),
                gallery,
                side: Some(side),
            },
            body,
        )
    }

    /// One dependency, as Modrinth lists them: its picture and name, which open its page when the
    /// registry has it, the versions it must be, and whether it is required.
    fn v2_dependency_line(
        &mut self,
        n: usize,
        dep: &super::model::discover_v2::V2Dependency,
        cx: &mut Context<Self>,
    ) -> Div {
        use super::model::discover_v2::V2Provider;
        let p = palette();
        let (picture, id, name, detail) = match &dep.provider {
            V2Provider::Listed { id, name, icon } => (
                icon.as_deref()
                    .and_then(|rel| self.v2_mod_image(id, rel, images::ROW)),
                id.clone(),
                name.clone(),
                None,
            ),
            V2Provider::Several(count) => (
                None,
                dep.mod_id.clone(),
                dep.mod_id.clone(),
                Some(trf(
                    "{count} mods on the registry provide it: installing asks which one",
                    &[("count", &count.to_string())],
                )),
            ),
            V2Provider::Missing => (
                None,
                dep.mod_id.clone(),
                dep.mod_id.clone(),
                Some(tr("Not on the registry").to_owned()),
            ),
        };
        let detail = match &dep.no_release_for {
            Some(build) => Some(trf(
                "Has no release for Sunrise {build}.",
                &[("build", build)],
            )),
            None => detail,
        };
        let mut title = div()
            .id(("alt-v2-dependency", n))
            .truncate()
            .font_weight(FontWeight::SEMIBOLD)
            .text_size(px(tokens::LABEL))
            .child(name.clone());
        if let V2Provider::Listed { id, .. } = &dep.provider {
            let project = id.clone();
            title = title
                .reach()
                .cursor_pointer()
                .hover(|s| s.text_color(c(p.accent_ink)))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.mods_show_dependency(project.clone());
                    cx.notify();
                }));
        }
        let range = dep
            .range
            .clone()
            .unwrap_or_else(|| tr("any version").to_owned());
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(14.))
            .py(px(10.))
            .when(n > 0, |d| d.border_t_1().border_color(p.rule()))
            .child(tile(picture, &id, &name, 36.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(px(8.))
                            .min_w_0()
                            .child(title)
                            .child(
                                div()
                                    .flex_none()
                                    .font_family(crate::theme::MONO)
                                    .text_size(px(tokens::SMALL))
                                    .text_color(c(p.muted))
                                    .child(range),
                            ),
                    )
                    .children(detail.map(|text| div().truncate().child(muted(text)))),
            )
            .child(match dep.required {
                true => tag(tr("Required"), chip::Kind::Neutral),
                false => tag(tr("Optional"), chip::Kind::Neutral),
            })
    }

    /// Captioned screenshot gallery for a v2 mod page: fetches verified content-addressed images
    /// only when the mod page is open.
    fn v2_gallery(
        &mut self,
        project_id: &str,
        screenshots: &[super::model::discover_v2::V2Screenshot],
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        if screenshots.is_empty() {
            return None;
        }
        let p = palette();
        let pictures: Rc<[super::viewer::Picture]> = screenshots
            .iter()
            .map(|shot| super::viewer::Picture::V2 {
                project_id: project_id.to_owned(),
                relative_path: shot.path.clone(),
                caption: shot.caption.clone(),
            })
            .collect::<Vec<_>>()
            .into();
        let count = screenshots.len();
        let content = self.content_width();
        let avail = if content < 860. {
            (content - 62.).max(240.)
        } else {
            (content - 402.).max(280.)
        };
        let columns = if count == 1 {
            1
        } else if avail < 680. {
            2
        } else {
            count.min(3)
        };
        let tile_w = (avail - 10. * (columns - 1) as f32) / columns as f32;
        let max_h = if count == 1 { 320. } else { 240. };
        let height = (tile_w * (9. / 16.)).clamp(110., max_h).round();
        let viewing = self.mods_state.viewer.is_some();

        let mut items = screenshots.iter().enumerate().map(|(i, shot)| {
            let photo = self.v2_mod_image(project_id, &shot.path, images::GALLERY);
            let all = pictures.clone();
            let tip_text = if shot.caption.is_empty() {
                trf(
                    "Picture {number} of {total}",
                    &[
                        ("number", &(i + 1).to_string()),
                        ("total", &count.to_string()),
                    ],
                )
            } else {
                shot.caption.clone()
            };
            div()
                .flex_1()
                .min_w_0()
                .max_w(px(height * (16. / 9.)))
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    div()
                        .id(ElementId::named_usize("v2-gallery-picture", i))
                        .reach_edge()
                        .h(px(height))
                        .w_full()
                        .cursor_pointer()
                        .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                        .border_1()
                        .border_color(p.rule())
                        .bg(p.card())
                        .overflow_hidden()
                        .when(!viewing, |d| d.tooltip(crate::glass_ui::tip(tip_text)))
                        .children(photo.map(|path| {
                            gpui::img(path)
                                .size_full()
                                .object_fit(gpui::ObjectFit::Contain)
                        }))
                        .track_scroll(&super::viewer::tile(i))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.viewer_open(all.clone(), i);
                            cx.notify();
                        })),
                )
                .when(!shot.caption.is_empty(), |d| {
                    d.child(
                        div()
                            .text_size(px(tokens::SMALL))
                            .text_color(c(p.muted))
                            .line_clamp(2)
                            .text_ellipsis()
                            .child(shot.caption.clone()),
                    )
                })
        });

        let mut grid = div().flex().flex_col().flex_none().gap(px(10.));
        for _ in 0..count.div_ceil(columns) {
            grid = grid.child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(10.))
                    .children(items.by_ref().take(columns)),
            );
        }
        Some(grid)
    }
}

impl Launcher {
    /// One release row on a mod's page: pressing it opens the builds the release works on.
    fn release_row(
        &self,
        id: (&'static str, usize),
        project: &str,
        line: &model::discover_v2::V2VersionLine,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let (project, version) = (project.to_owned(), line.version.clone());
        div()
            .id(id)
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .px(px(14.))
            .py(px(10.))
            .cursor_pointer()
            .hover(|d| d.bg(c(palette().line).opacity(0.35)))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.open_release_builds(&project, &version);
                cx.notify();
            }))
    }
}
