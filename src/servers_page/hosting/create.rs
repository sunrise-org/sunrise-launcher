//! The dialog a server is created in, framed as the Add server dialog is (`join::dialog`) and
//! laid out as the New profile dialog is: small labels over fields in two columns, and a
//! dropdown wherever there is a choice.
use super::*;
use crate::sheets::{modal_frame, title};
use crate::widgets::button::link;
use crate::widgets::check::{checkbox, Tick};
use crate::widgets::dropdown::{Dropdown, Item};
use crate::widgets::feedback::{banner, Tone};
use crate::widgets::field::field_box;

/// Closes the dialog, unless the server is being made.
fn close_create(this: &mut Launcher) {
    if !this.servers.creating {
        this.servers.create = None;
        this.servers.problem = None;
    }
}

/// A small muted label over a control.
fn part(label: &'static str, control: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            div()
                .text_size(px(tokens::SMALL))
                .text_color(rgb(palette().muted))
                .child(tr(label)),
        )
        .child(control)
}

/// Two parts side by side, each half the row.
fn pair(left: Div, right: Div) -> Div {
    div()
        .flex()
        .gap(px(12.))
        .child(left.flex_1().min_w_0())
        .child(right.flex_1().min_w_0())
}

/// A dropdown of the dialog: it opens and closes through `Create::menu`, and `pick` runs with
/// the row chosen.
fn dropdown(
    id: &'static str,
    (which, open): (CreateMenu, bool),
    items: Vec<Item>,
    pick: impl Fn(&mut Launcher, usize, &mut Context<Launcher>) + 'static,
    cx: &mut Context<Launcher>,
) -> Div {
    Dropdown::new(id, items, open)
        .fill()
        .on_toggle({
            let this = cx.entity();
            move |_, cx| {
                this.update(cx, |this, cx| {
                    this.close_menus();
                    let creating = this.servers.creating;
                    if let Some(c) = this.servers.create.as_mut().filter(|_| !creating) {
                        c.menu = (c.menu != Some(which)).then_some(which);
                    }
                    cx.notify();
                })
            }
        })
        .on_close({
            let this = cx.entity();
            move |_, cx| {
                this.update(cx, |this, cx| {
                    if let Some(c) = this
                        .servers
                        .create
                        .as_mut()
                        .filter(|c| c.menu == Some(which))
                    {
                        c.menu = None;
                    }
                    cx.notify();
                })
            }
        })
        .on_pick({
            let this = cx.entity();
            move |n, _, cx| {
                this.update(cx, |this, cx| {
                    pick(this, n, cx);
                    cx.notify();
                })
            }
        })
        .build()
}

/// A dropdown over `options` (value and English label), `current` its value; `set` stores the
/// pick.
fn options_dropdown(
    id: &'static str,
    state: (CreateMenu, bool),
    options: &'static [(&'static str, &'static str)],
    current: &str,
    set: fn(&mut Create, &'static str),
    cx: &mut Context<Launcher>,
) -> Div {
    let items = options
        .iter()
        .map(|&(value, name)| Item::new(tr(name)).current(value == current))
        .collect();
    dropdown(
        id,
        state,
        items,
        move |this, n, _| {
            if let (Some(c), Some(&(value, _))) = (this.servers.create.as_mut(), options.get(n)) {
                set(c, value);
            }
        },
        cx,
    )
}

/// The −/value/+ field of the most players: as tall as a dropdown, the steps inside it.
fn players_field(max_players: u32, cx: &mut Context<Launcher>) -> Div {
    let step = |id: &'static str, sign: &'static str, delta: i32, cx: &mut Context<Launcher>| {
        div()
            .id(id)
            .tab_index(0)
            .seen()
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .w(px(36.))
            .h_full()
            .text_color(rgb(palette().muted))
            .cursor_pointer()
            .hover(|s| s.bg(rgb(palette().hover)).text_color(rgb(palette().text)))
            .child(sign)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                if let Some(c) = &mut this.servers.create {
                    c.max_players = (c.max_players as i32 + delta).clamp(1, 100) as u32;
                }
                cx.notify();
            }))
    };
    field_box(false, false)
        .px(px(0.))
        .justify_between()
        .child(step("host-create-fewer", "−", -1, cx))
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .child(max_players.to_string()),
        )
        .child(step("host-create-more", "+", 1, cx))
}

/// The EULA line: its checkbox, the words and a link to read it.
fn create_eula_line(eula: bool, eula_missing: bool, cx: &mut Context<Launcher>) -> Div {
    div()
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
        )
}

impl Launcher {
    /// A text field of the dialog, in the New profile dialog's field style, as wide as its
    /// column.
    fn create_field(
        &mut self,
        f: Field,
        placeholder: &'static str,
        width: f32,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let focused = self.servers.has_focus(f);
        let invalid = self.servers.invalid.is_some_and(|(g, _)| g == f);
        self.field_on(f, placeholder, width, field_box(focused, invalid), cx)
            .w_full()
            .min_w_0()
    }

