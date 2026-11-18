//! Settings for the whole launcher, one section at a time. The game build and its channel stay
//! in the play bar, the one place that shows them.
use super::*;
use crate::frame::Reach as _;
use crate::frame::{button, BAD, BG, CARD, GOLD, LINE, MUTED, SOFT, TEXT};
use crate::mods::{chip, choices, switch, Choose};
use gpui::rgb;
use std::rc::Rc;
use storage::Category;
use sunrise_launcher::telemetry;
use sunrise_launcher_telemetry::Choices;

#[derive(Clone, Copy, Default, Eq, PartialEq)]
pub(super) enum Section {
    #[default]
    Account,
    Game,
    Mods,
    Storage,
    Privacy,
    Launcher,
}

/// The Settings section the window opens on, when what the last start left needs one: a data
/// folder that did not open or a move that failed or finished (Storage has its recoveries).
pub(super) fn startup_section(location_error: bool, location_note: bool) -> Option<Section> {
    (location_error || location_note).then_some(Section::Storage)
}

impl Section {
    const ALL: [Section; 6] = [
        Section::Account,
        Section::Game,
        Section::Mods,
        Section::Storage,
        Section::Privacy,
        Section::Launcher,
    ];

    fn label(self) -> &'static str {
        match self {
            Section::Account => "Account",
            Section::Game => "Game",
            Section::Mods => "Mods",
            Section::Storage => "Storage",
            Section::Privacy => "Privacy",
            Section::Launcher => "Launcher",
        }
    }

    fn about(self) -> &'static str {
        match self {
            Section::Account => "Who plays, and the Microsoft accounts signed in here.",
            Section::Game => "How the game starts. Its build is chosen in the bar below.",
            Section::Mods => "What mods with native code may do on this computer.",
            Section::Storage => "Where your files are, and the space downloads take.",
            Section::Privacy => "What Sunrise sends, and what it never does.",
            Section::Launcher => "Language, network and this version.",
        }
    }
}

#[derive(Default)]
pub(super) struct SettingsView {
    pub(super) section: Section,
    languages: bool,
    setup: bool,
    note: Option<String>,
}

/// A card of rows with a rule between them.
pub(crate) fn group(rows: Vec<Div>) -> Div {
    let mut card = div()
        .flex()
        .flex_col()
        .rounded(px(tokens::R_CARD))
        .border_1()
        .border_color(rgb(SOFT))
        .bg(rgb(CARD));
    for (i, row) in rows.into_iter().enumerate() {
        card = card.child(
            row.px(px(16.))
                .py(px(14.))
                .when(i > 0, |d| d.border_t_1().border_color(rgb(SOFT))),
        );
    }
    card
}

pub(crate) fn hint(text: impl Into<SharedString>) -> Div {
    div()
        .text_color(rgb(MUTED))
        .text_size(px(tokens::SMALL))
        .child(crate::selectable::selectable(text))
}

pub(crate) fn row(
    title: impl Into<SharedString>,
    about: impl Into<SharedString>,
    actions: Div,
) -> Div {
    let about = about.into();
    div()
        .flex()
        .items_center()
        .gap(px(16.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(crate::selectable::selectable(title)),
                )
                .when(!about.is_empty(), |d| d.child(hint(about))),
        )
        .child(actions.flex_none().flex().items_center().gap(px(6.)))
}

/// A row whose control needs the full width: the words above, the control below.
fn stacked(title: impl Into<SharedString>, about: impl Into<SharedString>, below: Div) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(crate::selectable::selectable(title)),
                )
                .child(hint(about)),
        )
        .child(below)
}

impl Launcher {
    /// A [`row`] whose controls are wide: in a narrow window they would leave the words a
    /// column a syllable wide, so there they go under the words.
    fn wide_row(
        &self,
        title: impl Into<SharedString>,
        about: impl Into<SharedString>,
        actions: Div,
    ) -> Div {
        if self.window_width < 960. {
            stacked(title, about, actions.flex().flex_wrap().gap(px(6.)))
        } else {
            row(title, about, actions)
        }
    }
}

pub(crate) fn off(b: Stateful<Div>) -> Stateful<Div> {
    b.opacity(0.4).cursor_default().tab_stop(false)
}

pub(crate) fn danger(b: Stateful<Div>) -> Stateful<Div> {
    b.text_color(rgb(BAD))
}

pub(crate) fn warning(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(tokens::SMALL))
        .text_color(rgb(BAD))
        .child(crate::selectable::selectable(text))
}

