//! "Check for updates" on the Mods tab: the button beside Add file, and the sheet that lists
//! what the check found, each mod ticked to be updated until the player says otherwise. The
//! check and the install are `mods::updates`; this is how they look.
use super::*;
use crate::mods::updates::{size_label, Candidate};
use crate::widgets::button::{button, link, primary, secondary, Look, Size, Style};
use crate::widgets::check::{checkbox, Tick};
use sunrise_launcher::browser::images;

/// The picture of a row in the sheet.
const ROW_PICTURE: f32 = 32.;

impl Launcher {
    /// The header's Check for updates: while it asks the registry its icon turns and it is
    /// disabled; a check that found nothing says "Up to date" for a few seconds; the game or a
    /// build running dims it with the reason.
    pub(super) fn update_check_button(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let look = Look::new(Style::Secondary, Size::Medium);
        if self.mods_state.update_check.is_checking() {
            let ink = Hsla::from(gpui::rgb(crate::paint::palette().text));
            let icon = crate::glass_ui::icon("refresh", 18., ink);
            let spinning = if crate::motion::still() {
                icon.into_any_element()
            } else {
                icon.with_animation(
                    "home-mods-check-spin",
                    crate::motion::endless(
                        gpui::Animation::new(Duration::from_millis(900)).repeat(),
                    ),
                    |icon, t| {
                        icon.with_transformation(gpui::Transformation::rotate(gpui::radians(
                            t * std::f32::consts::TAU,
                        )))
                    },
                )
                .into_any_element()
            };
            // The icon is added last and drawn first: the button takes it after its label.
            return button("home-mods-check", tr("Checking…"), look.disabled(true))
                .flex_row_reverse()
                .child(spinning);
        }
        if let Some(why) = self.mods_locked() {
            return button(
                "home-mods-check",
                tr("Check for updates"),
                look.icon("refresh").disabled(true),
            )
            .tooltip(crate::glass_ui::tip(why));
        }
        let up_to_date = self.mods_state.update_check.up_to_date(Instant::now());
        let (label, icon) = if up_to_date {
            (tr("Up to date"), "tick-02")
        } else {
            (tr("Check for updates"), "refresh")
        };
        button("home-mods-check", label, look.icon(icon)).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                this.check_for_updates(cx);
                cx.notify();
            },
        ))
    }

    /// `add` with Check for updates before it, on the tabs that have updates (Mods).
    pub(super) fn home_manage(
        &mut self,
        add: Stateful<Div>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        div()
            .id("home-mods-manage")
            .flex()
            .items_center()
            .gap(px(8.))
            .child(self.update_check_button(cx))
            .child(add)
    }

    /// The tile of the registry mod `item`, from its project's icon when it has one.
    fn update_picture(&mut self, item: &Candidate) -> Div {
        let image = item.project.as_deref().and_then(|project| {
            let icon = sunrise::v2::shown_index()
                .and_then(|catalog| Some(catalog.index.project(project)?.icon.clone()))?;
            self.v2_mod_image(project, &icon, images::ROW)
        });
        tile(image, &item.id, &item.name, ROW_PICTURE)
    }

    fn update_row(
        &mut self,
        n: usize,
        item: &Candidate,
        picked: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = crate::paint::palette();
        let muted = gpui::rgb(p.muted);
        let id = item.id.clone();
        let tick = if picked { Tick::On } else { Tick::Off };
        let slot = match &item.blocked {
            None => checkbox(("home-update-pick", n), tick)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.mods_state.update_check.toggle(&id);
                    cx.notify();
                }))
                .into_any_element(),
            Some(_) => div().w(px(18.)).flex_none().into_any_element(),
        };
        let mut line = format!("{} → {}", item.current, item.next);
        if let Some(size) = item.size {
            line = format!("{line} · {}", size_label(size));
        }
        let skip_id = item.id.clone();
        let page = item.project.clone();
        let links = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.))
            .children(page.map(|project| {
                link(("home-update-open", n), tr("Open page")).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.close_sheet();
                        this.mods_show_project(project.clone());
                        cx.notify();
                    },
                ))
            }))
            .child(
                link(("home-update-skip", n), tr("Skip this version")).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.skip_checked_update(&skip_id);
                        cx.notify();
                    },
                )),
            );
        let text = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .child(
                div()
                    .truncate()
                    .text_size(px(crate::tokens::LABEL))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(item.name.clone()),
            )
            .child(
                div()
                    .truncate()
                    .font_family(crate::theme::MONO)
                    .text_size(px(crate::tokens::SMALL))
                    .text_color(muted)
                    .child(line),
            )
            .children(item.blocked.clone().map(|why| {
                div()
                    .text_size(px(crate::tokens::SMALL))
                    .text_color(gpui::rgb(p.warning))
                    .child(why)
            }));
        let picture = self.update_picture(item);
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .py(px(8.))
            .when(item.blocked.is_some(), |d| d.opacity(0.7))
            .child(slot)
            .child(picture)
            .child(text)
            .child(links)
    }

    /// What the check found: a titled list with a tick per mod, the controls over it, and
    /// Cancel and Update n under it. With only skipped versions left it says so, with the way to
    /// offer them again.
    pub(crate) fn updates_sheet(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let check = &self.mods_state.update_check;
        let items = check.items.clone();
        let picked = check.picks.len();
        let tickable = check.tickable();
        let hidden = check.hidden;
        let p = crate::paint::palette();
        let muted = |text: String| {
            div()
                .text_size(px(crate::tokens::LABEL))
                .text_color(gpui::rgb(p.muted))
                .child(text)
        };
        let heading = match items.len() {
            0 => tr("No new updates").to_owned(),
            1 => tr("1 update available").to_owned(),
            n => trf("{count} updates available", &[("count", &n.to_string())]),
        };
        let mut body = div().flex().flex_col().child(crate::sheets::title(heading));
        if items.is_empty() {
            body = body.child(muted(trf(
                "{count} skipped versions are hidden.",
                &[("count", &hidden.to_string())],
            )));
        } else {
            let controls = div()
                .flex()
                .items_center()
                .gap(px(12.))
                .child(
                    link("home-update-all", tr("Select all")).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.mods_state.update_check.select_all();
                            cx.notify();
                        },
                    )),
                )
                .child(
                    link("home-update-none", tr("Select none")).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.mods_state.update_check.select_none();
                            cx.notify();
                        },
                    )),
                )
                .child(div().flex_1())
                .child(muted(trf(
                    "{picked} of {total} selected",
                    &[
                        ("picked", &picked.to_string()),
                        ("total", &tickable.to_string()),
                    ],
                )));
            let mut list = div()
                .id("home-update-list")
                .flex()
                .flex_col()
                .max_h(px(340.))
                .overflow_y_scroll();
            for (n, item) in items.iter().enumerate() {
                let on = self.mods_state.update_check.picks.contains(&item.id);
                list = list.child(self.update_row(n, item, on, cx));
            }
            body = body.child(controls).child(list);
        }
        let footer = self.updates_footer(items.is_empty(), picked, hidden, cx);
        crate::sheets::frame("home-updates", 640., body.child(footer), cx)
    }

    /// Under the list: what was skipped (with the way to offer it again), Cancel and Update n.
    fn updates_footer(
        &mut self,
        empty: bool,
        picked: usize,
        hidden: usize,
        cx: &mut Context<Self>,
    ) -> Div {
        let muted = |text: String| {
            div()
                .text_size(px(crate::tokens::LABEL))
                .text_color(gpui::rgb(crate::paint::palette().muted))
                .child(text)
        };
        let locked = self.mods_locked();
        let update = trf("Update {count}", &[("count", &picked.to_string())]);
        let go = if picked == 0 || locked.is_some() {
            let disabled = button(
                "home-update-go",
                update,
                Look::new(Style::Primary, Size::Medium).disabled(true),
            );
            match locked {
                Some(why) => disabled.tooltip(crate::glass_ui::tip(why)),
                None => disabled,
            }
        } else {
            primary("home-update-go", update).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    this.install_checked_updates();
                    cx.notify();
                },
            ))
        };
        let footer = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .children(
                (hidden > 0 && !empty)
                    .then(|| muted(trf("{count} skipped", &[("count", &hidden.to_string())]))),
            )
            .children((hidden > 0).then(|| {
                link("home-update-clear", tr("Offer skipped again")).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.clear_skipped_updates();
                        cx.notify();
                    },
                ))
            }))
            .child(div().flex_1())
            .child(
                secondary(
                    "home-update-cancel",
                    if empty { tr("OK") } else { tr("Cancel") },
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.close_sheet();
                    cx.notify();
                })),
            )
            .children((!empty).then_some(go));
        footer
    }
}
