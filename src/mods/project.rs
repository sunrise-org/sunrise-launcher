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
                .rounded(px(tokens::R_CONTROL))
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
pub(super) fn back(label: &'static str) -> Stateful<Div> {
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
        .child(label)
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
        let back_label = match back_to {
            Page::Installed => tr("Installed"),
            _ => tr("Discover"),
        };
        let mut page = div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(14.))
            .p(px(24.))
            .text_color(c(palette().text))
            .child(
                back(back_label).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.ui.page = back_to;
                    cx.notify();
                })),
            )
            .children(self.strip(cx));
        let loaded = self.ui.mods.open.as_ref().map(|o| o.page.clone());
        let p = match loaded {
            None => return page.child(muted(tr("No mod is open."))).into_any_element(),
            Some(None) => return page.child(muted(tr("Loading…"))).into_any_element(),
            Some(Some(Err(e))) => {
                return page
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(tr("This mod can't be shown")),
                    )
                    .child(muted(e))
                    .into_any_element()
            }
            Some(Some(Ok(p))) => p,
        };

        let installed_here = self
            .ui
            .mods
            .snapshot
            .as_ref()
            .and_then(|s| s.entries.iter().find(|e| e.project == p.id))
            .map(|e| e.number.clone());
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
            .child(tile(picture, &p.name, 64.))
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
                            .child(p.name.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_x(px(8.))
                            .gap_y(px(4.))
                            .child(muted(credit))
                            .child(tag(p.make.label()))
                            .children(p.categories.first().map(|k| {
                                div()
                                    .flex_none()
                                    .px(px(6.))
                                    .rounded(px(tokens::R_SMALL))
                                    .border_1()
                                    .border_color(c(palette().line))
                                    .text_size(px(tokens::CAPTION))
                                    .text_color(c(palette().muted))
                                    .child(sunrise_launcher::browser::details::category_label(k))
                            }))
                            .child(
                                div()
                                    .flex_none()
                                    .px(px(6.))
                                    .rounded(px(tokens::R_SMALL))
                                    .border_1()
                                    .border_color(c(palette().line))
                                    .text_size(px(tokens::CAPTION))
                                    .text_color(c(palette().muted))
                                    .child(p.side),
                            ),
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
                        .font_family(crate::theme::MONO)
                        .child(v.number.clone()),
                )
                .child(
                    div()
                        .w(px(90.))
                        .text_color(c(palette().muted))
                        .child(v.published.clone()),
                )
                .child(match (&v.fit.reason(), installed, will) {
                    (_, true, _) => div().text_color(c(palette().success)).child(tr("Added")),
                    (None, _, true) => div()
                        .text_color(c(palette().success))
                        .child(tr("Fits your game · what Add installs")),
                    (None, _, false) => div()
                        .text_color(c(palette().success))
                        .child(tr("Fits your game")),
                    (Some(why), _, _) => div().text_color(c(palette().muted)).child(why.clone()),
                })
        }));

        // The right column: the action and what it changes, then the facts.
        let native_allowed = self.settings.native_mods_allowed_at.is_some();
        let busy = self.mods_busy();
        let native = p.make == Make::Native;
        let action: AnyElement = match (&installed_here, &p.fit, &p.will_install) {
            (Some(n), ..) => div()
                .h(px(tokens::ACTION))
                .flex()
                .items_center()
                .justify_center()
                .text_color(c(palette().success))
                .child(trf("Added · version {version}", &[("version", n)]))
                .into_any_element(),
            (None, fit, Some(will)) if fit.fits() => {
                let label = match (native, native_allowed) {
                    (true, false) => tr("Add to my game…"),
                    _ => tr("Add to my game"),
                };
                let b = primary("alt-project-add", label)
                    .w_full()
                    .h(px(tokens::ACTION));
                if busy {
                    dim(b).into_any_element()
                } else {
                    let id = p.id.clone();
                    let _ = will;
                    b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.add(id.clone());
                        cx.notify();
                    }))
                    .into_any_element()
                }
            }
            (None, fit, _) => div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(dim(primary("alt-project-add", tr("Add to my game"))
                    .w_full()
                    .h(px(tokens::ACTION))))
                .child(muted(fit.reason().unwrap_or_else(|| {
                    tr("No version fits this build.").to_string()
                })))
                .into_any_element(),
        };
        let facts_line = p.will_install.as_ref().map(|w| {
            let code = match p.make {
                Make::Native => tr("native code, built on this computer after you allow it"),
                Make::Content => tr("content only, no native code"),
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
        let changes = |title: String, note: String, glyph: &'static str| {
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .py(px(10.))
                .child(div().w(px(22.)).text_color(c(palette().muted)).child(glyph))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_size(px(tokens::LABEL))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(title),
                        )
                        .child(muted(note)),
                )
        };
        let mut what = div().flex().flex_col().child(
            div()
                .text_size(px(tokens::CAPTION))
                .text_color(c(palette().muted))
                .pb(px(6.))
                .child(tr("WHAT THIS CHANGES")),
        );
        if let Some(w) = &p.will_install {
            what = what.child(changes(
                p.name.clone(),
                trf("Added · {version}", &[("version", &w.number)]),
                "+",
            ));
        }
        for (name, version) in &p.requires {
            what = what.child(changes(
                name.clone(),
                trf("Comes with it · {version}", &[("version", version)]),
                "•",
            ));
        }
        let right = div()
            .w(px(360.))
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(
                card()
                    .p(px(16.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(action)
                    .children(facts_line.map(|l| {
                        div()
                            .text_size(px(tokens::SMALL))
                            .text_color(c(palette().muted))
                            .child(l)
                    }))
                    .child(what)
                    .children(p.trust.iter().map(|line| muted(line.clone()))),
            )
            .child({
                let row = about_row;
                let source = p.source.clone();
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
                    .child(row(tr("Downloads"), p.downloads.to_string()))
                    .child(row(tr("Updated"), p.updated.clone()))
                    .child(row(tr("Licence"), p.license.clone()))
                    .children((source.starts_with("https://")).then(|| {
                        about_link("alt-project-source", tr("Source code")).on_click(cx.listener(
                            move |_, _: &ClickEvent, _, cx| Launcher::open_url(&source, cx),
                        ))
                    }))
            });

        let left = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(head)
            .children(gallery)
            .child(about)
            .child(
                div()
                    .text_size(px(tokens::HEADING))
                    .font_weight(FontWeight::BOLD)
                    .child(tr("Versions")),
            )
            .child(versions);
        page = page.child(
            div()
                .id("alt-project-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(&crate::frame::scrolls("project"))
                .child(
                    div()
                        .flex()
                        .gap(px(24.))
                        .items_start()
                        .child(left)
                        .child(right),
                ),
        );
        page.into_any_element()
    }
}
