//! The Installed and project pages' competing-ports views.

use super::*;
use crate::widgets::button::secondary;

impl Launcher {
    pub(crate) fn refresh_mods_after_port_switch(
        &mut self,
        notice: Result<String, String>,
        open_project_id: Option<String>,
    ) {
        self.mods_state.snapshot = None;
        self.mods_state.notice = Some(notice);
        if let Some(id) = open_project_id.filter(|_| self.ui.page == Page::Project) {
            self.mods_show_project(id);
        }
    }

    pub(in crate::mods) fn discover_port_groups(&self) -> Vec<PortGroup> {
        let Some(catalog) = v2::known_index() else {
            return Vec::new();
        };
        // This computer's fit checks fill what the index says nothing of, as installs plan.
        let index = &crate::native_mods::local_fit::filled(&catalog.index);
        let mut installed_map = BTreeMap::new();
        if let Some(rows) = self.discover_installed() {
            for entry in &rows.entries {
                if entry.source != SourceId::Sunrise || entry.kind != Kind::Mod {
                    continue;
                }
                let project = index
                    .project(&entry.project)
                    .or_else(|| index.project_by_slug(&entry.slug));
                let Some(mod_id) = project
                    .map(|p| p.mod_id.as_str())
                    .or_else(|| sunrise::mod_id(entry))
                else {
                    continue;
                };
                installed_map.insert(
                    mod_id.to_owned(),
                    InstalledPort {
                        project_id: project
                            .map(|p| p.id.clone())
                            .unwrap_or_else(|| entry.project.clone()),
                        version: entry.number.clone(),
                        repo: project
                            .map(|p| p.repository.name.clone())
                            .unwrap_or_else(|| entry.project.clone()),
                    },
                );
            }
        }
        let build_id = self.ports_build();
        group_ports(
            index,
            build_id.as_deref(),
            &installed_map,
            &self.mods_state.filters.text,
            self.mods_state.filters.category.as_deref(),
            self.mods_state.filters.show_all,
        )
    }

    /// The selected profile's exact Sunrise build, as the index's `fits` lists name it: a native
    /// profile's version carries a build suffix that its base build drops.
    fn ports_build(&self) -> Option<String> {
        let raw = self.profile_target(self.selected_instance()?)?;
        if native_mods::is_native(&raw) {
            crate::profile_builds::native_base(&raw).or(Some(raw))
        } else {
            Some(raw)
        }
    }

