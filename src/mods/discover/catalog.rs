//! Discover's page body: the registry read (loading, failed or ready), the notices above the list,
//! and the list of mod cards with its empty states.
use super::*;

impl Launcher {
    pub(in crate::mods) fn discover(
        &mut self,
        window: &mut Window,
        focus: &FocusHandle,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let page = self.discover_shell(KindTab::Mods, cx);
        if let V2Catalog::Failed(error) = &self.mods_state.v2_catalog {
            let error = error.to_string();
            return page
                .child(self.registry_failed(&error, cx))
                .into_any_element();
        }
        if matches!(self.mods_state.v2_catalog, V2Catalog::Ready { .. }) {
            let search = self.discover_search(KindTab::Mods, focus, window, cx);
            return self.discover_ready(page, search, cx).into_any_element();
        }
        page.child(self.registry_loading()).into_any_element()
    }

    /// The registry is still being read: placeholder cards in the list's shape, breathing.
    fn registry_loading(&self) -> Div {
        div()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .child(crate::widgets::feedback::skeleton_cards(1, 6))
    }

    /// The registry can't be read and nothing saved is shown: why, and Try again.
    fn registry_failed(&self, error: &str, cx: &mut Context<Self>) -> Div {
        let retry = secondary("alt-retry", tr("Try again")).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                this.ensure_v2_catalog(true);
                cx.notify();
            },
        ));
        empty_state_icon(
            Some("alert-02"),
            tr("The Sunrise registry can't be reached"),
            error.to_owned(),
            Some(retry.into_any_element()),
        )
    }

    /// The registry is read: the notices, the filter row, the list and its empty states.
    fn discover_ready(&mut self, page: Div, search: Stateful<Div>, cx: &mut Context<Self>) -> Div {
        let (cards, collections, categories) = match &self.mods_state.v2_catalog {
            V2Catalog::Ready {
                cards,
                collections,
                categories,
                ..
            } => (cards.clone(), collections.clone(), categories.clone()),
            _ => return page,
        };
        let f = self.mods_state.filters.clone();
        let mut shown = discover_v2::discover(&cards, &f);
        discover_v2::fold_members(&mut shown, &collections, &f);
        let total = shown.len();
        let hidden_other_builds = discover_v2::hidden_other_build_count(&cards, &f);
        let fitting_shown = if f.show_all {
            shown.iter().filter(|&&i| cards[i].fit.fits()).count()
        } else {
            total
        };
        let other_shown = total.saturating_sub(fitting_shown);
        let profile_name = self
            .selected_instance()
            .map(|i| i.name.clone())
            .unwrap_or_else(|| tr("Default").to_owned());
        let build_label = self
            .selected_exact_build()
            .as_deref()
            .map(crate::sheets::build_name);

        let (category_items, category_picks) =
            menu_parts(category_options(&categories, f.category.as_ref()));
        let category_menu = self.discover_menu(
            "alt-category-menu",
            Menu::Category,
            category_items,
            category_picks,
            Some(220.),
            cx,
        );
        let sort_now = f.sort;
        let (sort_items, sort_picks) = menu_parts(sort_options(sort_now));
        let sort_menu = self.discover_menu(
            "alt-sort-menu",
            Menu::Sort,
            sort_items,
            sort_picks,
            None,
            cx,
        );
        let mut page = page.child(self.filter_bar(
            search,
            category_menu,
            f.show_all,
            |this| {
                this.mods_state.filters.show_all = !this.mods_state.filters.show_all;
                this.mods_state.limit = 0;
            },
            sort_menu,
            cx,
        ));
        page = page.children(self.discover_notices(cx));
        if total > 0 {
            let count = CountLine {
                total,
                fitting: fitting_shown,
                other: other_shown,
                show_all: f.show_all,
                profile: &profile_name,
                build: build_label.as_deref(),
            };
            page = page.child(count_line(count, f.sort.summary_label()));
        }
        let (grid, ungrouped_cards) = self.discover_list(&shown, &cards, &collections, cx);
        // Competing ports sharing an in-game ID render as alternatives, never as one install.
        let port_groups = self.discover_port_groups();
        let profile_target = self.adding_to_name();
        let empty = self.discover_empty_state(
            &f,
            hidden_other_builds,
            !cards.is_empty(),
            &profile_target,
            cx,
        );
        let results = self.with_port_groups(grid, ungrouped_cards, &port_groups, cx);
        let shown_collections = discover_v2::discover_collections(&collections, &cards, &f);
        let results = self.with_collections(results, &collections, &shown_collections, &cards, cx);
        let shown_total = total.max(port_groups.len()).max(shown_collections.len());
        page.child(self.discover_results(results, shown_total, "discover", empty, cx))
    }

    /// Notices above the list: a stale or cached registry, a fit check that stopped, a build the
    /// registry doesn't list yet.
    fn discover_notices(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out: Vec<AnyElement> = Vec::new();
        let build_label = self
            .selected_exact_build()
            .as_deref()
            .map(crate::sheets::build_name);
        if let V2Catalog::Ready {
            stale_warning,
            from_cache,
            build_listed,
            ..
        } = &self.mods_state.v2_catalog
        {
            if let Some(warning) = stale_warning {
                out.push(registry_notice(
                    warning.clone(),
                    "alt-v2-refresh",
                    tr("Refresh"),
                    |this| this.ensure_v2_catalog(true),
                    cx,
                ));
            } else if *from_cache {
                out.push(registry_notice(
                    tr("Showing the saved registry list while offline."),
                    "alt-v2-offline-retry",
                    tr("Try again"),
                    |this| this.ensure_v2_catalog(true),
                    cx,
                ));
            }
            if !build_listed {
                if let Some(build) = build_label.as_deref() {
                    out.push(
                        muted(trf(
                            "{build} isn't in the Sunrise registry yet: mods are checked on this computer.",
                            &[("build", build)],
                        ))
                        .into_any_element(),
                    );
                }
            }
        }
        if let Some(error) = self.local_fit_failure() {
            let line = trf(
                "Couldn't check which mods fit {build}: {error}",
                &[
                    ("build", build_label.as_deref().unwrap_or_default()),
                    ("error", error),
                ],
            );
            out.push(registry_notice(
                line,
                "alt-local-fit-retry",
                tr("Try again"),
                |this| this.retry_local_fit(),
                cx,
            ));
        }
        out
    }

    /// The cards for `shown`, one to a row, culled so a long list draws only the rows near the view.
    /// Also returns how many cards are not grouped as ports.
    fn discover_list(
        &mut self,
        shown: &[usize],
        cards: &[discover_v2::V2Card],
        collections: &[discover_v2::V2Collection],
        cx: &mut Context<Self>,
    ) -> (Div, usize) {
        let limit = self.mods_state.limit.max(PAGE).min(shown.len());
        let port_groups = self.discover_port_groups();
        let grouped_ids = super::super::ports::grouped_project_ids(&port_groups);
        let visible: Vec<(usize, usize)> = shown
            .iter()
            .take(limit)
            .enumerate()
            .filter(|&(_, &i)| !grouped_ids.contains(&cards[i].id))
            .map(|(n, &i)| (n, i))
            .collect();
        let cull = self.mods.discover_list.clone();
        cull.items({
            use std::hash::{Hash, Hasher};
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            for &(_, i) in &visible {
                cards[i].id.hash(&mut hash);
            }
            hash.finish()
        });
        let scroll = crate::frame::scrolls("discover");
        let grid = cull.column(
            &scroll,
            visible.len(),
            10.,
            |_| false,
            |row_idx| {
                let (n, i) = visible[row_idx];
                self.discover_grid()
                    .child(self.discover_card(n, i, cards, collections, cx))
            },
        );
        (grid, visible.len())
    }

    /// One registry mod's card: its artwork and facts, its control, and what it opens.
    fn discover_card(
        &mut self,
        n: usize,
        i: usize,
        cards: &[discover_v2::V2Card],
        collections: &[discover_v2::V2Collection],
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let card_data = &cards[i];
        let id = card_data.id.clone();
        // Icons are loaded strictly from verified content-addressed pages/<id>/<sha256>.<png|jpg>.
        // Screenshots are never fetched on Discover.
        let picture = card_data
            .icon
            .as_deref()
            .and_then(|rel| self.v2_mod_image(&id, rel, images::ICON));
        let package = self.discover_installed().and_then(|rows| {
            let entry = rows.registry_entry(&id)?;
            rows.archives.get(&entry.file)?.icon.clone()
        });
        let working = self
            .card_working_label(&id)
            .map(|label| dim(secondary(format!("alt-add-{}", n), label)).into_any_element());
        let issue = self.registry_card_note(&id);
        let control = self.card_control(n, &id, card_data.fit.fits(), working, cx);

        let mut facts: Vec<Div> = vec![tag(tr("Sunrise"), chip::Kind::Sunrise)];
        if let Some(pill) = card_data.fit.fit_pill() {
            facts.push(fit_pill(pill));
            facts.push(tag(card_data.make.label(), chip::Kind::Neutral));
        } else if let Some(reason) = card_data.fit.reason() {
            facts.push(match card_data.fit {
                discover_v2::CardFit::Checking { .. } => muted(reason),
                _ => unfit_pill(reason),
            });
        }
        if let Some(collection) = collections
            .iter()
            .find(|c| c.members.iter().any(|&(member, _)| member == i))
        {
            facts.push(muted(trf(
                "Part of {collection}",
                &[("collection", &collection.name)],
            )));
        }
        let mut card_el = browse_card(
            ("alt-card", n),
            mod_artwork(picture, package),
            &id,
            &card_data.name,
            &card_data.authors,
            &card_data.summary,
            facts,
            None,
            control,
            issue,
        );
        if card_data.fit.faded() {
            card_el = card_el.opacity(0.62);
        }
        card_el.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.mods_show_v2_project(id.clone());
            cx.notify();
        }))
    }

    /// What a registry card says where Install goes: a working label, Finishing, the installed
    /// version (or Off), Install, or Install dimmed when the mod doesn't fit this build.
    fn card_control(
        &mut self,
        n: usize,
        id: &str,
        fits: bool,
        working: Option<AnyElement>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if working.is_some() {
            return working;
        }
        let installed = self.discover_installed();
        let entry = installed.and_then(|rows| rows.registry_entry(id));
        let installed_version = entry.map(|entry| entry.number.clone());
        // An installed mod says so once, where Install would be: its version, and Off when the mod
        // is turned off (a build switch holds off a mod that doesn't fit the new build).
        let off = entry
            .and_then(sunrise::mod_id)
            .zip(installed)
            .is_some_and(|(mod_id, rows)| !rows.enabled.contains(mod_id));
        // An install that just ended keeps saying so until the installed list it changed has been
        // read again, rather than offering Install for a moment.
        let rereading = installed_version.is_none()
            && self.mods_state.snapshot.as_ref().is_some_and(|s| s.reading)
            && matches!(
                self.registry_card(id),
                Some(crate::mods_page::install::Card::Done(outcome))
                    if outcome.failure.is_none()
            );
        if rereading {
            return Some(
                dim(secondary(format!("alt-add-{}", n), tr("Finishing…"))).into_any_element(),
            );
        }
        if let (Some(rows), Some(version)) = (installed, installed_version.as_deref()) {
            return Some(if off {
                status(
                    "minus-sign",
                    trf("Off · {version}", &[("version", version)]),
                    palette().muted,
                )
                .into_any_element()
            } else {
                install_fact(version, rows.registry_pending(id)).into_any_element()
            });
        }
        if !fits {
            // Non-fitting cards in Show all display Install disabled.
            return Some(
                dim(secondary(format!("alt-add-{}", n), tr("Install")))
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .into_any_element(),
            );
        }
        let id_for_add = id.to_owned();
        Some(
            secondary(format!("alt-add-{}", n), tr("Install"))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.add_v2(id_for_add.clone());
                    cx.notify();
                }))
                .into_any_element(),
        )
    }

    fn discover_empty_state(
        &self,
        f: &discover_v2::V2Filters,
        hidden_other_builds: usize,
        has_any_cards: bool,
        profile_target: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let show_all = |cx: &mut Context<Self>| {
            primary("alt-discover-show-all", tr("Show all")).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    this.mods_state.filters.show_all = true;
                    this.mods_state.limit = 0;
                    cx.notify();
                },
            ))
        };
        if !f.show_all && (hidden_other_builds > 0 || has_any_cards) {
            let title = if f.text.is_empty() && f.category.is_none() {
                trf("No mods fit {target} yet", &[("target", profile_target)])
            } else {
                trf(
                    "No matching mods fit {target}",
                    &[("target", profile_target)],
                )
            };
            let line = if hidden_other_builds > 0 {
                if !f.text.trim().is_empty() {
                    if hidden_other_builds == 1 {
                        trf(
                            "1 mod matches \"{query}\" on other Sunrise builds. Turn on Show all to see which builds it fits.",
                            &[("query", f.text.trim())],
                        )
                    } else {
                        trf(
                            "{count} mods match \"{query}\" on other Sunrise builds. Turn on Show all to see which builds they fit.",
                            &[
                                ("count", &hidden_other_builds.to_string()),
                                ("query", f.text.trim()),
                            ],
                        )
                    }
                } else if hidden_other_builds == 1 {
                    tr("1 mod is available on other Sunrise builds. Turn on Show all to see which builds it fits.")
                        .to_owned()
                } else {
                    trf(
                        "{count} mods are available on other Sunrise builds. Turn on Show all to see which builds they fit.",
                        &[("count", &hidden_other_builds.to_string())],
                    )
                }
            } else {
                tr("Try a different search, or turn on Show all to see mods for other Sunrise builds.")
                    .to_owned()
            };
            return empty_state_icon(
                Some("search-01"),
                title,
                line,
                Some(show_all(cx).into_any_element()),
            )
            .into_any_element();
        }
        let has_filters = !f.text.trim().is_empty() || f.category.is_some();
        if !has_any_cards && !has_filters {
            // Show all lists every build, so an empty list there means an empty registry.
            let (title, line) = if f.show_all {
                (
                    tr("The registry lists no mods yet").to_owned(),
                    tr("Mods show up here once the registry lists their releases."),
                )
            } else {
                (
                    trf("No mods fit {target} yet", &[("target", profile_target)]),
                    tr("Mods show up here once the registry lists releases for this build."),
                )
            };
            return empty_state_icon(Some("puzzle"), title, line, None).into_any_element();
        }
        let action = has_filters.then(|| {
            secondary("alt-discover-clear", tr("Clear filters")).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    this.mods_state.filters.text.clear();
                    this.mods_state.filters.category = None;
                    this.mods_state.find.set(String::new());
                    this.mods_state.limit = 0;
                    cx.notify();
                },
            ))
        });
        empty_state_icon(
            Some("search-01"),
            tr("No matching mods"),
            tr("Try a different search or clear your category filter."),
            action.map(|action| action.into_any_element()),
        )
        .into_any_element()
    }
}

