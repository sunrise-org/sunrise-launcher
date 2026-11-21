//! Mods › Discover's source catalogue: no GitHub work until a player opens a listing or checks updates.
use super::ui::*;
use super::*;
use crate::frame::button;
use std::time::{Duration, SystemTime};
use sunrise_launcher::browser::sunrise::sources::{self, Catalogue, Source};
use sunrise_launcher::instances::Channel;

mod jobs;
mod project;

pub(super) use jobs::Event;
pub(super) use project::{short_sha, source_error};

const PAGE_SIZE: usize = 48;
const FRESH: Duration = Duration::from_secs(600);

#[derive(Default)]
pub(super) struct State {
    overrides: HashMap<String, Channel>,
    catalogue: Option<Catalogue>,
    reading: bool,
    pub(super) open: Option<jobs::Open>,
    detail_request: u64,
    pub(super) limit: usize,
    pub(super) work: jobs::Work,
}

fn browse_channel(overrides: &HashMap<String, Channel>, profile: &str, saved: Channel) -> Channel {
    overrides.get(profile).copied().unwrap_or(saved)
}

impl Launcher {
    pub(super) fn nightly_selected(&self) -> bool {
        let instance = self.selected_instance();
        browse_channel(
            &self.ui.mods.nightly.overrides,
            instance.map_or(instances::DEFAULT, |i| i.id.as_str()),
            instance.map_or(Channel::Stable, |i| i.channel),
        ) == Channel::Nightly
    }

    pub(super) fn source_catalogue_bar(&self, cx: &mut Context<Self>) -> Div {
        let nightly = self.nightly_selected();
        let views = [(tr("Stable"), false), (tr("Nightly"), true)]
            .into_iter()
            .map(|(label, is_nightly)| {
                let choose: Choose = Rc::new(move |this, _| {
                    let profile = this
                        .selected_instance()
                        .map_or_else(|| instances::DEFAULT.to_owned(), |i| i.id.clone());
                    let channel = if is_nightly {
                        Channel::Nightly
                    } else {
                        Channel::Stable
                    };
                    this.ui.mods.nightly.overrides.insert(profile, channel);
                    this.ui.mods.menu = None;
                });
                (label.into(), is_nightly == nightly, choose)
            })
            .collect();
        let target = self.nightly_target_name();
        div()
            .flex()
            .flex_none()
            .flex_wrap()
            .items_center()
            .gap_x(px(10.))
            .gap_y(px(4.))
            .child(choices("mod-catalogue", views, cx).gap(px(3.)))
            .child(muted(if nightly {
                tr("Build directly from GitHub. Listed source code is not reviewed.")
            } else {
                tr("Reviewed releases from the Sunrise registry.")
            }))
            .child(div().flex_1())
            .child(muted(trf("Profile: {build}", &[("build", &target)])))
    }

    pub(super) fn nightly_target_name(&self) -> String {
        self.selected_instance()
            .and_then(|instance| self.profile_target(instance))
            .map_or_else(
                || tr("No game build selected").into(),
                |tag| crate::sheets::build_name(&tag),
            )
    }

