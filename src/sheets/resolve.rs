//! Resolve: an update is held because some mods don't work with it yet.

use super::*;
use crate::widgets::button::secondary;

impl Launcher {
    /// An update is out and some mods don't work with it yet: which, and the two ways on.
    /// Staying is the default and what closing does; nothing here runs until a button is pressed.
    pub(super) fn resolve_sheet(&mut self, held: Held, cx: &mut Context<Self>) -> Stateful<Div> {
        let Held {
            to,
            plays,
            mods,
            details,
            worlds,
            blocked,
            base,
            ..
        } = held;
        let known = self.mods_named(&mods);
        let names: Vec<String> = known.iter().map(|(name, _)| name.clone()).collect();
        let them = match names.as_slice() {
            [one] => one.clone(),
            _ => tr("them").to_string(),
        };
        let heading = match (blocked, names.as_slice()) {
            (true, _) => trf("{to} is out, but one of your mods stops it", &[("to", &to)]),
            (false, [one]) => trf(
                "{to} is out, but {mod} doesn't work with it yet",
                &[("to", &to), ("mod", one)],
            ),
            (false, many) => trf(
                "{to} is out, but {count} of your mods don't work with it yet",
                &[("to", &to), ("count", &many.len().to_string())],
            ),
        };
        let lead = if blocked {
            trf(
                "One of your mods can't be built for any game build. Remove it under Mods, or wait for its author. Until then you keep playing {plays}.",
                &[("plays", &plays)],
            )
        } else {
            trf(
                "You can keep playing {plays} and wait, or update without {them}.",
                &[("plays", &plays), ("them", &them)],
            )
        };

        // The mods, each with what can be done about it right here.
        let mut can_update = false;
        let waiting = known.iter().filter(|(_, newer)| newer.is_none()).count();
        let mut list = div()
            .id("alt-resolve-list")
            .flex()
            .flex_col()
            .max_h(px(280.))
            .overflow_y_scroll()
            .rounded(crate::tokens::r(crate::tokens::R_CARD))
            .border_1()
            .border_color(palette().input_edge())
            .bg(rgb(palette().background));
        for (n, (id, (name, newer))) in mods.iter().zip(&known).enumerate() {
            // Only a mod that can do something here gets a line under its name; the ones that
            // wait for their author are said once, under the list.
            let about = newer.as_ref().map(|version| {
                trf(
                    "Version {version} is out. Update it, then check the game update again.",
                    &[("version", version)],
                )
            });
            let update = newer.as_ref().map(|_| {
                can_update = true;
                let id = id.clone();
                secondary(
                    format!("alt-resolve-mod-{}", n),
                    trf("Update {mod}", &[("mod", name)]),
                )
                .bg(rgb(palette().text))
                .border_color(rgb(palette().text))
                .text_color(rgb(palette().background))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.close_sheet();
                    this.update_mod(&id);
                    cx.notify();
                }))
            });
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(14.))
                    .py(px(8.))
                    .when(n > 0, |d| d.border_t_1().border_color(rgb(palette().line)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(name.clone()))
                            .children(about.map(muted)),
                    )
                    .children(update),
            );
        }

        // The mods with no newer version, said once for all of them.
        let wait_line = (waiting > 0).then(|| {
            muted(match waiting {
                1 => tr("It needs an update from its author; when one is out, it shows here and under Mods.").to_string(),
                _ => tr("They need updates from their authors; when one is out, it shows here and under Mods.").to_string(),
            })
        });

        // What updating without them does, said before the button that does it.
        let without = (!blocked).then(|| {
            let kept = trf(
                "Updating without {them} turns it off; it isn't removed, and you can turn it back on when it fits.",
                &[("them", &them)],
            );
            match worlds.as_slice() {
                [] => kept,
                saved => format!(
                    "{kept} {}",
                    trf(
                        "{worlds}: opened without the mod, a world loses what the mod added to it, for good. Back it up first in the game (Edit World, then Make Backup) if you want to keep it.",
                        &[("worlds", &saved.join(", "))],
                    )
                ),
            }
        });

        let stay = secondary(
            "alt-resolve-stay",
            trf("Stay on {build} for now", &[("build", &plays)]),
        )
        // Recommended, unless a mod can simply be updated.
        .when(!can_update, |b| {
            b.bg(rgb(palette().text))
                .border_color(rgb(palette().text))
                .text_color(rgb(palette().background))
        })
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            // Staying is what already happens: nothing is written.
            this.close_sheet();
            cx.notify();
        }));
        let go = (!blocked).then(|| {
            secondary(
                "alt-resolve-update",
                trf("Update without {them}", &[("them", &them)]),
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.close_sheet();
                this.build_native_on(Some(base.clone()), true);
                cx.notify();
            }))
        });

        let open = self.ui.sheets.details;
        let more = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .id("alt-resolve-details")
                    .tab_index(0)
                    .seen()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(tokens::SMALL))
                    .text_color(rgb(palette().muted))
                    .cursor_pointer()
                    .child(crate::widgets::motion_bits::disclosure(
                        "resolve-details",
                        open,
                        11.,
                        rgb(palette().muted).into(),
                    ))
                    .child(tr("Details"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.ui.sheets.details = !this.ui.sheets.details;
                        cx.notify();
                    })),
            )
            .children(open.then(|| {
                crate::code_box::code_box(
                    "alt-resolve-details-box",
                    details.into_iter().map(Into::into).collect(),
                    None,
                )
                .border_0()
                .text_size(px(tokens::CAPTION))
            }));

        let body = div()
            .flex()
            .flex_col()
            .child(title(heading))
            .child(div().text_size(px(tokens::BODY)).child(lead))
            .children((!mods.is_empty()).then_some(list))
            .children(wait_line)
            .children(without.map(muted))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_end()
                    .gap(px(8.))
                    .children(go)
                    .child(stay),
            )
            .child(more);
        frame("alt-resolve", 580., body, cx)
    }
}
