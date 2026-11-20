//! Mods › Discover (mock 03): the registry's mods, filtered, sorted, one click to Add.
use super::install_state::{self, InstallState};
use super::model::{self, Fit, Make, Sort};
use super::ui::*;
use super::*;
use crate::paint::palette;
use gpui::deferred;
use sunrise_launcher::browser::details::category_label;
use sunrise_launcher::browser::images;

/// How many cards are drawn before "Show more".
const PAGE: usize = 48;

/// A dropdown line: its label, whether it is the current choice, and what picking it does.
type MenuOption = (SharedString, bool, Box<dyn Fn(&mut Launcher)>);

impl Launcher {
    pub(super) fn discover(
        &mut self,
        window: &mut Window,
        focus: &FocusHandle,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let header = self.mods_header(false, focus, window, cx);
        let mut page = div()
            .w_full()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(14.))
            .p(px(24.))
            .text_color(c(palette().text))
            .child(header)
            .children(self.strip(cx));
        let (cards, categories): (Vec<Card>, Vec<String>) = match &self.ui.mods.catalog {
            Catalog::Ready(cards, cats) => (cards.clone(), cats.clone()),
            Catalog::Failed(e) => {
                let e = e.clone();
                return page
                    .child(
                        card()
                            .p(px(20.))
                            .flex()
                            .flex_col()
                            .gap(px(10.))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(tr("The Sunrise registry can't be reached")),
                            )
                            .child(muted(e))
                            .child(
                                crate::frame::button("alt-mods-retry", tr("Try again")).on_click(
                                    cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.ui.mods.catalog = Catalog::Idle;
                                        cx.notify();
                                    }),
                                ),
                            ),
                    )
                    .into_any_element();
            }
            _ => {
                return page
                    .child(muted(tr("Loading the registry…")))
                    .into_any_element();
            }
        };

        let f = self.ui.mods.filters.clone();
        let shown = model::discover(&cards, &f);
        let total = shown.len();
        let limit = self.ui.mods.limit.max(PAGE);
        let added: HashSet<String> = self
            .ui
            .mods
            .snapshot
            .as_ref()
            .map(|s| s.entries.iter().map(|e| e.project.clone()).collect())
            .unwrap_or_default();
        let index = sunrise::known_index();
        let game_running = self.game_running();
        let busy = self.mods.busy.as_ref().map(|b| (b.0.clone(), b.2));
        let enabled = self
            .ui
            .mods
            .snapshot
            .as_ref()
            .map(|s| s.enabled.clone())
            .unwrap_or_default();
        let profile = (self.instances.list.len() > 1)
            .then(|| self.selected_instance().map(|i| i.name.clone()))
            .flatten();
        let run = self.ui.mods.build.as_ref().filter(|run| run.running());
        let build_fraction = run.map_or(0., |run| run.fraction());
        let tools = run.and_then(|run| run.getting_tools());

        // The filters: category and kind as dropdowns, fits my game, and the order.
        let fits = div()
            .id("alt-fits")
            .reach()
            .flex()
            .items_center()
            .gap(px(8.))
            .cursor_pointer()
            .text_size(px(tokens::LABEL))
            // The line is the Tab stop, not the switch in it.
            .child(ui::switch("alt-fits-switch", f.fits_only).tab_stop(false))
            .child(tr("Fits my game"))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                let f = &mut this.ui.mods.filters;
                f.fits_only = !f.fits_only;
                cx.notify();
            }));
        let open = self.ui.mods.menu;
        let (category_open, make_open, sort_open) = (
            open == Some(Menu::Category),
            open == Some(Menu::Make),
            open == Some(Menu::Sort),
        );
        let category_now = f.category.as_deref().map_or_else(
            || tr("All categories").to_string(),
            |k| category_label(k).to_string(),
        );
        let make_label = f.make.map_or(tr("Any kind"), Make::label);
        // A menu opens under its dropdown, from the side the dropdown sits on.
        let popover = |options: Vec<MenuOption>, from_left: bool, cx: &mut Context<Self>| {
            deferred(
                div()
                    .absolute()
                    .top(px(38.))
                    .when(from_left, |d| d.left(px(0.)))
                    .when(!from_left, |d| d.right(px(0.)))
                    .w(px(200.))
                    .p(px(6.))
                    .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                    .border_1()
                    .border_color(c(palette().line))
                    .bg(c(palette().raised))
                    .shadow_lg()
                    .occlude()
                    .flex()
                    .flex_col()
                    .children(
                        options
                            .into_iter()
                            .enumerate()
                            .map(|(i, (label, on, pick))| {
                                div()
                                    .id(("alt-menu-option", i))
                                    .reach()
                                    .px(px(10.))
                                    .py(px(7.))
                                    .rounded(crate::tokens::r(crate::tokens::R_SMALL))
                                    .text_size(px(tokens::LABEL))
                                    .cursor_pointer()
                                    .hover(|s| s.bg(c(palette().hover)))
                                    .when(on, |d| d.font_weight(FontWeight::SEMIBOLD))
                                    .child(label)
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        pick(this);
                                        this.ui.mods.menu = None;
                                        cx.notify();
                                    }))
                            }),
                    ),
            )
            .with_priority(10)
        };
        let dropdown = |id: &'static str, label: &str, which: Menu, cx: &mut Context<Self>| {
            div()
                .id(id)
                .reach()
                .flex()
                .items_center()
                .gap(px(6.))
                .h(px(tokens::CONTROL))
                .cursor_pointer()
                .text_size(px(tokens::LABEL))
                .child(label.to_owned())
                .child(blocks::icon("chevron-down", 13., c(palette().muted)))
                // A press here is the dropdown's: the page closing any open menu on a press would
                // shut this one just before its click opened it again.
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    let same = this.ui.mods.menu == Some(which);
                    this.ui.mods.menu = (!same).then_some(which);
                    cx.notify();
                }))
        };
        let category_options: Vec<MenuOption> = std::iter::once((
            tr("All categories").into(),
            f.category.is_none(),
            Box::new(|this: &mut Launcher| this.ui.mods.filters.category = None)
                as Box<dyn Fn(&mut Launcher)>,
        ))
        .chain(categories.iter().map(|k| {
            let key = k.clone();
            (
                category_label(k).into(),
                f.category.as_ref() == Some(k),
                Box::new(move |this: &mut Launcher| {
                    this.ui.mods.filters.category = Some(key.clone())
                }) as Box<dyn Fn(&mut Launcher)>,
            )
        }))
        .collect();
        let make_options: Vec<MenuOption> = std::iter::once((
            tr("Any kind").into(),
            f.make.is_none(),
            Box::new(|this: &mut Launcher| this.ui.mods.filters.make = None)
                as Box<dyn Fn(&mut Launcher)>,
        ))
        .chain(Make::ALL.into_iter().map(|m| {
            (
                m.label().into(),
                f.make == Some(m),
                Box::new(move |this: &mut Launcher| this.ui.mods.filters.make = Some(m))
                    as Box<dyn Fn(&mut Launcher)>,
            )
        }))
        .collect();
        let sort_options: Vec<MenuOption> = Sort::ALL
            .into_iter()
            .map(|s| {
                (
                    s.label().into(),
                    f.sort == s,
                    Box::new(move |this: &mut Launcher| this.ui.mods.filters.sort = s)
                        as Box<dyn Fn(&mut Launcher)>,
                )
            })
            .collect();
        // What narrows the list on the left, what orders it on the right.
        let filters = div()
            .flex()
            .items_center()
            .gap(px(18.))
            .child(
                div()
                    .relative()
                    .child(dropdown(
                        "alt-category-menu",
                        &category_now,
                        Menu::Category,
                        cx,
                    ))
                    .children(category_open.then(|| popover(category_options, true, cx))),
            )
            .child(
                div()
                    .relative()
                    .child(dropdown("alt-make-menu", make_label, Menu::Make, cx))
                    .children(make_open.then(|| popover(make_options, true, cx))),
            )
            .child(fits);
        let order = div()
            .relative()
            .child(dropdown("alt-sort-menu", f.sort.label(), Menu::Sort, cx))
            .children(sort_open.then(|| popover(sort_options, false, cx)));
        page = page.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(18.))
                .child(filters)
                .child(order),
        );

        // The cards.
        let mut grid = div()
            .grid()
            .grid_cols(columns(self.content_width()))
            .gap(px(12.));
        for (n, &i) in shown.iter().take(limit).enumerate() {
            let card_data = &cards[i];
            let id = card_data.id.clone();
            let picture = card_data
                .icon
                .clone()
                .and_then(|url| self.mods_image(&url, images::ICON));
            let is_added = match card_data.members {
                Some(_) => index
                    .as_ref()
                    .and_then(|i| i.collection(&id))
                    .is_some_and(|c| model::collection::installed(c, &cards, &added)),
                None => added.contains(&id),
            };
            let state = install_state::of(&id, self.mods.waiting(&id), &[], game_running);
            let stage = install_state::stage(
                &id,
                busy.as_ref().map(|(for_id, f)| (for_id.as_str(), *f)),
                self.native_step.is_some()
                    && self.native_for.as_deref()
                        == self.selected_instance().map(|i| i.id.as_str())
                    && self.native_built_mods.contains(&id),
                tools,
                build_fraction,
            );
            // An unknown fit says so in the place of Install, not as a note under the card.
            let note = card_data
                .fit
                .reason()
                .filter(|_| !is_added && card_data.fit != Fit::Unknown);
            let install_label = match &profile {
                Some(name) => trf("Install to {profile}", &[("profile", name)]),
                None => tr("Install").to_string(),
            };
            // A control too long for the right of the facts (the build tools' size, the game
            // running) takes a full-width row under them.
            let wide = matches!(&stage, Some(install_state::Stage::BuildTools(_)))
                || state == Some(InstallState::CloseGame);
            let control = if card_data.members.is_some() && !is_added {
                let b = crate::frame::button(("alt-add", n), tr("Choose mods"));
                let id = id.clone();
                Some(
                    b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.mods_show_project(id.clone());
                        cx.notify();
                    }))
                    .into_any_element(),
                )
            } else if let Some(stage) = &stage {
                // The install in place: the step in words over a thin bar, nothing else opens.
                let b = crate::frame::button(("alt-add", n), stage.label())
                    .relative()
                    .overflow_hidden()
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .right_0()
                            .bottom_0()
                            .child(crate::build::progress_bar(stage.fraction())),
                    );
                Some(dim(if wide { b.w_full() } else { b }).into_any_element())
            } else if is_added && !enabled.contains(&id) {
                let id = id.clone();
                Some(
                    crate::frame::button(("alt-turn-on", n), tr("Turn on"))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            this.turn_on_registry_mod(&id);
                            cx.notify();
                        }))
                        .into_any_element(),
                )
            } else if is_added {
                Some(added_mark())
            } else if card_data.fit == Fit::Unknown {
                Some(muted(tr("Checking…")).into_any_element())
            } else if state == Some(InstallState::BuildFailed) {
                Some(muted(tr("Couldn't build on your computer")).into_any_element())
            } else if card_data.fit == Fit::NeedsNewerSunrise && self.updates.sunrise.is_some() {
                Some(
                    crate::frame::button(("alt-card-update", n), tr("Update"))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            this.launch_act(&crate::launch_state::Act::Update);
                            cx.notify();
                        }))
                        .into_any_element(),
                )
            } else if note.is_some() {
                None
            } else {
                let label = match state {
                    Some(InstallState::Waiting) => tr("Waiting").to_string(),
                    Some(InstallState::CloseGame) => tr("Close the game to install").to_string(),
                    _ => install_label.clone(),
                };
                let b = crate::frame::button(("alt-add", n), label);
                let b = if wide { b.w_full() } else { b };
                Some(match state {
                    Some(InstallState::Waiting | InstallState::CloseGame) => {
                        dim(b).into_any_element()
                    }
                    _ => {
                        let id = id.clone();
                        let name = card_data.name.clone();
                        b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            this.add(id.clone(), name.clone());
                            cx.notify();
                        }))
                        .into_any_element()
                    }
                })
            };
            let failed_here = self
                .mods
                .card_error
                .as_ref()
                .filter(|e| e.id == id && stage.is_none());
            // What a build that stopped or a mod that didn't compile says, when no install error
            // is already on the card: its Details opens the same panel.
            let build_failure = self
                .native_error
                .as_ref()
                .filter(|_| {
                    failed_here.is_none()
                        && self.native_for.as_deref()
                            == self.selected_instance().map(|i| i.id.as_str())
                        && self.native_failed_mods.contains(&id)
                })
                .cloned();
            // A failed install keeps Install to try again, and a Details beside it.
            let control = match (failed_here, &build_failure) {
                (None, Some(line)) => {
                    let open_id = id.clone();
                    let line = line.clone();
                    let title = card_data.name.clone();
                    Some(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                crate::frame::button(("alt-card-details", n), tr("Details"))
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        cx.stop_propagation();
                                        this.mods.card_error = Some(crate::mods_page::CardError {
                                            id: open_id.clone(),
                                            title: title.clone(),
                                            line: line.clone(),
                                            details: std::fs::read_to_string(
                                                sunrise_launcher::native_mods::log_path(),
                                            )
                                            .unwrap_or_else(|_| line.clone()),
                                            built: true,
                                            open: true,
                                        });
                                        cx.notify();
                                    })),
                            )
                            .children(control)
                            .into_any_element(),
                    )
                }
                (Some(_), _) => Some(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            crate::frame::button(("alt-card-details", n), tr("Details")).on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| {
                                    cx.stop_propagation();
                                    if let Some(error) = &mut this.mods.card_error {
                                        error.open = !error.open;
                                    }
                                    cx.notify();
                                }),
                            ),
                        )
                        .children(control)
                        .into_any_element(),
                ),
                (None, None) => control,
            };
            let mut chips = match card_data.members {
                Some(count) => vec![
                    badge(tr("Collection")),
                    muted(trf("{count} mods", &[("count", &count.to_string())])),
                    muted(model::downloads(card_data.downloads)),
                ],
                None => {
                    let mut chips = vec![tag(card_data.make.label())];
                    if card_data.unsigned {
                        chips.push(tag(tr("Unsigned")));
                    }
                    chips.push(
                        muted(model::downloads(card_data.downloads))
                            .min_w_0()
                            .truncate(),
                    );
                    chips
                }
            };
            // The kind, whether it is signed and the downloads sit on one line; everything
            // else the card says is a line of its own under them.
            let mut lines: Vec<Div> = Vec::new();
            if let Some(line) = is_added
                .then(|| self.mods.also.get(&id))
                .flatten()
                .and_then(|names| install_state::also_line(names))
            {
                lines.push(muted(line).truncate());
            }
            if let Some(line) = is_added
                .then(|| self.mods.updated.get(&id))
                .flatten()
                .and_then(|moves| install_state::updated_line(moves))
            {
                lines.push(muted(line).truncate());
            }
            if let Some(error) = failed_here {
                lines.push(
                    div()
                        .text_size(px(tokens::SMALL))
                        .text_color(c(palette().danger))
                        .child(error.line.clone()),
                );
            }
            if card_data.library {
                chips.insert(1, badge(tr("Library")));
            }
            if let Some(name) = &card_data.part_of {
                lines.push(muted(trf("Part of {collection}", &[("collection", name)])));
            }
            if card_data.builds_locally && card_data.fit.fits() {
                lines.push(muted(tr("Builds on this computer")));
            }
            let facts = vec![div()
                .w_full()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .min_w_0()
                        .children(chips),
                )
                .children(lines)];
            let open = id.clone();
            grid = grid.child(
                browse_card(
                    ("alt-card", n),
                    picture,
                    &id,
                    &card_data.name,
                    &card_data.authors,
                    &card_data.summary,
                    facts,
                    control,
                    wide,
                    note,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.mods_show_project(open.clone());
                    cx.notify();
                })),
            );
        }
        let body = if shown.is_empty() {
            div().child(muted(if f.text.is_empty() && f.category.is_none() {
                tr("No mods for this build yet.")
            } else {
                tr("No mods match. Try another search, or turn off “Fits my game”.")
            }))
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(14.))
                .child(grid)
                .children((total > limit).then(|| {
                    crate::frame::button(
                        "alt-show-more",
                        trf(
                            "Show more ({count} more)",
                            &[("count", &(total - limit).to_string())],
                        ),
                    )
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.ui.mods.limit = this.ui.mods.limit.max(PAGE) + PAGE;
                        cx.notify();
                    }))
                }))
        };
        page.child(
            div()
                .id("alt-discover-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(&crate::frame::scrolls("discover"))
                .child(body),
        )
        .on_mouse_down(
            gpui::MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                if this.ui.mods.menu.take().is_some() {
                    cx.notify();
                }
            }),
        )
        .into_any_element()
    }
}