    pub(super) fn read_nightly_catalogue(&mut self, force: bool) {
        let n = &mut self.ui.mods.nightly;
        if n.reading
            || (!force
                && n.catalogue
                    .as_ref()
                    .is_some_and(|c| c.checked_at.elapsed().is_ok_and(|time| time < FRESH)))
        {
            return;
        }
        n.reading = true;
        let tx = self.ui.mods.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(super::Event::Nightly(Event::Catalogue(sources::catalogue(
                force,
            ))));
        });
    }

    pub(super) fn nightly_discover(
        &mut self,
        window: &mut Window,
        focus: &FocusHandle,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.read_nightly_catalogue(false);
        let header = self.mods_header(false, focus, window, cx);
        let mut page = div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(14.))
            .p(px(24.))
            .text_color(c(palette().text))
            .child(header)
            .child(self.source_catalogue_bar(cx))
            .children(self.strip(cx));
        let Some(catalogue) = &self.ui.mods.nightly.catalogue else {
            return page
                .child(muted(tr("Loading the Nightly list…")))
                .into_any_element();
        };
        if catalogue.error.is_some() {
            let retry = button("nightly-catalogue-retry", tr("Try again"));
            let retry = if self.ui.mods.nightly.reading {
                dim(retry)
            } else {
                retry.on_click(cx.listener(|this, _, _, cx| {
                    this.read_nightly_catalogue(true);
                    cx.notify();
                }))
            };
            let (line, cached) = match catalogue.saved_at {
                Some(time) => (
                    trf(
                        "You're offline · showing the saved list from {date}",
                        &[("date", &source_time(time))],
                    ),
                    true,
                ),
                None => (tr("Nightly list unavailable offline").to_owned(), false),
            };
            page = page.child(
                card()
                    .p(px(12.))
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(8.))
                    .child(div().flex_1().min_w_0().child(line))
                    .child(retry),
            );
            if !cached {
                return page.into_any_element();
            }
        }
        let shown = source_matches(&catalogue.sources, &self.ui.mods.filters.text);
        let limit = self.ui.mods.nightly.limit.max(PAGE_SIZE);
        let index = sunrise::known_index();
        let game = self.mods_game();
        let mut grid = div()
            .grid()
            .grid_cols(nightly_columns(self.content_width()))
            .gap(px(12.));
        for (n, source) in shown.iter().take(limit).enumerate() {
            let source = (*source).clone();
            let also = index
                .as_ref()
                .and_then(|index| sources::also_on_stable(&source, index, &game));
            let actions = div()
                .flex()
                .flex_wrap()
                .justify_end()
                .items_center()
                .gap(px(6.))
                .children(also.map(|v| {
                    let id = source.id().to_owned();
                    button(
                        ("also-on-stable", n),
                        trf("Also on Stable {version}", &[("version", &v.version)]),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.mods_show_project(id.clone());
                        cx.notify();
                    }))
                }))
                .child(
                    button(("nightly-details", n), tr("Details")).on_click(cx.listener({
                        let source = source.clone();
                        move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.inspect_nightly(source.clone(), false);
                            cx.notify();
                        }
                    })),
                );
            grid = grid.child(
                card()
                    .id(("nightly-card", n))
                    .reach()
                    .min_w_0()
                    .cursor_pointer()
                    .hover(|style| style.border_color(c(palette().muted)))
                    .p(px(12.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap(px(10.))
                            .min_w_0()
                            .child(tile(None::<PathBuf>, source.id(), source.name(), 40.))
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .gap(px(3.))
                                    .child(
                                        div()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(source.name().to_owned()),
                                    )
                                    .child(muted(source.repo().to_owned()).truncate()),
                            ),
                    )
                    .child(
                        muted(source.summary().to_owned())
                            .line_clamp(2)
                            .text_ellipsis(),
                    )
                    .child(div().flex_1())
                    .tooltip(crate::glass_ui::tip(source.name().to_owned()))
                    .child(actions)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.inspect_nightly(source.clone(), false);
                        cx.notify();
                    })),
            );
        }
        let more = (shown.len() > limit).then(|| {
            button("nightly-more", tr("Show more")).on_click(cx.listener(move |this, _, _, cx| {
                this.ui.mods.nightly.limit = limit + PAGE_SIZE;
                cx.notify();
            }))
        });
        let results = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(grid)
            .when(shown.is_empty(), |d| {
                d.child(muted(tr("No Nightly mods match your search.")))
            })
            .children(more);
        page.child(
            div()
                .flex()
                .justify_between()
                .items_center()
                .child(muted(trf(
                    "{count} source projects",
                    &[("count", &shown.len().to_string())],
                )))
                .child(muted(tr("Name A–Z"))),
        )
        .child(
            div()
                .id("nightly-catalogue-results")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(&crate::frame::scrolls("nightly-discover"))
                .child(results),
        )
        .into_any_element()
    }
}

fn source_matches<'a>(sources: &'a [Source], filter: &str) -> Vec<&'a Source> {
    let words: Vec<_> = filter.split_whitespace().map(str::to_lowercase).collect();
    let mut shown: Vec<_> = sources
        .iter()
        .filter(|source| {
            let text = format!(
                "{} {} {} {}",
                source.name(),
                source.id(),
                source.repo(),
                source.summary()
            )
            .to_lowercase();
            words.iter().all(|word| text.contains(word))
        })
        .map(|source| (source.name().to_lowercase(), source.id(), source))
        .collect();
    shown.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
    shown.into_iter().map(|(_, _, source)| source).collect()
}

fn nightly_columns(width: f32) -> u16 {
    (((width + 12.) / 274.).floor() as u16).clamp(1, 3)
}

pub(super) fn installed_source(row: &Row) -> Option<&sunrise_launcher::github_mods::Origin> {
    row.local
        .as_ref()?
        .github
        .as_ref()
        .filter(|origin| origin.source.is_some())
}

pub(super) fn source_provenance(origin: &sunrise_launcher::github_mods::Origin) -> String {
    let repo = format!("{}/{}", origin.owner, origin.repo);
    trf(
        "Built from source · {repo} @ {sha}",
        &[("repo", &repo), ("sha", short_sha(&origin.commit))],
    )
}

fn source_time(time: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(time)
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_overrides_apply_only_to_the_browsed_profile() {
        let overrides = HashMap::from([("garden".into(), Channel::Nightly)]);
        assert_eq!(
            browse_channel(&overrides, "garden", Channel::Stable),
            Channel::Nightly
        );
        assert_eq!(
            browse_channel(&overrides, "farm", Channel::Stable),
            Channel::Stable
        );
        assert_eq!(
            browse_channel(&overrides, "grove", Channel::Nightly),
            Channel::Nightly
        );
    }

    #[test]
    fn compact_and_wide_catalogues_keep_card_width() {
        assert_eq!(nightly_columns(660.), 2);
        assert_eq!(nightly_columns(1120.), 3);
    }
}