    /// The World dropdown: a new world, a copy of one of the player's worlds, or a world file or
    /// folder to import. The seed field or a note about the choice goes under it.
    fn create_world_part(&mut self, cx: &mut Context<Self>) -> Div {
        let Some(c) = self.servers.create.as_ref() else {
            return div();
        };
        let (world, open) = (c.world.clone(), c.menu == Some(CreateMenu::World));
        let mut items = vec![Item::new(tr("New world"))
            .current(world == WorldChoice::New)
            .rule_after()];
        let mut paths = Vec::new();
        for w in &c.worlds {
            let label = if w.profile.is_empty() {
                w.name.clone()
            } else {
                format!("{} · {}", w.profile, w.name)
            };
            items.push(
                Item::new(label)
                    .sub(w.hint.clone())
                    .current(world == WorldChoice::Copy(w.path.clone())),
            );
            paths.push(w.path.clone());
        }
        if !paths.is_empty() {
            if let Some(last) = items.pop() {
                items.push(last.rule_after());
            }
        }
        let mut import =
            Item::new(tr("Import a world file…")).sub(tr("A .zip of a world, or a world folder"));
        if let WorldChoice::Import(path) = &world {
            let name = path.file_name().map_or_else(
                || path.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            import = import.shown(name).current(true);
        }
        items.push(import);
        let menu = dropdown(
            "host-create-world",
            (CreateMenu::World, open),
            items,
            move |this, n, cx| {
                if n == 0 {
                    if let Some(c) = this.servers.create.as_mut() {
                        c.world = WorldChoice::New;
                    }
                } else if let Some(path) = paths.get(n - 1) {
                    if let Some(c) = this.servers.create.as_mut() {
                        c.world = WorldChoice::Copy(path.clone());
                    }
                } else {
                    this.pick_create_world(cx);
                }
            },
            cx,
        );
        let under = match world {
            WorldChoice::New => Some(
                self.create_field(
                    Field::Seed,
                    tr("Seed (optional: leave empty for a random world)"),
                    FIELD_WIDTH,
                    cx,
                )
                .into_any_element(),
            ),
            WorldChoice::Copy(_) => Some(
                hint(tr(
                    "The server gets a copy: your single-player world stays as it is.",
                ))
                .into_any_element(),
            ),
            WorldChoice::Import(_) => None,
        };
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(part("World", menu))
            .children(under)
    }

    /// The names of the friends let in, as chips with the field that adds one.
    fn create_friends(&mut self, friends: Vec<String>, cx: &mut Context<Self>) -> Div {
        let mut names = div().flex().flex_wrap().items_center().gap(px(6.));
        for (i, friend) in friends.into_iter().enumerate() {
            names = names.child(
                div()
                    .id(ElementId::named_usize("host-create-friend", i))
                    .reach_edge()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .h(px(tokens::FIELD))
                    .px(px(12.))
                    .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
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
        let field = self
            .create_field(Field::Players, tr("A player's name, then Enter"), 240., cx)
            .w(px(240.));
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(names.child(field))
            .child(hint(tr(
                "The first one becomes an operator. Add more later under Players.",
            )))
    }

    /// What the dialog says (the job's last line, a problem, the EULA reminder), then the foot:
    /// the existing-folder link, and Cancel and Create.
    fn create_footer(&mut self, eula_missing: bool, cx: &mut Context<Self>) -> Div {
        let creating = self.servers.creating;
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
        div().flex().flex_col().gap(px(16.)).children(said).child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(8.))
                .child(div().children((!creating).then_some(existing)))
                .child(buttons),
        )
    }

    /// The one dialog a server is created in, over the page.
    pub(crate) fn hosting_create(&mut self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let c = self.servers.create.as_ref()?;
        let (mode, difficulty, max_players) = (c.mode, c.difficulty, c.max_players);
        let (friends_only, eula, eula_missing) = (c.friends_only, c.eula, c.eula_missing);
        let (friends, menu) = (c.friends.clone(), c.menu);
        let open = |which: CreateMenu| (which, menu == Some(which));
        let world = self.create_world_part(cx);
        let name = self.create_field(Field::Name, tr("My server"), FIELD_WIDTH, cx);
        let mode_menu = options_dropdown(
            "host-create-mode",
            open(CreateMenu::Mode),
            &MODES,
            mode,
            |c, v| c.mode = v,
            cx,
        );
        let difficulty_menu = options_dropdown(
            "host-create-difficulty",
            open(CreateMenu::Difficulty),
            &DIFFICULTIES,
            difficulty,
            |c, v| c.difficulty = v,
            cx,
        );
        let who_menu = dropdown(
            "host-create-who",
            open(CreateMenu::Who),
            vec![
                Item::new(tr("Friends only")).current(friends_only),
                Item::new(tr("Anyone with the address")).current(!friends_only),
            ],
            |this, n, _| {
                if let Some(c) = this.servers.create.as_mut() {
                    c.friends_only = n == 0;
                }
            },
            cx,
        );
        let eula_line = create_eula_line(eula, eula_missing, cx);
        let body = div()
            .flex()
            .flex_col()
            .child(title(tr("Create a server").to_string()))
            .child(hint(tr(
                "Everything has a default: accept Minecraft's terms and Create makes a friends-only survival world. You can change all of it later.",
            )))
            .child(part("Name", name))
            .child(world)
            .child(pair(part("Game mode", mode_menu), part("Who can join", who_menu)))
            .child(pair(
                part("Difficulty", difficulty_menu),
                part("Most players at once", players_field(max_players, cx)),
            ))
            .children(friends_only.then(|| self.create_friends(friends, cx)))
            .child(eula_line);
        let body = body.child(self.create_footer(eula_missing, cx));
        Some(modal_frame("host-create", 560., body, cx, close_create))
    }
}

/// The width of a text field in the dialog: its frame's inside.
const FIELD_WIDTH: f32 = 520.;