/// What the count line says about the list: how many fit the profile and build.
struct CountLine<'a> {
    total: usize,
    fitting: usize,
    other: usize,
    show_all: bool,
    profile: &'a str,
    build: Option<&'a str>,
}

/// The count line above the list, with the sort's summary on the right.
fn count_line(count: CountLine<'_>, sort_summary: &str) -> Div {
    let line = summary_count_line(
        count.total,
        count.fitting,
        count.other,
        count.show_all,
        count.profile,
        count.build,
    );
    div()
        .flex()
        .flex_none()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap(px(8.))
        .child(muted(line))
        .child(muted(sort_summary.to_owned()))
}

/// A card-shaped notice above the list: its text in the warning colour, and one action.
fn registry_notice(
    text: impl Into<SharedString>,
    id: &'static str,
    label: &'static str,
    action: fn(&mut Launcher),
    cx: &mut Context<Launcher>,
) -> AnyElement {
    card()
        .px(px(12.))
        .py(px(8.))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(10.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(tokens::SMALL))
                .text_color(c(palette().warning))
                .child(text.into()),
        )
        .child(
            secondary(id, label).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                action(this);
                cx.notify();
            })),
        )
        .into_any_element()
}

/// The category options: "All categories", then each category of the registry.
fn category_options(categories: &[String], current: Option<&String>) -> Vec<MenuOption> {
    std::iter::once((
        tr("All categories").into(),
        current.is_none(),
        Box::new(|this: &mut Launcher| {
            this.mods_state.filters.category = None;
            this.mods_state.limit = 0;
        }) as Box<dyn Fn(&mut Launcher)>,
    ))
    .chain(categories.iter().map(|k| {
        let key = k.clone();
        (
            category_label(k).into(),
            current == Some(k),
            Box::new(move |this: &mut Launcher| {
                this.mods_state.filters.category = Some(key.clone());
                this.mods_state.limit = 0;
            }) as Box<dyn Fn(&mut Launcher)>,
        )
    }))
    .collect()
}

/// The sort options, the one in use marked.
fn sort_options(current: V2Sort) -> Vec<MenuOption> {
    V2Sort::ALL
        .into_iter()
        .map(|s| {
            (
                s.label().into(),
                current == s,
                Box::new(move |this: &mut Launcher| {
                    this.mods_state.filters.sort = s;
                    this.mods_state.limit = 0;
                }) as Box<dyn Fn(&mut Launcher)>,
            )
        })
        .collect()
}
