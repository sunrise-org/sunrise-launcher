//! Mods › Discover: browses the signed v2 registry index for the selected profile's exact build.
use super::model::discover_v2::{self, V2Sort};
use super::state::Menu;
use super::ui::*;
use super::*;
use crate::paint::palette;
use crate::widgets::button::{primary, secondary};
use crate::widgets::check::switch;
use crate::widgets::chip::{self, tag};
use crate::widgets::feedback::empty_state;
use gpui::{deferred, MouseButton};
use sunrise_launcher::browser::details::category_label;
use sunrise_launcher::browser::images;

/// How many cards are drawn before "Show more".
const PAGE: usize = 36;

pub(super) type MenuOption = (SharedString, bool, Box<dyn Fn(&mut Launcher)>);

impl Launcher {
    pub(super) fn discover(
        &mut self,
        window: &mut Window,
        focus: &FocusHandle,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut page = self.discover_shell(window, focus, cx);
        let (cards, collections, categories, build_listed, stale_warning, from_cache) =
            match &self.mods_state.v2_catalog {
                V2Catalog::Ready {
                    cards,
                    collections,
                    categories,
                    build_listed,
                    stale_warning,
                    from_cache,
                } => (
                    cards.clone(),
                    collections.clone(),
                    categories.clone(),
                    *build_listed,
                    stale_warning.clone(),
                    *from_cache,
                ),
                V2Catalog::Idle | V2Catalog::Loading => {
                    return page
                        .child(muted(tr("Loading the registry…")))
                        .into_any_element();
                }
                V2Catalog::Failed(error) => {
                    let error = error.clone();
                    return page
                        .child(
                            card()
                                .p(px(20.))
                                .flex()
                                .flex_col()
                                .items_start()
                                .gap(px(10.))
                                .child(
                                    div()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(tr("The Sunrise registry can't be reached")),
                                )
                                .child(muted(error))
                                .child(secondary("alt-retry", tr("Try again")).on_click(
                                    cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.ensure_v2_catalog(true);
                                        cx.notify();
                                    }),
                                )),
                        )
                        .into_any_element();
                }
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

        // Filter row: profile switcher, category dropdown on the left; Show all switch and sort on the right.
        let category_now = f
            .category
            .as_deref()
            .map_or_else(|| tr("All categories").to_owned(), category_label);
        let category_options = || -> Vec<MenuOption> {
            std::iter::once((
                tr("All categories").into(),
                f.category.is_none(),
                Box::new(|this: &mut Launcher| {
                    this.mods_state.filters.category = None;
                    this.mods_state.limit = 0;
                }) as Box<dyn Fn(&mut Launcher)>,
            ))
            .chain(categories.iter().map(|k| {
                let key = k.clone();
                (
                    category_label(k).into(),
                    f.category.as_ref() == Some(k),
                    Box::new(move |this: &mut Launcher| {
                        this.mods_state.filters.category = Some(key.clone());
                        this.mods_state.limit = 0;
                    }) as Box<dyn Fn(&mut Launcher)>,
                )
            }))
            .collect()
        };
        let sort_options = || -> Vec<MenuOption> {
            V2Sort::ALL
                .into_iter()
                .map(|s| {
                    (
                        s.label().into(),
                        f.sort == s,
                        Box::new(move |this: &mut Launcher| {
                            this.mods_state.filters.sort = s;
                            this.mods_state.limit = 0;
                        }) as Box<dyn Fn(&mut Launcher)>,
                    )
                })
                .collect()
        };
        let category_menu = self.discover_menu(&category_now, Menu::Category, category_options, cx);
        let sort_menu = self.discover_menu(f.sort.label(), Menu::Sort, sort_options, cx);
        page = page.child(self.filter_bar(
            category_menu,
            f.show_all,
            |this| {
                this.mods_state.filters.show_all = !this.mods_state.filters.show_all;
                this.mods_state.limit = 0;
            },
            sort_menu,
            cx,
        ));

