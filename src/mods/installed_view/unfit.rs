//! A row's switch slot for a mod the game was made without: "Not on <build>" and why.
use super::*;

impl Launcher {
    /// Where a row's switch would be, for a mod the game was made without: "Not on Nov 17" and
    /// a "?" that says why in one sentence, the changed items for its author, and the way on.
    pub(in crate::mods) fn unfit(
        &self,
        n: usize,
        r: &Row,
        unfit: model::Unfit,
        busy: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        use crate::widgets::button::secondary;
        let open = self.mods_state.unfit_open.as_deref() == Some(r.id.as_str());
        let id = r.id.clone();
        let help = status(
            "information-circle",
            tr("Needs an update"),
            palette().warning,
        )
        .id(("alt-unfit-help", n))
        .reach()
        // Its glyph stands where every other row's does.
        .px(px(8.))
        .ml(px(-8.))
        .h(px(28.))
        .when(open, |d| d.bg(c(palette().hover)))
        .hover(|s| s.bg(c(palette().hover)))
        .cursor_pointer()
        .tooltip(crate::glass_ui::tip(trf(
            "{mod} needs an update for {build}. It comes back when its author releases one.",
            &[("mod", &r.title), ("build", &unfit.on)],
        )))
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            let m = &mut this.mods_state;
            m.unfit_open = match m.unfit_open.as_ref() == Some(&id) {
                true => None,
                false => Some(id.clone()),
            };
            m.unfit_details = false;
            cx.stop_propagation();
            cx.notify();
        }));
        let said = open.then(|| {
            let sentence = match &unfit.made_for {
                Some((_, made)) => trf(
                    "{mod} needs an update for {on}. It was made for {made}, and comes back when its author releases a new version. It stays installed.",
                    &[("mod", &r.title), ("on", &unfit.on), ("made", made)],
                ),
                None => trf(
                    "{mod} needs an update for {on}. It comes back when its author releases a new version. It stays installed.",
                    &[("mod", &r.title), ("on", &unfit.on)],
                ),
            };
            let details = self.mods_state.unfit_details;
            let technical = (!unfit.changes.is_empty()).then(|| {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        div()
                            .id("alt-unfit-details")
.reach()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(tokens::SMALL))
                            .text_color(c(palette().muted))
                            .cursor_pointer()
                            .child(crate::widgets::motion_bits::disclosure(
                                "installed-unfit-details",
                                details,
                                12.,
                                c(palette().muted),
                            ))
                            .child(tr("Details"))
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.mods_state.unfit_details = !this.mods_state.unfit_details;
                                cx.stop_propagation();
                                cx.notify();
                            })),
                    )
                    .children(details.then(|| {
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .children(unfit.changes.iter().map(|change| {
                                div()
                                    .text_size(px(tokens::SMALL))
                                    .font_family(crate::theme::MONO)
                                    .text_color(c(palette().muted))
                                    .child(change.clone())
                            }))
                    }))
            });
            // One way on: a version that fits when the registry has a newer one, else back to
            // the build it was made for when that build is still here.
            let action = match (busy, &r.update, &unfit.made_for) {
                (true, _, _) => None,
                (false, Some(_), _) => {
                    let b = secondary("alt-unfit-update",
                        trf("Update {mod}", &[("mod", &r.title)]));
                    if nightly::installed_source(r).is_some() && !self.nightly_catalogue_current() {
                        Some(dim(b).on_click(|_, _, cx| cx.stop_propagation()))
                    } else {
                        let row = r.clone();
                        Some(b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.mods_state.unfit_open = None;
                            if nightly::installed_source(&row).is_some() {
                                this.submit_source_update(&row.id);
                            } else {
                                this.update_one(&row);
                            }
                            cx.stop_propagation();
                            cx.notify();
                        })))
                    }
                }
                (false, None, Some((tag, made))) if self.installed.contains(tag) => {
                    let b = secondary("alt-unfit-stay",
                        trf("Stay on {build}", &[("build", made)]));
                    let tag = tag.clone();
                    Some(b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.mods_state.unfit_open = None;
                        this.stay_on(tag.clone());
                        cx.stop_propagation();
                        cx.notify();
                    })))
                }
                _ => {
                    let id = r.id.clone();
                    let origin = nightly::installed_source(r).cloned();
                    Some(secondary("alt-unfit-check", tr("Check for update")).on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            if let Some(origin) = &origin {
                                this.check_source_updates(Some(origin));
                            } else {
                                this.check_mod_update(&id);
                            }
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ))
                }
            };
            // Opens under the "?", or over it when the row is low in the window.
            let popup = div()
                    .w(px(360.))
                    .max_w(px(self.content_width() - 28.))
                    .p(px(14.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                    .border_1()
                    .border_color(palette().input_edge())
                    .bg(c(palette().raised))
                    .shadow_lg()
                    .occlude()
                    .child(div().text_size(px(tokens::LABEL)).child(sentence))
                    .children(technical)
                    .children(action.map(|a| div().flex().justify_end().child(a)));
            gpui::deferred(
                div().absolute().top(px(24.)).right_0().child(
                    gpui::anchored()
                        .anchor(gpui::Anchor::TopRight)
                        .snap_to_window_with_margin(px(8.))
                        .child(popup),
                ),
            )
            .with_priority(10)
        });
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .child(div().relative().flex_none().child(help).children(said))
    }

    /// "Stay on Nov 16" for a mod made for it: the setup stays on that build, where the mod
    /// runs. The next Play builds there as usual.
    pub(super) fn stay_on(&mut self, tag: String) {
        let Some(id) = self.selected_instance().map(|i| i.id.clone()) else {
            return;
        };
        if !self.guard() {
            return;
        }
        match instances::edit(&id, |meta| meta.version = Some(tag)) {
            Ok(_) => self.reload_instances(),
            Err(e) => self.mods_state.notice = Some(Err(e)),
        }
    }
}
