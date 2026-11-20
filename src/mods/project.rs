//! A mod's page (mock 04): one button, and a plain list of what adding it changes.
use super::model::{self, Make};
use super::ui::*;
use super::*;
use crate::paint::palette;
use sunrise_launcher::browser::images;

/// Show every registry screenshot. Pack feeds can list many more: the full-size viewer steps
/// through those too, without fetching all of a large pack gallery on opening its page.
impl Launcher {
    pub(super) fn gallery(&mut self, urls: Vec<String>, cx: &mut Context<Self>) -> Option<Div> {
        if urls.is_empty() {
            return None;
        }
        let urls: Rc<[String]> = urls.into();
        const PREVIEWS: usize = sunrise_launcher_smod::index::page::MAX_SCREENSHOTS;
        let count = urls.len().min(PREVIEWS);
        let columns = if self.window_width < 950. { 3 } else { 4 };
        let height = if count == 1 { 220. } else { 104. };
        // A tile's tooltip would float over its own picture once that is open.
        let viewing = self.ui.mods.viewer.is_some();
        let mut tiles = urls.iter().take(PREVIEWS).enumerate().map(|(i, url)| {
            let photo = self.mods_image(url, images::GALLERY);
            // No wider than the picture's own shape at the row's height: a wide window would
            // otherwise blow it up and crop it.
            let ratio = photo
                .as_deref()
                .and_then(super::viewer::picture_size)
                .map_or(16. / 9., |(w, h)| w.max(1) as f32 / h.max(1) as f32);
            let all = urls.clone();
            let label = trf(
                "Picture {number} of {total}",
                &[
                    ("number", &(i + 1).to_string()),
                    ("total", &urls.len().to_string()),
                ],
            );
            div()
                .id(ElementId::named_usize("gallery-picture", i))
                .reach_edge()
                .relative()
                .flex_1()
                .min_h_0()
                .w_full()
                .max_w(px(height * ratio))
                .cursor_pointer()
                .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                .border_1()
                .border_color(c(palette().divider))
                .bg(palette().card())
                .overflow_hidden()
                .when(!viewing, |d| d.tooltip(crate::glass_ui::tip(label)))
                .children(photo.map(|path| {
                    gpui::img(path)
                        .size_full()
                        .object_fit(gpui::ObjectFit::Cover)
                }))
                .when(i + 1 == PREVIEWS && urls.len() > PREVIEWS, |d| {
                    d.child(
                        div()
                            .absolute()
                            .right_0()
                            .bottom_0()
                            .px(px(6.))
                            .bg(c(palette().raised))
                            .child(format!("+{}", urls.len() - PREVIEWS)),
                    )
                })
                .track_scroll(&super::viewer::tile(i))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.viewer_open(all.clone(), i);
                    cx.notify();
                }))
        });
        let mut grid = div().flex().flex_col().flex_none().gap(px(6.));
        for _ in 0..count.div_ceil(columns) {
            grid = grid.child(
                div()
                    .flex()
                    .gap(px(6.))
                    .h(px(height))
                    .children(tiles.by_ref().take(columns)),
            );
        }
        Some(grid)
    }
}

/// A page's description, written in Markdown, in this look's colours.
pub(super) fn prose(theme: crate::theme::Theme, text: &str) -> AnyElement {
    let mut t = field_theme(theme);
    t.ink_soft = c(palette().text);
    t.plate_soft = c(palette().card_color);
    t.rule = c(palette().divider);
    crate::markdown::render(text, &t)
}

/// "‹ Discover": the way back from a page.
pub(super) fn back(label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id("alt-project-back")
        .reach()
        .self_start()
        .flex()
        .items_center()
        .gap(px(8.))
        .text_size(px(tokens::LABEL))
        .text_color(c(palette().muted))
        .cursor_pointer()
        .child("‹")
        .child(label.into())
}

/// One line of a page's About card.
pub(super) fn about_row(key: &'static str, value: String) -> Div {
    div()
        .flex()
        .justify_between()
        .gap(px(12.))
        .py(px(5.))
        .text_size(px(tokens::LABEL))
        .child(div().flex_none().text_color(c(palette().muted)).child(key))
        .child(div().min_w_0().truncate().child(value))
}