        // Stale or offline cache advisory (non-blocking).
        if let Some(warning) = stale_warning {
            page = page.child(
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
                            .child(warning),
                    )
                    .child(
                        secondary("alt-v2-refresh", tr("Refresh")).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.ensure_v2_catalog(true);
                                cx.notify();
                            },
                        )),
                    ),
            );
        } else if from_cache {
            page = page.child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_between()
                    .gap(px(8.))
                    .child(muted(tr("Showing the saved registry list while offline.")))
                    .child(
                        muted(tr("Try again"))
                            .id("alt-v2-offline-retry")
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.ensure_v2_catalog(true);
                                cx.notify();
                            })),
                    ),
            );
        }

        // This computer's fit check stopped: the cards it would have answered stay unchecked.
        if let Some(error) = self.local_fit_failure() {
            let line = trf(
                "Couldn't check which mods fit {build}: {error}",
                &[
                    ("build", build_label.as_deref().unwrap_or_default()),
                    ("error", error),
                ],
            );
            page = page.child(
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
                            .child(line),
                    )
                    .child(secondary("alt-local-fit-retry", tr("Try again")).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.retry_local_fit();
                            cx.notify();
                        }),
                    )),
            );
        }
        // A build the registry doesn't list yet: its mods are checked here, said once.
        if !build_listed {
            if let Some(build) = build_label.as_deref() {
                page = page.child(muted(trf(
                    "{build} isn't in the Sunrise registry yet: mods are checked on this computer.",
                    &[("build", build)],
                )));
            }
        }

        // Count & ranking summary line when results exist.
        if total > 0 {
            let count_line = summary_count_line(
                total,
                fitting_shown,
                other_shown,
                f.show_all,
                &profile_name,
                build_label.as_deref(),
            );
            page = page.child(
                div()
                    .flex()
                    .flex_none()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap(px(8.))
                    .child(muted(count_line))
                    .child(muted(f.sort.summary_label())),
            );
        }

        // The cards grid, culled by grid row so expanding "Show more" only builds cards near view.
        let limit = self.mods_state.limit.max(PAGE).min(total);
        // Competing ports sharing an in-game ID render as alternatives, never as one install.
        let port_groups = self.discover_port_groups();
        let grouped_ids = super::ports::grouped_project_ids(&port_groups);
        let visible: Vec<(usize, usize)> = shown
            .iter()
            .take(limit)
            .enumerate()
            .filter(|&(_, &i)| !grouped_ids.contains(&cards[i].id))
            .map(|(n, &i)| (n, i))
            .collect();
        let ungrouped_cards = visible.len();
        let cols = columns(self.content_width()).max(1) as usize;
        let row_count = visible.len().div_ceil(cols);
        let cull = self.mods.discover_list.clone();
        cull.items({
            use std::hash::{Hash, Hasher};
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            cols.hash(&mut hash);
            for &(_, i) in &visible {
                cards[i].id.hash(&mut hash);
            }
            hash.finish()
        });
        let scroll = crate::frame::scrolls("discover");
        let grid = cull.column(
            &scroll,
            row_count,
            10.,
            |_| false,
            |row_idx| {
                let start = row_idx * cols;
                let end = (start + cols).min(visible.len());
                let mut row = self.discover_grid();
                for &(n, i) in &visible[start..end] {
                    let card_data = &cards[i];
                    let id = card_data.id.clone();
                    // Icons are loaded strictly from verified content-addressed pages/<id>/<sha256>.<png|jpg>.
                    // Screenshots are never fetched on Discover.
                    let picture = card_data
                        .icon
                        .as_deref()
                        .and_then(|rel| self.v2_mod_image(&id, rel, images::ICON));
                    let installed = self.discover_installed();
                    let entry = installed.and_then(|rows| rows.registry_entry(&id));
                    let installed_version = entry.map(|entry| entry.number.as_str());
                    let package = installed
                        .zip(entry)
                        .and_then(|(rows, entry)| rows.archives.get(&entry.file)?.icon.clone());
                    let fits = card_data.fit.fits();
                    let working = self.card_working_label(&id).map(|label| {
                        dim(secondary(format!("alt-add-{}", n), label)).into_any_element()
                    });
                    let issue = self.registry_card_note(&id);

                    // An installed mod says so once, where Install would be: its version,
                    // and Off when the mod is turned off (a build switch holds off a mod that
                    // doesn't fit the new build).
                    let off = entry
                        .and_then(sunrise::mod_id)
                        .zip(installed)
                        .is_some_and(|(mod_id, rows)| !rows.enabled.contains(mod_id));
                    // An install that just ended keeps saying so until the installed list it
                    // changed has been read again, rather than offering Install for a moment.
                    let rereading = installed_version.is_none()
                        && self.mods_state.snapshot.as_ref().is_some_and(|s| s.reading)
                        && matches!(
                            self.registry_card(&id),
                            Some(crate::mods_page::install::Card::Done(outcome))
                                if outcome.failure.is_none()
                        );
                    let control = if working.is_some() {
                        working
                    } else if rereading {
                        Some(
                            dim(secondary(format!("alt-add-{}", n), tr("Finishing…")))
                                .into_any_element(),
                        )
                    } else if let (Some(rows), Some(version)) = (installed, installed_version) {
                        Some(if off {
                            status(
                                "minus-sign",
                                trf("Off · {version}", &[("version", version)]),
                                palette().muted,
                            )
                            .into_any_element()
                        } else {
                            install_fact(version, rows.registry_pending(&id)).into_any_element()
                        })
                    } else if !fits {
                        // Non-fitting cards in Show all display Install disabled.
                        Some(
                            dim(secondary(format!("alt-add-{}", n), tr("Install")))
                                .on_click(|_, _, cx| cx.stop_propagation())
                                .into_any_element(),
                        )
                    } else {
                        let id_for_add = id.clone();
                        Some(
                            secondary(format!("alt-add-{}", n), tr("Install"))
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    cx.stop_propagation();
                                    this.add_v2(id_for_add.clone());
                                    cx.notify();
                                }))
                                .into_any_element(),
                        )
                    };

                    let mut facts: Vec<Div> = Vec::new();
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
                        control,
                        issue,
                    );
                    if card_data.fit.faded() {
                        card_el = card_el.opacity(0.62);
                    }
                    row = row.child(card_el.on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            this.mods_show_v2_project(id.clone());
                            cx.notify();
                        },
                    )));
                }
                row
            },
        );

        let profile_target = match build_label.as_deref() {
            Some(b) => trf(
                "{profile} ({build})",
                &[("profile", profile_name.as_str()), ("build", b)],
            ),
            None => profile_name.clone(),
        };
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
            .into_any_element()
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
            return empty_state(title, line, Some(show_all(cx).into_any_element()))
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
            return empty_state(title, line, None).into_any_element();
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
        empty_state(
            tr("No matching mods"),
            tr("Try a different search or clear your category filter."),
            action.map(|action| action.into_any_element()),
        )
        .into_any_element()
    }

    /// Discover's filter row, the same for mods and packs: the category menu on the left, the
    /// Show all switch and the sort menu on the right.
    pub(super) fn filter_bar(
        &self,
        category_menu: Div,
        show_all: bool,
        flip_show_all: fn(&mut Launcher),
        sort_menu: Div,
        cx: &mut Context<Self>,
    ) -> Div {
        let show_all_toggle = div()
            .id("alt-show-all")
            .flex()
            .items_center()
            .gap(px(8.))
            .cursor_pointer()
            .child(switch("alt-show-all-switch", show_all).tab_stop(false))
            .child(div().text_size(px(tokens::LABEL)).child(tr("Show all")))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                flip_show_all(this);
                cx.notify();
            }));
        div()
            .flex()
            .flex_none()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(10.))
                    .child(category_menu),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .child(show_all_toggle)
                    .child(sort_menu),
            )
    }

    pub(super) fn discover_shell(
        &mut self,
        window: &mut Window,
        focus: &FocusHandle,
        cx: &mut Context<Self>,
    ) -> Div {
        let header = self.mods_header(false, focus, window, cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(14.))
            .p(px(24.))
            .text_color(c(palette().text))
            .child(header)
            .children(self.strip(cx))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.mods_state.menu.take().is_some() {
                        cx.notify();
                    }
                }),
            )
    }

    /// What a working card says where Install was, in a label about Install's size: the strip
    /// above the page says each step, and the card keeps its shape.
    fn card_working_label(&self, id: &str) -> Option<&'static str> {
        match self.registry_card(id) {
            Some(crate::mods_page::install::Card::Progress(stage)) => {
                Some(if stage == sunrise::InstallProgress::Publishing {
                    tr("Finishing…")
                } else {
                    tr("Installing…")
                })
            }
            Some(crate::mods_page::install::Card::Waiting) => Some(tr("Waiting…")),
            _ => None,
        }
    }

    /// The collections matching Discover's filters, as cards above the mods.
    fn with_collections(
        &mut self,
        results: Div,
        collections: &[discover_v2::V2Collection],
        shown: &[usize],
        cards: &[discover_v2::V2Card],
        cx: &mut Context<Self>,
    ) -> Div {
        if shown.is_empty() {
            return results;
        }
        let mut grid = self.discover_grid();
        for (n, &i) in shown.iter().enumerate() {
            let collection = &collections[i];
            let id = collection.id.clone();
            let picture = collection
                .icon
                .as_deref()
                .and_then(|rel| self.v2_mod_image(&id, rel, images::ICON));
            let (fitting, all) = (collection.fitting(cards), collection.members.len());
            let fit = match fitting {
                0 => unfit_pill(tr("No mod in it fits this build").to_owned()),
                _ => fit_pill(trf(
                    "{fitting} of {all} mods fit build",
                    &[("fitting", &fitting.to_string()), ("all", &all.to_string())],
                )),
            };
            let installed = self.discover_installed();
            let is_installed = |project: &str| {
                installed.is_some_and(|rows| rows.registry_entry(project).is_some())
            };
            let added = collection
                .members
                .iter()
                .filter(|&&(i, _)| is_installed(&cards[i].id))
                .count();
            // Install takes the members on by default that fit, as the page opens picked.
            let to_install =
                collection.to_install(cards, &collection.initial_picks(cards), &is_installed);
            let mut facts = vec![fit, tag(tr("Collection"), chip::Kind::Neutral)];
            if added > 0 {
                facts.push(muted(trf(
                    "{added} of {total} added",
                    &[("added", &added.to_string()), ("total", &all.to_string())],
                )));
            }
            let control = match self.card_working_label(&id) {
                Some(label) => {
                    dim(secondary(format!("alt-collection-add-{}", n), label)).into_any_element()
                }
                None if to_install.is_empty() && added > 0 => added_mark(),
                None if to_install.is_empty() => dim(secondary(
                    format!("alt-collection-add-{}", n),
                    tr("Install"),
                ))
                .on_click(|_, _, cx| cx.stop_propagation())
                .into_any_element(),
                None => {
                    let (label, why) =
                        collection.install_label(cards, to_install.len(), &is_installed);
                    let (card, ids) = (id.clone(), to_install);
                    secondary(format!("alt-collection-add-{}", n), label)
                        .when_some(why, |d, why| d.tooltip(crate::glass_ui::tip(why)))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            this.add_v2_collection(&card, ids.clone());
                            cx.notify();
                        }))
                        .into_any_element()
                }
            };
            let note = self.registry_card_note(&id);
            let card = browse_card(
                ("alt-collection-card", n),
                picture,
                &id,
                &collection.name,
                &collection.publisher,
                &collection.summary,
                facts,
                Some(control),
                note,
            )
            .when(fitting == 0, |card| card.opacity(0.62))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.mods_show_v2_collection(id.clone(), Page::Discover);
                cx.notify();
            }));
            grid = grid.child(card);
        }
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(
                div()
                    .text_size(px(tokens::BODY))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(c(palette().text))
                    .child(tr("Collections")),
            )
            .child(grid)
            .child(results)
    }

    fn discover_grid(&self) -> Div {
        div()
            .grid()
            .grid_cols(columns(self.content_width()))
            .gap(px(10.))
    }

    fn discover_results(
        &self,
        grid: Div,
        total: usize,
        scroll_key: &'static str,
        empty: AnyElement,
        cx: &mut Context<Self>,
    ) -> Div {
        let limit = self.mods_state.limit.max(PAGE).min(total);
        let results = div()
            .id("mods-discover-results")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .pb(px(16.))
                    .when(total == 0, |d| d.child(empty))
                    .when(total > 0, |d| d.child(grid))
                    .when(limit < total, |d| {
                        d.child(
                            div().flex().justify_center().child(
                                secondary(
                                    "mods-discover-more",
                                    trf(
                                        "Show more ({count} left)",
                                        &[("count", &(total - limit).to_string())],
                                    ),
                                )
                                .on_click(cx.listener(
                                    |this, _: &ClickEvent, _, cx| {
                                        this.mods_state.limit =
                                            this.mods_state.limit.max(PAGE) + PAGE;
                                        cx.notify();
                                    },
                                )),
                            ),
                        )
                    }),
            );
        crate::scrollbar::wrap_page_in_margin(scroll_key, results, &self.theme)
    }

    pub(super) fn discover_menu(
        &self,
        label: &str,
        which: Menu,
        options: impl FnOnce() -> Vec<MenuOption>,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = palette();
        let open = self.mods_state.menu == Some(which);
        let popover =
            |items: Vec<MenuOption>, left: bool, cx: &mut Context<Self>| {
                deferred(
                    div()
                        .absolute()
                        .top(px(tokens::CONTROL + 4.))
                        .when(left, |d| d.left(px(0.)))
                        .when(!left, |d| d.right(px(0.)))
                        .min_w(px(190.))
                        .max_h(px(300.))
                        .id("alt-discover-menu-list")
                        .overflow_y_scroll()
                        .p(px(4.))
                        .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                        .bg(c(p.raised))
                        .border_1()
                        .border_color(c(p.line))
                        .shadow_md()
                        .occlude()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .flex()
                        .flex_col()
                        .children(items.into_iter().enumerate().map(
                            |(n, (text, active, pick))| {
                                div()
                                    .id(("alt-menu-opt", n))
                                    .px(px(10.))
                                    .py(px(6.))
                                    .rounded(crate::tokens::r(crate::tokens::R_SMALL))
                                    .text_size(px(tokens::LABEL))
                                    .text_color(c(if active { p.accent_ink } else { p.text }))
                                    .when(active, |d| d.bg(c(p.selected)))
                                    .cursor_pointer()
                                    .hover(|s| s.bg(c(p.hover)))
                                    .child(text)
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        pick(this);
                                        this.mods_state.menu = None;
                                        cx.notify();
                                    }))
                            },
                        )),
                )
            };
        let id = match which {
            Menu::Category => "alt-category-menu",
            Menu::Sort => "alt-sort-menu",
        };
        div()
            .relative()
            .child(
                div()
                    .id(id)
                    .h(px(tokens::CONTROL))
                    .px(px(12.))
                    .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                    .border_1()
                    .border_color(if open {
                        c(p.accent_ink)
                    } else {
                        p.input_edge()
                    })
                    .bg(c(if open { p.selected } else { p.field }))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(tokens::LABEL))
                    .cursor_pointer()
                    .hover(|s| s.border_color(c(p.muted)))
                    .child(label.to_owned())
                    .child(crate::widgets::motion_bits::chevron(
                        format!("discover-menu-{which:?}"),
                        open,
                        12.,
                        c(p.muted),
                    ))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.mods_state.menu = match this.mods_state.menu {
                            Some(m) if m == which => None,
                            _ => Some(which),
                        };
                        cx.notify();
                    })),
            )
            .children(open.then(|| popover(options(), which != Menu::Sort, cx)))
    }

    /// Never label cards from another profile or from an incomplete installed read.
    pub(super) fn discover_installed(&self) -> Option<&InstalledRows> {
        let home = self.mods_home();
        self.mods_state
            .snapshot
            .as_ref()
            // While a re-read runs, the profile's last confirmed rows stay shown.
            .filter(|snap| snap.home == home && snap.error.is_none())
            .map(|snap| &snap.rows)
    }
}