    pub(in crate::mods) fn with_port_groups(
        &mut self,
        grid: Div,
        ungrouped_cards: usize,
        groups: &[PortGroup],
        cx: &mut Context<Self>,
    ) -> Div {
        if groups.is_empty() {
            grid
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(14.))
                .child(self.render_port_groups(groups, cx))
                .when(ungrouped_cards > 0, |d| d.child(grid))
        }
    }

    fn render_port_groups(&mut self, groups: &[PortGroup], cx: &mut Context<Self>) -> Div {
        let busy = self.mods_busy() || self.mods_locked().is_some();
        let mut container = div().flex().flex_col().gap(px(12.));
        for (group_idx, group) in groups.iter().enumerate() {
            let header = div()
                .flex()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap(px(8.))
                .child(
                    div()
                        .text_size(px(tokens::BODY))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(c(palette().text))
                        .child(trf(
                            "{count} ports of {name}",
                            &[
                                ("count", &group.ports.len().to_string()),
                                ("name", &group.display_name),
                            ],
                        )),
                )
                .child(muted(trf(
                    "Same in-game ID ({id}) · Pick one provider per profile",
                    &[("id", &group.mod_id)],
                )));

            let mut cards_row = div().flex().flex_wrap().gap(px(12.));
            for (port_idx, port) in group.ports.iter().enumerate() {
                let picture = self.v2_mod_image(&port.project_id, &port.icon, images::ICON);
                let is_installed = port.action == PortAction::Installed;
                let success = c(palette().success);
                let fit_badge = port.latest_version.as_ref().map(|ver| {
                    let (label, color) = match port.fit {
                        FitVerdict::Fits => (
                            trf("{version} · Fits build", &[("version", ver)]),
                            palette().success,
                        ),
                        FitVerdict::DoesNotFit => (
                            trf("{version} · Doesn't fit build", &[("version", ver)]),
                            palette().muted,
                        ),
                        FitVerdict::Unknown => (
                            trf("{version} · Not checked yet", &[("version", ver)]),
                            palette().muted,
                        ),
                    };
                    div()
                        .flex_none()
                        .px(px(8.))
                        .py(px(3.))
                        .rounded(crate::tokens::r(crate::tokens::R_SMALL))
                        .border_1()
                        .border_color(c(color).opacity(0.4))
                        .bg(c(color).opacity(0.1))
                        .text_size(px(tokens::CAPTION))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(c(color))
                        .child(label)
                });

                let view_id = port.project_id.clone();
                let view_btn = secondary(
                    format!("alt-port-view-{}", group_idx * 100 + port_idx),
                    tr("View"),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.mods_show_project(view_id.clone());
                    cx.notify();
                }));

                let action_el: AnyElement = match port.action {
                    PortAction::Installed => div()
                        .flex_none()
                        .h(px(tokens::CONTROL))
                        .px(px(12.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                        .border_1()
                        .border_color(success.opacity(0.45))
                        .bg(success.opacity(0.12))
                        .text_size(px(tokens::LABEL))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(success)
                        .child(tr("Installed"))
                        .into_any_element(),
                    PortAction::SwitchToPort => {
                        let btn = secondary(
                            format!("alt-port-switch-{}", group_idx * 100 + port_idx),
                            tr("Switch to this port"),
                        );
                        if let Some(req) = port.switch_request.clone().filter(|_| !busy) {
                            btn.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.begin_provider_switch(req.clone());
                                cx.notify();
                            }))
                            .into_any_element()
                        } else {
                            dim(btn).into_any_element()
                        }
                    }
                    PortAction::Install => {
                        let btn = secondary(
                            format!("alt-port-install-{}", group_idx * 100 + port_idx),
                            tr("Install"),
                        );
                        if busy {
                            dim(btn).into_any_element()
                        } else {
                            let id = port.project_id.clone();
                            btn.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.add_v2(id.clone());
                                cx.notify();
                            }))
                            .into_any_element()
                        }
                    }
                    PortAction::Unavailable => dim(secondary(
                        format!("alt-port-install-{}", group_idx * 100 + port_idx),
                        tr("Install"),
                    ))
                    .into_any_element(),
                };

                let port_card = card()
                    .flex_1()
                    .min_w(px(300.))
                    .p(px(14.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .when(is_installed, |d| d.border_color(success.opacity(0.45)))
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .justify_between()
                            .gap(px(10.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .items_center()
                                    .gap(px(10.))
                                    .child(tile(picture, &port.project_id, &port.name, 40.))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .flex()
                                            .flex_col()
                                            .gap(px(3.))
                                            .child(
                                                div()
                                                    .truncate()
                                                    .text_size(px(tokens::BODY))
                                                    .font_weight(FontWeight::BOLD)
                                                    .text_color(c(palette().text))
                                                    .child(port.name.clone()),
                                            )
                                            .child(
                                                div()
                                                    .self_start()
                                                    .max_w_full()
                                                    .truncate()
                                                    .px(px(6.))
                                                    .py(px(1.))
                                                    .rounded(crate::tokens::r(
                                                        crate::tokens::R_SMALL,
                                                    ))
                                                    .border_1()
                                                    .border_color(palette().rule())
                                                    .bg(c(palette().field))
                                                    .font_family(crate::theme::MONO)
                                                    .text_size(px(tokens::CAPTION))
                                                    .text_color(c(palette().muted))
                                                    .child(port.publisher_repo.clone()),
                                            ),
                                    ),
                            )
                            .children(fit_badge),
                    )
                    .child(muted(port.summary.clone()))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .justify_between()
                            .gap(px(8.))
                            .child(
                                div().flex().flex_wrap().gap(px(6.)).children(
                                    port.categories
                                        .iter()
                                        .take(3)
                                        .map(|cat| category_chip(category_label(cat))),
                                ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(view_btn)
                                    .child(action_el),
                            ),
                    );
                cards_row = cards_row.child(port_card);
            }

            container = container.child(
                card()
                    .p(px(16.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(header)
                    .child(cards_row),
            );
        }
        container
    }

    pub(in crate::mods) fn other_ports_card(
        &self,
        project_id: &str,
        installed_row: Option<&Row>,
        busy: bool,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let row = installed_row?;
        if row.local.is_some() {
            return None;
        }
        let catalog = v2::known_index()?;
        let profile_name = self
            .selected_instance()
            .map(|i| i.name.clone())
            .unwrap_or_else(|| tr("Sunrise").into());
        let build_id = self.ports_build();
        let current_project = row
            .entry
            .as_ref()
            .map(|entry| entry.project.as_str())
            .unwrap_or(project_id);
        let other = other_ports_for(
            &crate::native_mods::local_fit::filled(&catalog.index),
            current_project,
            &row.id,
            &row.version,
            build_id.as_deref(),
        )?;
        let heading = if other.alternatives.len() == 1 {
            trf(
                "OTHER PORT OF {mod}",
                &[("mod", &other.mod_name.to_uppercase())],
            )
        } else {
            trf(
                "OTHER PORTS OF {mod}",
                &[("mod", &other.mod_name.to_uppercase())],
            )
        };
        let description = if other.alternatives.len() == 1 {
            trf(
                "Another reviewed project provides the {id} in-game ID. Switching replaces this port on {profile}.",
                &[("id", &other.mod_id), ("profile", &profile_name)],
            )
        } else {
            trf(
                "Other reviewed projects provide the {id} in-game ID. Switching replaces this port on {profile}.",
                &[("id", &other.mod_id), ("profile", &profile_name)],
            )
        };

        let mut box_el = card()
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .text_size(px(tokens::CAPTION))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(c(palette().muted))
                    .child(heading),
            )
            .child(muted(description));

        for (i, alt) in other.alternatives.iter().enumerate() {
            let label = trf("Switch to {repo}", &[("repo", &alt.publisher_repo)]);
            let btn = secondary(format!("alt-project-switch-port-{}", i), label).w_full();
            let control = if let Some(req) = alt.switch_request.clone().filter(|_| !busy) {
                btn.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.begin_provider_switch(req.clone());
                    cx.notify();
                }))
            } else {
                dim(btn)
            };
            let fit_note = match alt.fit {
                FitVerdict::Fits => None,
                FitVerdict::DoesNotFit => Some(note(tr("Doesn't fit this build"))),
                FitVerdict::Unknown => Some(note(tr("Not checked for this build yet"))),
            };
            box_el = box_el.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(control)
                    .children(fit_note),
            );
        }
        Some(box_el)
    }
}

fn category_chip(label: String) -> Div {
    div()
        .flex_none()
        .px(px(6.))
        .py(px(1.))
        .rounded(crate::tokens::r(crate::tokens::R_SMALL))
        .border_1()
        .border_color(c(palette().line))
        .text_size(px(tokens::CAPTION))
        .text_color(c(palette().muted))
        .child(label)
}