/// The line of an About card that opens a web page.
pub(super) fn about_link(id: &'static str, key: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .reach()
        .flex()
        .justify_between()
        .py(px(5.))
        .text_size(px(tokens::LABEL))
        .cursor_pointer()
        .child(div().text_color(c(palette().muted)).child(key))
        .child(
            div()
                .text_color(c(palette().accent_ink))
                .child(tr("Open ↗")),
        )
}

impl Launcher {
    pub(super) fn project(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let back_to = self
            .ui
            .mods
            .open
            .as_ref()
            .map_or(Page::Discover, |o| o.from);
        let parent = self.ui.mods.open.as_ref().and_then(|o| o.parent.clone());
        let parent_name = parent.as_ref().and_then(|id| match &self.ui.mods.catalog {
            Catalog::Ready(cards, _) => cards.iter().find(|c| c.id == *id).map(|c| c.name.clone()),
            _ => None,
        });
        let back_label = match (back_to, &parent_name) {
            (_, Some(name)) => name.clone(),
            (Page::Installed, _) => tr("Installed").to_string(),
            _ => tr("Discover").to_string(),
        };
        // The back link scrolls away with the page, above the side panel.
        let header = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(
                back(back_label).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    match &parent {
                        Some(collection) => this.mods_load_page(collection.clone(), back_to, None),
                        None => this.ui.page = back_to,
                    }
                    cx.notify();
                })),
            )
            .children(self.strip(cx));
        let page = div()
            .w_full()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(14.))
            .p(px(24.))
            .text_color(c(palette().text));
        if self.ui.mods.open.as_ref().is_some_and(|o| o.collection) {
            return self.collection_page(header, page, cx);
        }
        let loaded = self.ui.mods.open.as_ref().map(|o| o.page.clone());
        let p = match loaded {
            None => {
                return page
                    .child(header)
                    .child(muted(tr("No mod is open.")))
                    .into_any_element()
            }
            Some(None) => {
                return page
                    .child(header)
                    .child(muted(tr("Loading…")))
                    .into_any_element()
            }
            Some(Some(Err(e))) => {
                return page
                    .child(header)
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(tr("This mod can't be shown")),
                    )
                    .child(note(e))
                    .into_any_element()
            }
            Some(Some(Ok(p))) => p,
        };

        let installed_here = p.installed.clone().or_else(|| {
            self.ui.mods.snapshot.as_ref().and_then(|s| {
                s.entries
                    .iter()
                    .find(|e| e.project == p.id)
                    .map(|e| e.number.clone())
            })
        });
        let picture = p
            .icon
            .clone()
            .and_then(|u| self.mods_image(&u, images::ICON));
        let credit = match p.authors.is_empty() {
            true => String::new(),
            false => trf("by {authors}", &[("authors", &p.authors.join(", "))]),
        };
        let head = div()
            .flex()
            .items_center()
            .gap(px(16.))
            .child(tile(picture, &p.id, &p.name, 64.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        div()
                            .text_size(px(tokens::DISPLAY))
                            .font_weight(FontWeight::BOLD)
                            .child(crate::selectable::selectable(p.name.clone())),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_x(px(8.))
                            .gap_y(px(4.))
                            .child(note(credit))
                            .children(p.make.map(|make| tag(make.label())))
                            .children(p.categories.first().map(|k| {
                                div()
                                    .flex_none()
                                    .px(px(6.))
                                    .rounded(crate::tokens::r(crate::tokens::R_SMALL))
                                    .border_1()
                                    .border_color(c(palette().line))
                                    .text_size(px(tokens::CAPTION))
                                    .text_color(c(palette().muted))
                                    .child(sunrise_launcher::browser::details::category_label(k))
                            }))
                            .children(p.about.as_ref().map(|about| {
                                div()
                                    .flex_none()
                                    .px(px(6.))
                                    .rounded(crate::tokens::r(crate::tokens::R_SMALL))
                                    .border_1()
                                    .border_color(c(palette().line))
                                    .text_size(px(tokens::CAPTION))
                                    .text_color(c(palette().muted))
                                    .child(about.side)
                            })),
                    ),
            );

        let gallery = self.gallery(p.screenshots.clone(), cx);
        let about = prose(self.theme, &p.description);

        let versions = card().children(p.versions.iter().enumerate().map(|(i, v)| {
            let installed = installed_here.as_deref() == Some(v.number.as_str());
            let will = p
                .will_install
                .as_ref()
                .is_some_and(|w| w.number == v.number);
            div()
                .flex()
                .items_center()
                .gap(px(16.))
                .px(px(14.))
                .py(px(10.))
                .when(i > 0, |d| d.border_t_1().border_color(c(palette().divider)))
                .text_size(px(tokens::LABEL))
                .child(
                    div()
                        .w(px(70.))
                        .flex_none()
                        .truncate()
                        .font_family(crate::theme::MONO)
                        .child(v.number.clone()),
                )
                .child(
                    div()
                        .w(px(90.))
                        .flex_none()
                        .text_color(c(palette().muted))
                        .child(v.published.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(match (&v.fit.reason(), installed, will) {
                            (_, true, _) => div()
                                .text_color(c(palette().success))
                                .child(tr("Installed")),
                            (None, _, true) => div()
                                .text_color(c(palette().success))
                                .child(tr("Fits your game · what Install adds")),
                            (None, _, false) => div()
                                .text_color(c(palette().success))
                                .child(tr("Fits your game")),
                            (Some(why), _, _) => {
                                div().text_color(c(palette().muted)).child(why.clone())
                            }
                        }),
                )
        }));

        // The right column: the action and what it changes, then the facts.
        let busy = self.mods_busy();
        let requires = p.requires.clone();
        // An update is offered here too: the version Add would install, over the one installed.
        let update = match (&installed_here, &p.will_install) {
            (Some(have), Some(will)) if p.fit.fits() && *have != will.number => {
                Some((have.clone(), will.number.clone()))
            }
            _ => None,
        };
        let action: AnyElement = match (&installed_here, &p.fit, &p.will_install) {
            (Some(_), ..) if update.is_some() => {
                let (from, to) = update.clone().unwrap_or_default();
                let b = primary(
                    "alt-project-update",
                    trf("Update {from} → {to}", &[("from", &from), ("to", &to)]),
                )
                .w_full()
                .h(px(tokens::ACTION));
                if busy {
                    dim(b).into_any_element()
                } else {
                    let id = p.id.clone();
                    b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.update_mod(&id);
                        cx.notify();
                    }))
                    .into_any_element()
                }
            }
            (Some(_), ..) => div()
                .h(px(tokens::ACTION))
                .flex()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                .border_1()
                .border_color(c(palette().divider))
                .text_color(c(palette().success))
                .child(blocks::icon("check", 14., c(palette().success)))
                .child(tr("Installed"))
                .into_any_element(),
            (None, fit, Some(_)) if fit.fits() => {
                let profile = (self.instances.list.len() > 1)
                    .then(|| self.selected_instance().map(|i| i.name.clone()))
                    .flatten();
                let label = match profile {
                    Some(name) => trf("Install to {profile}", &[("profile", &name)]),
                    None => tr("Install").to_string(),
                };
                let b = primary("alt-project-add", label)
                    .w_full()
                    .h(px(tokens::ACTION));
                if busy {
                    dim(b).into_any_element()
                } else {
                    let id = p.id.clone();
                    let name = p.name.clone();
                    b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.add(id.clone(), name.clone());
                        cx.notify();
                    }))
                    .into_any_element()
                }
            }
            (None, fit, _) => div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(dim(primary("alt-project-add", tr("Install"))
                    .w_full()
                    .h(px(tokens::ACTION))))
                .child(note(fit.reason().unwrap_or_else(|| {
                    tr("No version fits this build.").to_string()
                })))
                .into_any_element(),
        };
        // The version this panel is about: what would be installed, else what is.
        let shown_version = p
            .will_install
            .as_ref()
            .map(|w| w.number.clone())
            .or_else(|| installed_here.clone());
        // ---- the facts, as chips: a glyph, a few words, the whole sentence on hover and
        // under Details.
        let fact = |id: &'static str, icon: &'static str, label: String, tip: String, tone: u32| {
            div()
                .id(id)
                .flex_none()
                .max_w_full()
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(9.))
                .h(px(28.))
                .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                .border_1()
                .border_color(c(palette().divider))
                .bg(c(palette().field))
                .text_size(px(tokens::SMALL))
                .text_color(c(palette().text))
                .tooltip(crate::glass_ui::tip(tip))
                .child(crate::glass_ui::icon(icon, 14., c(tone)))
                .child(div().min_w_0().truncate().child(label))
        };
        let mut facts = div().flex().flex_wrap().gap(px(6.));
        if let Some(w) = &p.will_install {
            facts = facts.child(fact(
                "alt-fact-size",
                "download-04",
                model::size(w.size),
                trf("{size} to download", &[("size", &model::size(w.size))]),
                palette().muted,
            ));
        }
        facts = facts.children(p.make.map(|make| match make {
            Make::Native => fact(
                "alt-fact-code",
                "shield",
                tr("Native code").into(),
                if p.about.is_some() {
                    tr("Native code: it is built into the game on this computer and runs with full access to your computer. Listed after a person reviewed its entry and file.")
                } else {
                    tr("native code, built on this computer after you allow it")
                }.into(),
                palette().accent_ink,
            ),
            Make::Content => fact(
                "alt-fact-code",
                "package-01",
                tr("Content only").into(),
                tr("Content only: it adds no native code.").into(),
                palette().muted,
            ),
        }));
        if p.about.as_ref().is_some_and(|about| !about.signed) {
            facts = facts.child(fact(
                "alt-fact-signed",
                "information-circle",
                tr("Not signed by its author").into(),
                tr("Not signed by its author: Sunrise checks the download arrives complete and unchanged.").into(),
                palette().warning,
            ));
        }
        if !p.authors.is_empty() {
            let by = trf("By {authors}", &[("authors", &p.authors.join(", "))]);
            facts = facts.child(fact(
                "alt-fact-by",
                "user",
                by,
                p.trust.first().cloned().unwrap_or_default(),
                palette().muted,
            ));
        }
        let game_line = p.another_build.then(|| {
            div()
                .flex()
                .items_start()
                .gap(px(8.))
                .text_size(px(tokens::SMALL))
                .text_color(c(palette().muted))
                .child(crate::glass_ui::icon("clock", 14., c(palette().muted)))
                .child(div().flex_1().min_w_0().child(tr(
                    "Made for an older game version; it's checked before it runs.",
                )))
        });
        // ---- what the press changes, only when it is more than this one mod
        let change = |glyph: &'static str, title: String, detail: String| {
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .py(px(6.))
                .child(crate::glass_ui::icon(glyph, 14., c(palette().muted)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(tokens::LABEL))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(note(detail).flex_none())
        };
        let pending = installed_here.is_none() || update.is_some();
        let changes = (pending && !requires.is_empty()).then(|| {
            let mut list = div().flex().flex_col().child(
                div()
                    .text_size(px(tokens::CAPTION))
                    .text_color(c(palette().muted))
                    .pb(px(2.))
                    .child(trf(
                        "{count} CHANGES",
                        &[("count", &(requires.len() + 1).to_string())],
                    )),
            );
            list = list.child(match &update {
                Some((from, to)) => change(
                    "refresh",
                    p.name.clone(),
                    trf("{from} → {to}", &[("from", from), ("to", to)]),
                ),
                None => change(
                    "plus-sign",
                    p.name.clone(),
                    shown_version.clone().unwrap_or_default(),
                ),
            });
            for (name, version) in &requires {
                list = list.child(change(
                    "plus-sign",
                    name.clone(),
                    trf("comes with it · {version}", &[("version", version)]),
                ));
            }
            list
        });
        // ---- everything else, as whole sentences, behind Details
        let open = self.ui.mods.facts_open;
        let details_toggle = div()
            .id("alt-project-details")
            .reach()
            .flex()
            .items_center()
            .gap(px(6.))
            .text_size(px(tokens::SMALL))
            .text_color(c(palette().muted))
            .cursor_pointer()
            .child(blocks::icon(
                if open {
                    "chevron-down"
                } else {
                    "chevron-right"
                },
                12.,
                c(palette().muted),
            ))
            .child(tr("Details"))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.ui.mods.facts_open = !this.ui.mods.facts_open;
                cx.notify();
            }));
        let details = open.then(|| {
            let facts_line = p.will_install.as_ref().map(|w| {
                let code = match p.make {
                    Some(Make::Native) => {
                        tr("native code, built on this computer after you allow it")
                    }
                    _ => tr("content only, no native code"),
                };
                trf(
                    "Version {version} · {size} to download · {code}",
                    &[
                        ("version", &w.number),
                        ("size", &model::size(w.size)),
                        ("code", code),
                    ],
                )
            });
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .children(facts_line.map(note))
                .children(p.trust.iter().map(|line| note(line.clone())))
        });
        let panel_head = div()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(tile(
                p.icon
                    .clone()
                    .and_then(|u| self.mods_image(&u, images::ROW)),
                &p.id,
                &p.name,
                40.,
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .truncate()
                            .font_weight(FontWeight::BOLD)
                            .child(p.name.clone()),
                    )
                    .children(shown_version.clone().map(|v| {
                        div()
                            .text_size(px(tokens::SMALL))
                            .font_family(crate::theme::MONO)
                            .text_color(c(palette().muted))
                            .child(v)
                    })),
            );
        let source_button = p.source.map(|source| {
            let (icon, label, url) = match source {
                model::SourceLink::GitHub(url) => ("github", tr("Open on GitHub"), url),
                model::SourceLink::Other(url) => ("link-01", tr("Open source page"), url),
            };
            crate::frame::button("alt-project-source", "")
                .child(crate::glass_ui::icon(icon, 15., c(palette().text)))
                .child(div().pl(px(8.)).child(label))
                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| Launcher::open_url(&url, cx)))
        });
        let scroll = crate::frame::scrolls("project");
        let right = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(
                card()
                    .p(px(16.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(panel_head)
                    .child(action)
                    .when(p.make.is_some(), |d| d.child(facts))
                    .children(game_line)
                    .children(changes)
                    .when(p.about.is_some() || source_button.is_some(), |d| {
                        d.child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap(px(8.))
                                .when(p.about.is_some(), |d| d.child(details_toggle))
                                .children(source_button.map(|button| button.ml_auto())),
                        )
                    })
                    .children(details),
            )
            .children(p.about.as_ref().map(|about| {
                card()
                    .p(px(16.))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(px(tokens::CAPTION))
                            .text_color(c(palette().muted))
                            .pb(px(6.))
                            .child(tr("ABOUT")),
                    )
                    .child(about_row(tr("Downloads"), about.downloads.to_string()))
                    .child(about_row(tr("Updated"), about.updated.clone()))
                    .child(about_row(tr("Licence"), about.license.clone()))
            }));

        // A narrow window has one column: the action stays near the top. Beside the column, the
        // panel stays in view while the page scrolls.
        let (above, beside) = match self.content_width() < 860. {
            true => (Some(right), None),
            false => (
                None,
                Some(
                    right
                        .relative()
                        .top(px(self.mods_sticky.shift(&scroll, 1, 1)))
                        .w(px(360.))
                        .flex_none(),
                ),
            ),
        };
        let left = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(head)
            .children(above)
            .children(gallery)
            .when(!p.description.is_empty(), |d| d.child(about))
            .when(!p.versions.is_empty(), |d| {
                d.child(
                    div()
                        .text_size(px(tokens::HEADING))
                        .font_weight(FontWeight::BOLD)
                        .child(tr("Versions")),
                )
                .child(versions)
            });
        page.child(
            div()
                .id("alt-project-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(&scroll)
                .flex()
                .flex_col()
                .gap(px(14.))
                .child(header)
                .child(
                    div()
                        .id("alt-project-columns")
                        .track_scroll(self.mods_sticky.row())
                        .flex()
                        .gap(px(24.))
                        .items_start()
                        .child(left)
                        .children(beside),
                ),
        )
        .into_any_element()
    }
}