impl InstalledRows {
    pub(super) fn registry_entry(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|entry| {
            entry.source == SourceId::Sunrise && entry.kind == Kind::Mod && entry.project == id
        })
    }

    pub(super) fn registry_pending(&self, id: &str) -> bool {
        self.registry_entry(id)
            .and_then(sunrise::mod_id)
            .is_some_and(|id| self.unbuilt.contains(id))
    }
}

fn summary_count_line(
    total: usize,
    fitting: usize,
    other: usize,
    show_all: bool,
    profile_name: &str,
    build_label: Option<&str>,
) -> String {
    let target = match build_label {
        Some(build) => trf(
            "{profile} ({build})",
            &[("profile", profile_name), ("build", build)],
        ),
        None => profile_name.to_owned(),
    };
    if show_all && other > 0 {
        trf(
            "{total} mods total · {fit} fit {target} · {other} for other builds",
            &[
                ("total", &total.to_string()),
                ("fit", &fitting.to_string()),
                ("target", &target),
                ("other", &other.to_string()),
            ],
        )
    } else if build_label.is_some() {
        if total == 1 {
            trf("1 mod fits {target}", &[("target", &target)])
        } else {
            trf(
                "{count} mods fit {target}",
                &[("count", &total.to_string()), ("target", &target)],
            )
        }
    } else if total == 1 {
        tr("1 mod").to_owned()
    } else {
        trf("{count} mods", &[("count", &total.to_string())])
    }
}

