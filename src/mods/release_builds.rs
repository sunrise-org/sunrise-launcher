//! The builds one registry release works on, opened from its row on the mod's page: what the
//! index says of each build, and where it says nothing, what this computer found.
use super::model::discover_v2::{self, BuildFit};
use super::ui::*;
use super::*;
use crate::paint::palette;
use crate::sheets::{build_name_with_commit, Sheet};
use crate::widgets::button::primary;
use sunrise_launcher::native_mods::local_fit;

impl Launcher {
    /// Opens the builds window for release `version` of registry project `id`.
    pub(super) fn open_release_builds(&mut self, id: &str, version: &str) {
        let Some(catalog) = sunrise::v2::known_index() else {
            return;
        };
        self.mods_state.release_builds =
            discover_v2::release_builds(&catalog.index, id, version, &local_fit::kept);
        if self.mods_state.release_builds.is_some() {
            self.open_sheet(Sheet::ReleaseBuilds);
        }
    }

    pub(crate) fn release_builds_sheet(&mut self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let shown = self.mods_state.release_builds.as_ref()?;
        let profile_build = self.selected_exact_build();
        let p = palette();
        let section = |heading: &'static str, wanted: &[BuildFit]| {
            let rows: Vec<Div> = shown
                .builds
                .iter()
                .filter(|(_, fit)| wanted.contains(fit))
                .map(|(build, fit)| {
                    let here = matches!(fit, BuildFit::FitsHere | BuildFit::DoesNotFitHere);
                    div()
                        .flex()
                        .items_baseline()
                        .gap(px(8.))
                        .py(px(3.))
                        .child(
                            div()
                                .text_size(px(tokens::LABEL))
                                .child(build_name_with_commit(build)),
                        )
                        .children(
                            (profile_build.as_deref() == Some(build.as_str()))
                                .then(|| muted(tr("This profile's build"))),
                        )
                        .children(here.then(|| muted(tr("Checked on this computer"))))
                })
                .collect();
            (!rows.is_empty()).then(|| {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(tokens::SMALL))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(c(p.muted))
                            .child(heading),
                    )
                    .children(rows)
            })
        };
        let body = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(crate::sheets::title(trf(
                "{mod} {version}",
                &[("mod", &shown.name), ("version", &shown.version)],
            )))
            .child(muted(trf(
                "Made for {build}",
                &[("build", &build_name_with_commit(&shown.made_for))],
            )))
            .child(
                div()
                    .id("release-builds-list")
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .max_h(px(420.))
                    .overflow_y_scroll()
                    .children(section(
                        tr("Works on"),
                        &[BuildFit::Fits, BuildFit::FitsHere],
                    ))
                    .children(section(
                        tr("Doesn't work on"),
                        &[BuildFit::DoesNotFit, BuildFit::DoesNotFitHere],
                    ))
                    .children(section(tr("Not checked yet"), &[BuildFit::NotChecked])),
            )
            .child(div().flex().justify_end().child(
                primary("release-builds-close", tr("Close")).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.close_sheet();
                        cx.notify();
                    },
                )),
            ));
        Some(crate::sheets::frame("release-builds", 520., body, cx))
    }
}
