//! A pack's page, opened from a card in Packs › Discover: the same page a mod has, for a
//! resource, shader or data pack. What it says comes from the pack's source (`Source::details`
//! and `Source::versions`), read in a worker; the action is the card's own (Add, or the world
//! picker for a data pack). Back returns to the grid as it was left.
use super::project::look::{action_column, card, head, muted, note, section};
use super::project::{about_row, back, project_frame, prose};
use super::ui::*;
use super::*;
use crate::paint::palette;
use crate::widgets::button::{primary, secondary};
use crate::widgets::chip::{self, tag};
use sunrise_launcher::browser::details::Details;
use sunrise_launcher::browser::{self, images, Kind, Project, Source as _, Version};

/// How many versions the page lists: the newest.
const VERSIONS: usize = 8;

pub(super) struct PackOpen {
    project: Project,
    page: Option<PackPage>,
}

pub(super) type PackPage = Result<Box<(Details, Vec<Version>)>, String>;

/// "2026-11-02T10:11:12Z" as "2026-11-02"; anything else as it is.
fn day(stamp: &str) -> String {
    match stamp.split_once('T') {
        Some((date, _)) => date.to_owned(),
        None => stamp.to_owned(),
    }
}

/// A source's Markdown without its pictures: badges and banners would only show as their
/// alternative text. A link around a picture goes with it.
fn without_pictures(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(at) = rest.find("![") {
        let close = rest[at..]
            .find("](")
            .and_then(|mid| rest[at + mid..].find(')').map(|end| at + mid + end + 1));
        let Some(close) = close else { break };
        out.push_str(&rest[..at]);
        rest = &rest[close..];
        // "[" + picture + "](link)": the link has nothing left to show.
        if out.ends_with('[') && rest.starts_with("](") {
            if let Some(end) = rest.find(')') {
                out.pop();
                rest = &rest[end + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// "1.21.4, 1.21.5, 1.21.6 and 9 more".
fn games(versions: &[String]) -> String {
    let newest: Vec<&str> = versions.iter().rev().take(3).map(String::as_str).collect();
    match versions.len().saturating_sub(newest.len()) {
        0 => newest.join(", "),
        more => trf(
            "{versions} and {count} more",
            &[
                ("versions", &newest.join(", ")),
                ("count", &more.to_string()),
            ],
        ),
    }
}

impl Launcher {
    /// Opens the page of a pack among Discover's results; its details are read in a worker.
    pub(super) fn pack_show(&mut self, source: SourceId, id: &str) {
        let Some(project) = self
            .mods_state
            .packs
            .results
            .iter()
            .find(|p| p.source == source && p.id == id)
            .cloned()
        else {
            return;
        };
        self.mods_state.menu = None;
        self.mods_state.packs.open = Some(PackOpen {
            project: project.clone(),
            page: None,
        });
        self.ui.page = Page::PackProject;
        let tx = self.mods_state.tx.clone();
        std::thread::spawn(move || {
            let s = browser::modrinth::Modrinth;
            let result = (|| Ok(Box::new((s.details(&project)?, s.versions(&project)?))))();
            let _ = tx.send(Event::PackPage((project.source, project.id), result));
        });
    }

    pub(super) fn pack_page_answer(&mut self, key: (SourceId, String), page: PackPage) {
        if let Some(open) = &mut self.mods_state.packs.open {
            if (open.project.source, &open.project.id) == (key.0, &key.1) {
                open.page = Some(page);
            }
        }
    }

    pub(super) fn pack_project(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let header = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(
                back(tr("Discover")).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.ui.page = Page::PackFind;
                    cx.notify();
                })),
            )
            .children(self.strip(cx));
        let Some(open) = &self.mods_state.packs.open else {
            return project_frame()
                .child(header)
                .child(muted(tr("No pack is open.")))
                .into_any_element();
        };
        let p = open.project.clone();
        let loaded = open.page.clone();
        let data = p.kind == Kind::DataPack;
        if data {
            self.data_list(cx);
        }
        let support = p.support(&self.mods_game());
        let picture = p
            .icon
            .clone()
            .and_then(|url| self.mods_image(&url, images::ICON));
        let details = match &loaded {
            Some(Ok(page)) => Some(&page.0),
            _ => None,
        };
        // A source may list one number once per loader: here it is one version.
        let mut versions: Vec<&Version> = match &loaded {
            Some(Ok(page)) => page.1.iter().collect(),
            _ => Vec::new(),
        };
        versions.dedup_by(|b, a| a.number == b.number);
        let authors = match details.map(|d| d.authors.as_slice()) {
            Some(list) if !list.is_empty() => list.join(", "),
            _ => p.author.clone(),
        };
        let by = (!authors.is_empty()).then(|| trf("by {authors}", &[("authors", &authors)]));
        let source_kind = match p.source {
            SourceId::Modrinth => chip::Kind::Modrinth,
            SourceId::Sunrise => chip::Kind::Sunrise,
        };
        let chips: Vec<AnyElement> = vec![
            tag(super::pack_discover::source_label(p.source), source_kind).into_any_element(),
            tag(support.label(), chip::Kind::Neutral).into_any_element(),
        ];
        let mut facts = vec![trf(
            "Downloads: {count}",
            &[("count", &p.downloads.to_string())],
        )];
        if let Some(d) = details.filter(|d| !d.updated.is_empty()) {
            facts.push(trf("Updated: {day}", &[("day", &day(&d.updated))]));
        }
        if let Some(newest) = versions.first() {
            facts.push(trf("Version {number}", &[("number", &newest.number)]));
        }
        let header_block = head(
            tile(picture, &p.id, &p.title, 64.).into_any_element(),
            &p.title,
            by,
            chips,
            Some(p.summary.clone()).filter(|s| !s.trim().is_empty()),
            facts,
        );

        // The action, as on the card.
        let added = !data
            && self.mods_state.snapshot.as_ref().is_some_and(|s| {
                s.entries
                    .iter()
                    .any(|e| e.source == p.source && e.project == p.id)
            });
        let action: AnyElement = if added {
            div()
                .h(px(tokens::ACTION))
                .flex()
                .items_center()
                .justify_center()
                .child(added_mark())
                .into_any_element()
        } else {
            let label = if data {
                tr("Add to worlds…")
            } else {
                tr("Add to my game")
            };
            let b = primary("alt-pack-page-add", label)
                .w_full()
                .h(px(tokens::ACTION));
            if !support.installable() || self.mods_state.busy.is_some() {
                dim(b).into_any_element()
            } else {
                let (source, id) = (p.source, p.id.clone());
                b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.pack_install(source, &id);
                    cx.notify();
                }))
                .into_any_element()
            }
        };
        let within = match self.mods_state.data.worlds_with(p.source, &p.id) {
            _ if !data => None,
            0 => None,
            1 => Some(tr("In 1 world of this profile.").to_string()),
            n => Some(trf(
                "In {count} worlds of this profile.",
                &[("count", &n.to_string())],
            )),
        };
        let link = p.page.clone();
        let mut action_col = action_column()
            .child(action)
            .children(within.map(muted))
            .children(support.reason().map(|why| note(why.to_owned())));
        if link.starts_with("https://") {
            let label = match p.source {
                SourceId::Sunrise => tr("Registry page"),
                SourceId::Modrinth => tr("Modrinth page"),
            };
            action_col = action_col.child(secondary("alt-pack-page-link", label).on_click(
                cx.listener(move |_, _: &ClickEvent, _, cx| Launcher::open_url(&link, cx)),
            ));
        }

        let side = card()
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
            .child(about_row(tr("Downloads"), p.downloads.to_string()))
            .children(
                details
                    .filter(|d| !d.updated.is_empty())
                    .map(|d| about_row(tr("Updated"), day(&d.updated))),
            )
            .children((!p.license.is_empty()).then(|| about_row(tr("Licence"), p.license.clone())));

        let shots = details
            .map(|d| {
                d.gallery
                    .iter()
                    .map(|i| super::viewer::Picture::Url(i.url.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let mut body: Vec<AnyElement> = Vec::new();
        match &loaded {
            None => body.push(muted(tr("Loading…")).into_any_element()),
            Some(Err(why)) => body.push(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        div()
                            .text_size(px(tokens::LABEL))
                            .child(crate::selectable::selectable(p.summary.clone())),
                    )
                    .child(note(trf(
                        "The rest of this page couldn't be read: {why}",
                        &[("why", why)],
                    )))
                    .into_any_element(),
            ),
            Some(Ok(page)) if page.0.body.trim().is_empty() => body.push(
                div()
                    .text_size(px(tokens::LABEL))
                    .child(crate::selectable::selectable(p.summary.clone()))
                    .into_any_element(),
            ),
            Some(Ok(page)) => body.push(prose(self.theme, &without_pictures(&page.0.body))),
        }
        let more = versions.len().saturating_sub(VERSIONS);
        if !versions.is_empty() {
            body.push(section(tr("Versions")).into_any_element());
            body.push(
                card()
                    .children(versions.iter().take(VERSIONS).enumerate().map(|(i, v)| {
                        div()
                            .flex()
                            .items_center()
                            .gap(px(16.))
                            .px(px(14.))
                            .py(px(10.))
                            .when(i > 0, |d| d.border_t_1().border_color(palette().rule()))
                            .text_size(px(tokens::LABEL))
                            .child(
                                div()
                                    .w(px(110.))
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
                                    .child(day(&v.published)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(c(palette().muted))
                                    .child(games(&v.game_versions)),
                            )
                    }))
                    .into_any_element(),
            );
            if more > 0 {
                body.push(
                    muted(trf(
                        "And {count} older versions, on its page.",
                        &[("count", &more.to_string())],
                    ))
                    .into_any_element(),
                );
            }
        }
        let gallery = self.gallery(shots, cx);
        self.project_layout(
            "pack-project",
            super::project::Layout {
                header,
                head: header_block,
                action: Some(action_col),
                gallery,
                side: Some(side),
            },
            body,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_and_the_links_around_them_leave_the_text() {
        assert_eq!(
            without_pictures(
                "# T\n[![Badge](a.svg)](https://x) ![banner](b.png)\nSee [docs](https://d)."
            ),
            "# T\n \nSee [docs](https://d)."
        );
        assert_eq!(without_pictures("a ![broken"), "a ![broken");
    }
}