fn fit_pill(label: String) -> Div {
    let p = palette();
    div()
        // A long pill wraps within the card instead of running under its button.
        .min_w_0()
        .px(px(7.))
        .py(px(1.))
        .rounded(crate::tokens::r(crate::tokens::R_SMALL))
        .bg(c(p.success).opacity(0.16))
        .border_1()
        .border_color(c(p.success).opacity(0.35))
        .text_size(px(tokens::CAPTION))
        .text_color(c(p.success))
        .child(label)
}

fn unfit_pill(label: String) -> Div {
    let p = palette();
    div()
        .min_w_0()
        .px(px(7.))
        .py(px(1.))
        .rounded(crate::tokens::r(crate::tokens::R_SMALL))
        .bg(c(p.line).opacity(0.35))
        .border_1()
        .border_color(c(p.line))
        .text_size(px(tokens::CAPTION))
        .text_color(c(p.muted))
        .child(label)
}

pub(super) fn install_fact(version: &str, pending: bool) -> Div {
    let text = if pending {
        trf(
            "Installed {version} · awaiting build",
            &[("version", version)],
        )
    } else {
        trf("Installed {version}", &[("version", version)])
    };
    status(
        if pending { "clock" } else { "tick-02" },
        text,
        if pending {
            palette().warning
        } else {
            palette().success
        },
    )
}
