//! The dialog a server is created in, framed as the Add server dialog is (`join::dialog`).
use super::widgets::*;
use super::*;
use crate::sheets::{modal_frame, title};
use crate::widgets::button::{link, secondary};
use crate::widgets::check::{checkbox, Tick};
use crate::widgets::feedback::{banner, Tone};

/// Closes the dialog, unless the server is being made.
fn close_create(this: &mut Launcher) {
    if !this.servers.creating {
        this.servers.create = None;
        this.servers.problem = None;
    }
}

impl Launcher {
    /// The one dialog a server is created in, over the page.
    pub(crate) fn hosting_create(&mut self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let c = self.servers.create.as_ref()?;
        let (new_world, copy, worlds) = (c.new_world, c.copy.clone(), c.worlds.clone());
        let (mode, difficulty, max_players) = (c.mode, c.difficulty, c.max_players);
        let (friends_only, eula, eula_missing) = (c.friends_only, c.eula, c.eula_missing);
        let friends = c.friends.clone();
        let creating = self.servers.creating;
        let choice = |id: &'static str,
                      options: &[(&'static str, &'static str)],
                      current: &str,
                      f: fn(&mut Create, &'static str),
                      cx: &mut Context<Self>| {
            chip_group(id, options, current, cx, move |this, value| {
                if let Some(c) = &mut this.servers.create {
                    f(c, value);
                }
            })
        };
        let part = |name: &'static str, control: Div| {
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(label(name))
                .child(control)
        };
        let world_kind = choice(
            "host-create-world",
            &[("new", "New world"), ("copy", "Copy one of my worlds")],
            if new_world { "new" } else { "copy" },
            |c, v| c.new_world = v == "new",
            cx,
        );
        let world_detail = if new_world {
            div().child(self.host_field(
                Field::Seed,
                tr("Seed (optional: leave empty for a random world)"),
                FIELD_WIDTH,
                cx,
            ))
        } else if worlds.is_empty() {
            hint(tr("No single-player world yet. Choose New world."))
        } else {
            let mut list = div().flex().flex_col().gap(px(2.));
            for (i, (name, path)) in worlds.into_iter().take(8).enumerate() {
                let chosen = copy.as_ref() == Some(&path);
                list = list.child(
                    div()
                        .id(ElementId::named_usize("host-create-copy", i))
                        .tab_index(0)
                        .seen()
                        .border_1()
                        .border_color(gpui::transparent_black())
                        .focus_visible(|s| s.border_color(rgb(palette().focus)))
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .h(px(tokens::CONTROL))
                        .px(px(10.))
                        .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                        .cursor_pointer()
                        .when(chosen, |d| d.bg(rgb(palette().selected)))
                        .child(
                            div()
                                .size(px(12.))
                                .rounded(crate::tokens::r(crate::tokens::R_SMALL))
                                .border_1()
                                .border_color(rgb(if chosen {
                                    palette().text
                                } else {
                                    palette().muted
                                }))
                                .when(chosen, |d| d.bg(rgb(palette().text))),
                        )
                        .child(name)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            if let Some(c) = &mut this.servers.create {
                                c.copy = Some(path.clone());
                            }
                            cx.notify();
                        })),
                );
            }
            list.child(hint(tr(
                "The server gets a copy: your single-player world stays as it is.",
            )))
        };
        let step = |id: &'static str, sign: &'static str, delta: i32, cx: &mut Context<Self>| {
            secondary(id, sign)
                .w(px(32.))
                .px(px(0.))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if let Some(c) = &mut this.servers.create {
                        c.max_players = (c.max_players as i32 + delta).clamp(1, 100) as u32;
                    }
                    cx.notify();
                }))
        };
        let players = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(step("host-create-fewer", "−", -1, cx))
            .child(
                div()
                    .w(px(36.))
                    .flex()
                    .justify_center()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(max_players.to_string()),
            )
            .child(step("host-create-more", "+", 1, cx));
        let mut who = div().flex().flex_col().gap(px(10.)).child(choice(
            "host-create-who",
            &[
                ("friends", "Friends only"),
                ("anyone", "Anyone with the address"),
            ],
            if friends_only { "friends" } else { "anyone" },
            |c, v| c.friends_only = v == "friends",
            cx,
        ));
        if friends_only {
            let mut names = div().flex().flex_wrap().items_center().gap(px(6.));
            for (i, friend) in friends.into_iter().enumerate() {
                names = names.child(
                    div()
                        .id(ElementId::named_usize("host-create-friend", i))
                        .reach_edge()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .h(px(tokens::CONTROL))
                        .px(px(12.))
                        .rounded(crate::tokens::r(crate::tokens::R_PANEL))
                        .border_1()
                        .border_color(palette().input_edge())
                        .text_size(px(tokens::LABEL))
                        .cursor_pointer()
                        .child(friend)
                        .child(div().text_color(rgb(palette().muted)).child("×"))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            if let Some(c) = &mut this.servers.create {
                                if i < c.friends.len() {
                                    c.friends.remove(i);
                                }
                            }
                            cx.notify();
                        })),
                );
            }
            who = who
                .child(names.child(self.host_field(
                    Field::Players,
                    tr("A player's name, then Enter"),
                    240.,
                    cx,
                )))
                .child(hint(tr(
                    "The first one becomes an operator. Add more later under Players.",
                )));
        }
        let eula_line = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(
                checkbox("host-create-eula", if eula { Tick::On } else { Tick::Off }).on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| {
                        if let Some(c) = &mut this.servers.create {
                            c.eula = !c.eula;
                            c.eula_missing = false;
                        }
                        cx.notify();
                    }),
                ),
            )
            .child(
                div()
                    .when(eula_missing, |d| d.text_color(rgb(palette().danger)))
                    .child(tr("I agree to the Minecraft EULA")),
            )
            .child(
                div()
                    .id("host-create-eula-read")
                    .tab_index(0)
                    .seen()
                    .px(px(4.))
                    .rounded(crate::tokens::r(crate::tokens::R_SMALL))
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .focus_visible(|s| s.border_color(rgb(palette().focus)))
                    .text_color(rgb(palette().accent_ink))
                    .cursor_pointer()
                    .child(tr("Read it"))
                    .on_click(|_, _, cx| cx.open_url(servers::EULA_URL)),
            );
        let body = div()
            .flex()
            .flex_col()
            .child(title(tr("Create a server").to_string()))
            .child(hint(tr(
                "Everything has a default: accept Minecraft's terms and Create makes a friends-only survival world. You can change all of it later.",
            )))
            .child(part(
                "Name",
                div().child(self.host_field(Field::Name, tr("My server"), FIELD_WIDTH, cx)),
            ))
            .child(part(
                "World",
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(world_kind)
                    .child(world_detail),
            ))
            .child(part(
                "Game mode",
                choice("host-create-mode", &MODES, mode, |c, v| c.mode = v, cx),
            ))
            .child(part(
                "Difficulty",
                choice(
                    "host-create-difficulty",
                    &DIFFICULTIES,
                    difficulty,
                    |c, v| c.difficulty = v,
                    cx,
                ),
            ))
            .child(part("Most players at once", players))
            .child(part("Who can join", who))
            .child(eula_line);
        let said = if creating {
            let last = self.servers.job_log.back().cloned().unwrap_or_default();
            Some(hint(format!("{} {last}", tr("Creating…"))))
        } else if let Some(p) = &self.servers.problem {
            Some(banner(Tone::Danger, p.message.clone(), None))
        } else if eula_missing {
            Some(banner(
                Tone::Danger,
                tr("Tick the EULA box to create the server."),
                None,
            ))
        } else {
            None
        };
        let prepare = self
            .servers
            .problem
            .as_ref()
            .and_then(|p| p.fix.clone())
            .filter(|fix| !creating && matches!(fix, Fix::Prepare))
            .map(|fix| {
                self.host_button(
                    "host-create-fix",
                    tr("Prepare the game files"),
                    true,
                    cx,
                    move |this, cx| this.host_fix(fix.clone(), cx),
                )
            });
        // An existing folder is the other way in: a quiet link, left of the buttons.
        let existing = link("host-create-folder", tr("Use an existing folder…")).on_click(
            cx.listener(move |this, _: &ClickEvent, _, cx| {
                if !this.servers.creating {
                    this.add_server_folder(cx);
                }
                cx.notify();
            }),
        );
        let buttons = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .children(prepare)
            .child(self.host_button(
                "host-create-cancel",
                tr("Cancel"),
                !creating,
                cx,
                |this, _| close_create(this),
            ))
            .child(
                self.host_primary("host-create-go", tr("Create"), !creating, cx, |this, _| {
                    this.create_server()
                }),
            );
        let body = body.children(said).child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(8.))
                .child(div().children((!creating).then_some(existing)))
                .child(buttons),
        );
        Some(modal_frame("host-create", 520., body, cx, close_create))
    }
}

/// The width of a text field in the dialog: its frame's inside.
const FIELD_WIDTH: f32 = 470.;