impl Launcher {
    pub(super) fn settings(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.args_focused = self.args_focus.is_focused(window);
        self.proxy_focused = self.proxy_focus.is_focused(window);
        let section = self.ui.settings.section;
        let mut nav = div().flex().flex_col().gap(px(2.)).w(px(180.)).flex_none();
        for (n, s) in Section::ALL.into_iter().enumerate() {
            let on = s == section;
            nav = nav.child(
                div()
                    .id(ElementId::named_usize("alt-settings-nav", n))
                    .reach()
                    .h(px(tokens::FIELD))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .rounded(px(tokens::R_CONTROL))
                    .cursor_pointer()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(if on { TEXT } else { MUTED }))
                    .when(on, |d| d.bg(rgb(CARD)))
                    .hover(|d| d.text_color(rgb(TEXT)))
                    .child(tr(s.label()))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.settings_open(s);
                        cx.notify();
                    })),
            );
        }
        let body = match section {
            Section::Account => self.settings_account(cx),
            Section::Game => self.settings_game(cx),
            Section::Mods => self.settings_mods(cx),
            Section::Storage => self.settings_storage(cx),
            Section::Privacy => self.settings_privacy(cx),
            Section::Launcher => self.settings_launcher(cx),
        };
        let pane = div()
            .flex_1()
            .min_w_0()
            .max_w(px(760.))
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(tokens::HEADING))
                            .font_weight(FontWeight::BOLD)
                            .child(crate::selectable::selectable(tr(section.label()))),
                    )
                    .child(hint(tr(section.about()))),
            )
            .children(self.location_error.clone().map(warning))
            .child(body);
        div()
            .id("alt-settings-scroll")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&crate::frame::scrolls("settings"))
            .px(px(32.))
            .py(px(24.))
            .text_color(rgb(TEXT))
            .flex()
            .flex_col()
            .gap(px(20.))
            .child(
                div()
                    .text_size(px(tokens::DISPLAY))
                    .font_weight(FontWeight::BOLD)
                    .child(tr("Settings")),
            )
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(28.))
                    .child(nav)
                    .child(pane),
            )
            .into_any_element()
    }

    /// Opens a section, leaving no question from another one standing.
    pub(super) fn settings_open(&mut self, section: Section) {
        let view = &mut self.ui.settings;
        view.section = section;
        view.languages = false;
        view.setup = false;
        view.note = None;
        self.storage_revoke = None;
        self.storage_confirm = None;
        self.collection.confirm_delete = false;
        if section == Section::Storage {
            self.refresh_storage(None, false);
        }
    }

    fn settings_account(&mut self, cx: &mut Context<Self>) -> Div {
        let theme = self.theme;
        let selected = self.accounts.selected.clone();
        let mut rows = Vec::new();
        for (i, a) in self.accounts.accounts.clone().into_iter().enumerate() {
            let plays = selected.as_ref() == Some(&a.uuid);
            let status = self.auth_status.get(&a.uuid);
            let needs_sign_in =
                status.is_some_and(|s| s.state == account::session::State::SignInRequired);
            let progress = self.auth_progress.get(&a.uuid).map(|phase| phase.label());
            let busy = self.auth_busy.contains_key(&a.uuid);
            let said = match (progress, status) {
                (Some(doing), _) => tr(doing).to_string(),
                (None, Some(s)) if needs_sign_in => tr(&s.message).to_string(),
                _ if plays => tr("Plays when you press Play").to_string(),
                _ => tr("Signed in").to_string(),
            };
            let mut actions = div();
            if needs_sign_in {
                let uuid = a.uuid.clone();
                let again = button(
                    ElementId::named_usize("alt-account-again", i),
                    tr("Sign in again"),
                );
                actions = actions.child(if busy {
                    off(again)
                } else {
                    again.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.re_sign_in(&uuid);
                        cx.notify();
                    }))
                });
            } else if !plays {
                let uuid = a.uuid.clone();
                actions = actions.child(
                    button(ElementId::named_usize("alt-account-play", i), tr("Play as")).on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.play_as(&uuid);
                            cx.notify();
                        }),
                    ),
                );
            }
            let uuid = a.uuid.clone();
            let out = button(ElementId::named_usize("alt-account-out", i), tr("Sign out"));
            actions = actions.child(if busy {
                off(out)
            } else {
                out.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    let Some(generation) = this.begin_auth_job(&uuid) else {
                        return;
                    };
                    let (tx, uuid) = (this.tx.clone(), uuid.clone());
                    std::thread::spawn(move || {
                        let mut accounts = Accounts::load();
                        let _ = tx.send(Msg::AccountProgress(
                            uuid.clone(),
                            generation,
                            account::session::Phase::SigningOut,
                        ));
                        let result = accounts
                            .remove(account::store(), &uuid)
                            .and_then(|()| accounts.save())
                            .map(|()| accounts.has_owner(account::store()));
                        let _ = tx.send(Msg::AccountRemoved(uuid, generation, result));
                    });
                    cx.notify();
                }))
            });
            rows.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_none()
                            .rounded(px(tokens::R_CONTROL))
                            .overflow_hidden()
                            .child(
                                self.profile_heads
                                    .draw(Some(&a.uuid), 36., self.scale, &theme),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(a.name.clone()),
                            )
                            .child(if needs_sign_in {
                                warning(said)
                            } else {
                                hint(said)
                            }),
                    )
                    .child(actions.flex_none().flex().items_center().gap(px(6.))),
            );
        }
        let none = rows.is_empty();
        rows.push(row(
            tr(if none {
                "Not signed in"
            } else {
                "Another account"
            }),
            tr("Sign in with the Microsoft account that owns Minecraft: Java Edition."),
            div().child(
                button("alt-add-account", tr("Sign in with Microsoft")).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.sign_in_to_play();
                        cx.notify();
                    },
                )),
            ),
        ));
        group(rows)
    }

    fn settings_game(&mut self, cx: &mut Context<Self>) -> Div {
        use backend::WhilePlaying;
        let current = self.settings.while_playing.here();
        let mut ways = vec![
            (WhilePlaying::Stay, "Stay open"),
            (WhilePlaying::Minimize, "Minimise"),
        ];
        if cfg!(target_os = "macos") {
            ways.push((WhilePlaying::Hide, "Hide"));
        }
        let ways = ways
            .into_iter()
            .map(|(way, label)| {
                let pick: Choose = Rc::new(move |this, _| {
                    this.settings.while_playing = way;
                    this.save_settings();
                });
                (tr(label).into(), way == current, pick)
            })
            .collect();
        let picker = div().child(choices("alt-while-playing", ways, cx).gap(px(6.)));
        let while_playing = row(
            tr("While you play"),
            tr(match current {
                WhilePlaying::Stay => "The launcher stays open beside the game.",
                WhilePlaying::Minimize => {
                    "The launcher goes down when the game starts, and comes back when it closes."
                }
                WhilePlaying::Hide => {
                    "The launcher hides when the game starts, and comes back when it closes."
                }
            }),
            picker,
        );
        let minecraft = self.settings.minecraft_dir.as_deref().map_or_else(
            || {
                tr("Not found yet. Sunrise reads the game's own files from your Minecraft folder.")
                    .to_string()
            },
            short,
        );
        let source = row(
            tr("Minecraft folder"),
            minecraft,
            div().child(
                button("alt-minecraft-folder", tr("Choose…"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.choose_folder(cx))),
            ),
        );
        let args = self.settings_field(
            "alt-game-args",
            self.args_focus.clone(),
            self.args_focused,
            &self.args_input,
            "For example --server-thread",
            |this| &mut this.args_input,
            |this| {
                this.settings.game_args = this.args_input.text().to_string();
                this.save_settings();
            },
            cx,
        );
        let args = stacked(
            tr("Game arguments"),
            tr("Extra arguments for the game, passed through as they are. They apply the next time it starts."),
            args,
        );
        let prune = row(
            tr("Remove old game builds"),
            tr("After an update, builds nothing uses any more are deleted."),
            div().child(
                switch("alt-prune", self.settings.auto_prune()).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.settings.prune_old_versions = Some(!this.settings.auto_prune());
                        this.save_settings();
                        cx.notify();
                    },
                )),
            ),
        );
        let asking = self.ui.settings.setup;
        let mut setup = row(
            tr("A second set of mods"),
            tr("A setup keeps its own mods, worlds and servers. Switch between setups in the bar below."),
            div().child(
                button("alt-create", tr(if asking { "Cancel" } else { "New setup…" })).on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.ui.settings.setup = !this.ui.settings.setup;
                        this.ui.settings.note = None;
                        cx.notify();
                    }),
                ),
            ),
        );
        if asking {
            let input = self.field(servers_page::Field::InstanceName, "Setup name", 250., cx);
            setup = div().flex().flex_col().gap(px(12.)).child(setup).child(
                div().flex().items_center().gap(px(8.)).child(input).child(
                    button("alt-create-empty", tr("Create setup")).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            let name = this.servers.value(servers_page::Field::InstanceName).trim();
                            match instances::create(name, None) {
                                Ok(_) => {
                                    this.reload_instances();
                                    this.ui.settings.setup = false;
                                    this.servers
                                        .text(servers_page::Field::InstanceName)
                                        .set(String::new());
                                    this.ui.settings.note = Some(
                                        tr("Setup created. Select it in the bar below.").into(),
                                    );
                                }
                                Err(e) => this.ui.settings.note = Some(e),
                            };
                            cx.notify();
                        },
                    )),
                ),
            );
        }
        if let Some(note) = self.ui.settings.note.clone() {
            setup = div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(setup)
                .child(hint(note));
        }
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(group(vec![while_playing, args]))
            .child(group(vec![source, prune]))
            .child(group(vec![setup]))
    }

    fn settings_mods(&mut self, cx: &mut Context<Self>) -> Div {
        let allowed = self.settings.native_mods_allowed_at.is_some();
        let (about, actions) = match (allowed, self.storage_revoke) {
            (false, _) => (
                tr("Not allowed. Sunrise asks the first time a mod needs it, and builds or runs nothing before you answer.").to_string(),
                div(),
            ),
            (true, None) => (
                tr("Allowed. Mods with native code run inside the game with the same access to your computer as the game has.").to_string(),
                div().child(match self.storage_counting {
                    Some(_) => off(button("alt-native-revoke", tr("Counting your mods…"))),
                    None => button("alt-native-revoke", tr("Take back…"))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.storage_count_native(cx))),
                }),
            ),
            (true, Some((mods, setups))) => (
                match (mods, setups) {
                    (0, _) => tr("No mod with native code is turned on. Sunrise will ask again before the next one.").to_string(),
                    (1, _) => tr("This also turns off your one mod with native code. Turn it back on in Mods; Sunrise asks again then.").to_string(),
                    (_, 1) => trf(
                        "This also turns off {mods} mods with native code. Turn them back on in Mods; Sunrise asks again then.",
                        &[("mods", &mods.to_string())],
                    ),
                    _ => trf(
                        "This also turns off {mods} mods with native code in {setups} setups. Turn them back on in each setup's Mods; Sunrise asks again then.",
                        &[("mods", &mods.to_string()), ("setups", &setups.to_string())],
                    ),
                },
                div()
                    .child(
                        button("alt-native-keep", tr("Keep it")).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.storage_revoke = None;
                                cx.notify();
                            },
                        )),
                    )
                    .child(danger(button("alt-native-revoke-yes", tr("Take it back"))).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| this.turn_off_native_mods(cx)),
                    )),
            ),
        };
        let mut native = row(tr("Native code"), about, actions);
        if let Some(error) = self.storage_revoke_error.clone() {
            native = div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(native)
                .child(hint(error));
        }
        group(vec![native])
    }

    fn settings_storage(&mut self, cx: &mut Context<Self>) -> Div {
        let game = backend::game_home();
        let backups = self
            .selected_instance()
            .map_or_else(|| game.clone(), |i| i.home.clone())
            .join("backups");
        let has_backups = backups.is_dir();
        let files = row(
            tr("Game files and worlds"),
            short(&game),
            div().child({
                let dir = game.clone();
                button("alt-open-game", tr("Open folder"))
                    .on_click(move |_, _, cx| glass_toast::open(cx, &dir))
            }),
        );
        let open_backups = button("alt-open-backups", tr("Open folder"));
        let kept = row(
            tr("World backups"),
            if has_backups {
                short(&backups)
            } else {
                tr("None yet. Sunrise makes one before a mod change that could harm a world.")
                    .to_string()
            },
            div().child(if has_backups {
                open_backups.on_click(move |_, _, cx| glass_toast::open(cx, &backups))
            } else {
                off(open_backups)
            }),
        );

        let busy = self.storage_pending;
        let total = self
            .storage_report
            .as_ref()
            .map(|report| report.rows.iter().map(|r| r.remaining_bytes).sum::<u64>());
        let nothing = busy || total.is_none_or(|bytes| bytes == 0);
        // Counting that failed is not still counting.
        let uncounted = if self.storage_error.is_some() && !busy {
            tr("Not counted")
        } else {
            tr("Counting…")
        };
        let clear_all = button("alt-storage-clear-all", tr("Clear all…"));
        let mut cache = vec![row(
            tr("Downloads kept for reuse"),
            trf(
                "{size}. Files Sunrise can fetch or make again; your worlds, mods and settings are never part of it.",
                &[(
                    "size",
                    &total.map_or_else(|| uncounted.to_string(), storage::format_bytes),
                )],
            ),
            div()
                .child(
                    button("alt-storage-recount", tr("Recount")).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.refresh_storage(None, false);
                            cx.notify();
                        },
                    )),
                )
                .child(if nothing {
                    off(clear_all)
                } else {
                    clear_all.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.storage_confirm = Some(None);
                        cx.notify();
                    }))
                }),
        )];
        // A problem no row owns is said once, quietly, under the total.
        if let Some(error) = self.storage_error.clone() {
            let total = cache.remove(0);
            cache.push(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(total)
                    .child(hint(error)),
            );
        }
        if let Some(asked) = self.storage_confirm {
            let bytes: u64 = self
                .storage_report
                .iter()
                .flat_map(|report| &report.rows)
                .filter(|r| asked.is_none_or(|category| r.category == category))
                .map(|r| r.remaining_bytes)
                .sum();
            let size = storage::format_bytes(bytes);
            let question = match asked {
                None => trf(
                    "Clear everything kept for reuse? Up to {size} is freed. Files a game or build is using stay.",
                    &[("size", &size)],
                ),
                Some(category) => trf(
                    "Clear {category}? Up to {size} is freed. Files a game or build is using stay.",
                    &[
                        ("category", tr(glass_storage::category_label(category))),
                        ("size", &size),
                    ],
                ),
            };
            cache.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .child(div().flex_1().min_w_0().child(question))
                    .child(button("alt-storage-keep", tr("Keep")).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.storage_confirm = None;
                            cx.notify();
                        },
                    )))
                    .child(
                        danger(button("alt-storage-clear-yes", tr("Clear"))).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                if let Some(clear) = this.storage_confirm.take() {
                                    this.refresh_storage(clear, clear.is_none());
                                }
                                cx.notify();
                            },
                        )),
                    ),
            );
        }
        let limit = self.settings.cache_limit_gib.unwrap_or(25);
        let mut limits = div();
        for (n, (value, label)) in [
            (10u64, "10 GB"),
            (25, "25 GB"),
            (50, "50 GB"),
            (100, "100 GB"),
            (0, "No limit"),
        ]
        .into_iter()
        .enumerate()
        {
            limits = limits.child(
                // Each its own Tab stop, pressed with Enter or Space: a smaller limit clears
                // downloads, which an arrow key passing over it must not do.
                chip(
                    ElementId::named_usize("alt-storage-limit", n),
                    tr(label),
                    value == limit,
                )
                .tab_index(0)
                .seen()
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.settings.cache_limit_gib = Some(value);
                    if let Err(error) = this.settings.save() {
                        this.storage_error = Some(error);
                    }
                    cx.notify();
                })),
            );
        }
        cache.push(self.wide_row(
            tr("Keep at most"),
            tr("Past this, the oldest downloads are cleared by themselves."),
            limits,
        ));
        for (n, category) in Category::ALL.into_iter().enumerate() {
            let found = self
                .storage_report
                .as_ref()
                .and_then(|report| report.rows.iter().find(|r| r.category == category));
            let size = found.map_or_else(
                || uncounted.to_string(),
                |r| storage::format_bytes(r.remaining_bytes),
            );
            let empty = busy || found.is_none_or(|r| r.remaining_bytes == 0);
            let clear = button(ElementId::named_usize("alt-storage-clear", n), tr("Clear…"));
            let mut line = row(
                trf(
                    "{category} · {size}",
                    &[
                        ("category", tr(glass_storage::category_label(category))),
                        ("size", &size),
                    ],
                ),
                tr(glass_storage::category_about(category)),
                div().child(if empty {
                    off(clear)
                } else {
                    clear.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.storage_confirm = Some(Some(category));
                        cx.notify();
                    }))
                }),
            );
            let kept: Vec<Div> = found
                .iter()
                .flat_map(|r| &r.kept)
                .map(|k| {
                    // A unit that was refused whole was not counted either.
                    if k.bytes == 0 {
                        return hint(k.reason.clone());
                    }
                    hint(trf(
                        "Space it couldn't free: {bytes}. {reason}",
                        &[
                            ("bytes", &storage::format_bytes(k.bytes)),
                            ("reason", &k.reason),
                        ],
                    ))
                })
                .collect();
            if !kept.is_empty() {
                line = div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(line)
                    .children(kept);
            }
            cache.push(line);
        }
        let notes = [
            self.client
                .is_some()
                .then(|| hint(tr("A game is running. Files it may use are kept."))),
            busy.then(|| hint(tr("Counting, or clearing…"))),
        ];
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(group(vec![self.location_row(cx)]))
            .child(group(vec![files, kept]))
            .child(group(cache))
            .children(notes.into_iter().flatten())
    }

    fn settings_privacy(&mut self, cx: &mut Context<Self>) -> Div {
        let consent = self.collection.consent.clone();
        // A row: its name with a "?" that says exactly what is sent, a line on why, the switch.
        let line = |this: &Self,
                    name: &'static str,
                    summary: &'static str,
                    full: &'static str,
                    toggle: Stateful<Div>,
                    cx: &mut Context<Self>| {
            let pinned = this.collection.sent_open.contains(&name);
            let shown = pinned || this.collection.sent_hover == Some(name);
            let help = div()
                .id(name)
                .tab_index(0)
                .seen()
                .flex_none()
                .size(px(16.))
                .rounded_full()
                .border_1()
                .border_color(rgb(if pinned { TEXT } else { LINE }))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(tokens::CAPTION))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(if pinned { TEXT } else { MUTED }))
                .cursor_pointer()
                .focus_visible(|s| s.border_color(rgb(GOLD)))
                .child("?")
                .on_hover(cx.listener(move |this, over: &bool, _, cx| {
                    let hover = &mut this.collection.sent_hover;
                    match *over {
                        true => *hover = Some(name),
                        false if *hover == Some(name) => *hover = None,
                        false => {}
                    }
                    cx.notify();
                }))
                // A press pins it open, for keyboards and touch; another press closes it.
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    let list = &mut this.collection.sent_open;
                    match list.contains(&name) {
                        true => list.clear(),
                        false => *list = vec![name],
                    }
                    cx.notify();
                }));
            let said = shown.then(|| {
                // Under the "?", or over it when the row is low in the window.
                gpui::deferred(
                    div().absolute().top(px(24.)).left_0().child(
                        gpui::anchored().snap_to_window_with_margin(px(8.)).child(
                            div()
                                .w(px(340.))
                                .p(px(12.))
                                .rounded(px(tokens::R_CONTROL))
                                .border_1()
                                .border_color(rgb(LINE))
                                .bg(rgb(CARD))
                                .shadow_lg()
                                .occlude()
                                .child(hint(tr(full))),
                        ),
                    ),
                )
                .with_priority(10)
            });
            div()
                .flex()
                .items_start()
                .gap(px(16.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .relative()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(div().font_weight(FontWeight::SEMIBOLD).child(tr(name)))
                                .child(help)
                                .children(said),
                        )
                        .child(hint(tr(summary))),
                )
                .child(toggle)
        };
        let ping = line(
            self,
            telemetry::PING_NAME,
            telemetry::PING_SUMMARY,
            telemetry::PING,
            switch("alt-ping", !self.settings.ping_off).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    this.settings.ping_off = !this.settings.ping_off;
                    this.save_settings();
                    cx.notify();
                },
            )),
            cx,
        );
        // The game reads the same switch (`backend::no_ai`).
        let no_ai_on = backend::no_ai();
        let no_ai = line(
            self,
            "No AI",
            "Hides and turns off every AI feature.",
            "Every AI feature in the launcher and the game is hidden and turned off: agent control of the launcher and the game, sunrise-ctl and the MCP servers.",
            switch("alt-no-ai", no_ai_on).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    // A failed save is a problem like any other: the play bar says it.
                    if let Err(e) = backend::set_no_ai(!no_ai_on) {
                        this.client_error = Some(e);
                    }
                    cx.notify();
                },
            )),
            cx,
        );
        let mut reports = Vec::new();
        for (n, &(kind, name, summary, what)) in telemetry::SWITCHES.iter().enumerate() {
            let on = !consent.needs_asking() && consent.choices.allows(kind);
            let toggle = switch(ElementId::named_usize("alt-report", n), on).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    let mut choices = if this.collection.consent.needs_asking() {
                        Choices::NONE
                    } else {
                        this.collection.consent.choices
                    };
                    choices.set(kind, !on);
                    this.answer_consent(choices);
                    cx.notify();
                },
            ));
            reports.push(line(self, name, summary, what, toggle, cx));
        }
        let id = match consent.install_id.as_str() {
            "" => tr("No install ID: nothing has been collected.").to_string(),
            id => trf(
                "Install ID {prefix}…{suffix}",
                &[("prefix", &id[..4]), ("suffix", &id[id.len() - 4..])],
            ),
        };
        let waiting = if sunrise_launcher_telemetry::endpoint().is_some() {
            let count = sunrise_launcher_telemetry::outbox::pending(&telemetry::home()).len();
            trf(
                "{count} waiting to be sent",
                &[("count", &count.to_string())],
            )
        } else {
            tr("Sending is turned off on this computer").to_string()
        };
        let confirm = self.collection.confirm_delete;
        let viewing = self.collection.view.is_some();
        let mut yours = self.wide_row(
            tr("Your data"),
            format!("{id} · {waiting}"),
            div()
                .child(
                    button(
                        "alt-data-view",
                        tr(if viewing {
                            "Hide"
                        } else {
                            "See what would be sent"
                        }),
                    )
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.toggle_view();
                        cx.notify();
                    })),
                )
                .child(
                    danger(button(
                        "alt-data-delete",
                        tr(if confirm {
                            "Delete everything?"
                        } else {
                            "Delete my data"
                        }),
                    ))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        if this.collection.confirm_delete {
                            this.collection.note = Some(match telemetry::delete_my_data() {
                                Ok(done) => done,
                                Err(e) => e,
                            });
                            this.collection.consent = telemetry::consent();
                            this.collection.view = None;
                        }
                        this.collection.confirm_delete = !this.collection.confirm_delete;
                        cx.notify();
                    })),
                ),
        );
        if let Some((what, json)) = self.collection.view.clone() {
            yours = div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(yours)
                .child(hint(what))
                .child(
                    div()
                        .id("alt-data-json")
                        .max_h(px(220.))
                        .overflow_y_scroll()
                        .p(px(12.))
                        .rounded(px(tokens::R_CONTROL))
                        .bg(rgb(BG))
                        .border_1()
                        .border_color(rgb(SOFT))
                        .font_family(theme::MONO)
                        .text_size(px(tokens::SMALL))
                        .text_color(rgb(MUTED))
                        .children(json.lines().map(|l| div().child(l.to_string()))),
                );
        }
        if let Some(note) = self.collection.note.clone() {
            yours = div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(yours)
                .child(hint(note));
        }
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(group(vec![ping]))
            .child(hint(tr("These reports are off until you turn them on.")))
            .child(group(reports))
            .child(group(vec![yours]))
            .child(hint(tr(telemetry::NEVER)))
            .child(group(vec![no_ai]))
    }

    fn settings_launcher(&mut self, cx: &mut Context<Self>) -> Div {
        let picking = self.ui.settings.languages;
        let mut language = row(
            tr("Language"),
            "",
            div().child(
                button(
                    "alt-language",
                    self.settings
                        .language
                        .map_or(tr("Same as the system"), |l| l.name()),
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.ui.settings.languages = !this.ui.settings.languages;
                    cx.notify();
                })),
            ),
        );
        if picking {
            let chosen = self.settings.language;
            let languages = std::iter::once(None)
                .chain(i18n::Language::ALL.map(Some))
                .map(|choice| {
                    let pick: Choose = Rc::new(move |this, _| {
                        this.settings.language = choice;
                        i18n::set(this.settings.language());
                        this.save_settings();
                        this.ui.settings.languages = false;
                    });
                    (
                        choice.map_or(tr("Same as the system"), |l| l.name()).into(),
                        choice == chosen,
                        pick,
                    )
                })
                .collect();
            let options = choices("alt-languages", languages, cx)
                .max_h(px(190.))
                .overflow_y_scroll()
                .gap(px(6.));
            language = div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(language)
                .child(options);
        }
        let refused = (!self.proxy_focused)
            .then(|| sunrise_launcher::proxy::refuse(self.proxy_input.text()))
            .flatten();
        let field = self.settings_field(
            "alt-proxy",
            self.proxy_focus.clone(),
            self.proxy_focused,
            &self.proxy_input,
            "For example http://127.0.0.1:7890",
            |this| &mut this.proxy_input,
            |this| this.save_proxy(),
            cx,
        );
        let mut proxy = stacked(
            tr("Proxy"),
            tr("Downloads, sign-ins and updates go through it. Empty means none; HTTPS_PROXY then applies if the launcher was started with one."),
            field,
        );
        if let Some(why) = refused {
            proxy = proxy.child(warning(why));
        }
        let version = self.launcher_update_row(cx);
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(group(vec![language]))
            .child(group(vec![proxy]))
            .child(group(vec![version]))
    }

    /// This launcher's version and its own update: the state of the staged download, and the
    /// action that fits it (Update, or Download again when this copy can't replace itself).
    fn launcher_update_row(&mut self, cx: &mut Context<Self>) -> Div {
        let title = trf(
            "Sunrise Launcher {version}",
            &[("version", backend::VERSION)],
        );
        let failed = |error: &str| trf("the update failed: {error}", &[("error", error)]);
        let (about, action, bad) = match &self.self_update {
            SelfUpdate::Downloading(version, done) => (
                trf(
                    "downloading {version}, {percent}%",
                    &[
                        ("version", version),
                        ("percent", &format!("{:.0}", done * 100.)),
                    ],
                ),
                None,
                false,
            ),
            SelfUpdate::Ready(version) => (
                trf("{version} available", &[("version", version)]),
                Some(
                    button("alt-launcher-update", tr("Update")).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.apply_launcher_update(cx);
                            cx.notify();
                        },
                    )),
                ),
                false,
            ),
            SelfUpdate::Stuck(error) => {
                let url = backend::download_page(&self.links.website);
                (
                    self.launcher_update_help(error.as_deref()),
                    self.launcher_download_available().then(|| {
                        button("alt-launcher-download", tr("Download again"))
                            .on_click(move |_, _, cx| cx.open_url(&url))
                    }),
                    error.is_some(),
                )
            }
            SelfUpdate::Failed(error) => {
                let url = backend::download_page(&self.links.website);
                (
                    failed(error),
                    self.launcher_download_available().then(|| {
                        button("alt-launcher-download", tr("Download again"))
                            .on_click(move |_, _, cx| cx.open_url(&url))
                    }),
                    true,
                )
            }
            SelfUpdate::Idle => match &self.updates.launcher {
                Some(release) => {
                    let version = backend::launcher_version(&release.tag).unwrap_or_default();
                    let about = trf("{version} available", &[("version", version)]);
                    let release = release.clone();
                    let get = button(
                        "alt-launcher-get",
                        trf("Update to {version}", &[("version", version)]),
                    );
                    (
                        about,
                        Some(if backend::self_update_allowed() {
                            get.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.update_launcher(release.clone());
                                cx.notify();
                            }))
                        } else {
                            off(get)
                        }),
                        false,
                    )
                }
                None if self.checked.is_some() && self.update_error.is_none() => {
                    (tr("up to date").to_string(), None, false)
                }
                None => (String::new(), None, false),
            },
        };
        let mut words = div().flex().flex_col().gap(px(2.));
        words = words.child(div().font_weight(FontWeight::SEMIBOLD).child(title));
        if !about.is_empty() {
            words = words.child(if bad { warning(about) } else { hint(about) });
        }
        div()
            .flex()
            .items_center()
            .gap(px(16.))
            .child(words.flex_1().min_w_0())
            .child(div().flex_none().children(action))
    }

    /// One line of text in this look, saved as it is typed.
    #[allow(clippy::too_many_arguments)]
    fn settings_field(
        &self,
        id: &'static str,
        focus: FocusHandle,
        focused: bool,
        input: &line_edit::LineEdit,
        placeholder: &'static str,
        get: fn(&mut Self) -> &mut line_edit::LineEdit,
        edited: fn(&mut Self),
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = crate::mods::field_theme(self.theme);
        let to_focus = focus.clone();
        let field = div()
            .id(id)
            .track_focus(&focus)
            .seen()
            .h(px(tokens::FIELD))
            .px(px(12.))
            .flex()
            .items_center()
            .overflow_hidden()
            .rounded(px(tokens::R_CONTROL))
            .bg(rgb(BG))
            .border_1()
            .border_color(rgb(if focused { GOLD } else { LINE }))
            .font_family(theme::MONO)
            .text_size(px(tokens::LABEL))
            .text_color(rgb(TEXT))
            .cursor_text()
            .child(line_edit::content(
                input,
                &theme,
                focused,
                false,
                false,
                placeholder,
            ))
            .on_click(cx.listener(move |_, _: &ClickEvent, window, cx| {
                window.focus(&to_focus, cx);
                cx.notify();
            }))
            .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, window, cx| {
                match line_edit::handle(get(this), ev, cx) {
                    line_edit::Key::Pass => return,
                    line_edit::Key::Escape => window.blur(cx),
                    line_edit::Key::Edited => edited(this),
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
            }));
        div().child(line_edit::mouse(field, cx, get))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_data_folder_problem_opens_settings_on_storage() {
        assert!(startup_section(true, false) == Some(Section::Storage));
        assert!(startup_section(false, true) == Some(Section::Storage));
        assert!(startup_section(false, false).is_none());
    }
}
