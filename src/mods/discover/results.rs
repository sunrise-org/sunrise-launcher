//! Discover's results: the collections, the grid of cards and the Show more paging.
use super::*;

impl Launcher {
    /// What a working card says where Install was, in a label about Install's size: the strip
    /// above the page says each step, and the card keeps its shape.
    pub(in crate::mods) fn card_working_label(&self, id: &str) -> Option<&'static str> {
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
    pub(in crate::mods) fn with_collections(
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
                None,
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

    pub(in crate::mods) fn discover_grid(&self) -> Div {
        div().flex().flex_col().gap(px(10.))
    }

    pub(in crate::mods) fn discover_results(
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
}
