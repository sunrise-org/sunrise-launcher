//! Settings › Game: how the game starts.
use super::*;
use crate::paint::palette;
use crate::widgets::button::secondary;
use crate::widgets::focus::{c, mix};
use gpui::SharedString;

impl Launcher {
    pub(super) fn settings_game(&mut self, cx: &mut Context<Self>) -> Div {
        use backend::WhilePlaying;
        let current = self.settings.while_playing.here();
        let mut ways = vec![
            (WhilePlaying::Stay, crate::i18n::tr_key("Stay open")),
            (WhilePlaying::Minimize, crate::i18n::tr_key("Minimise")),
        ];
        if cfg!(target_os = "macos") {
            ways.push((WhilePlaying::Hide, crate::i18n::tr_key("Hide")));
        }
        let ways: Vec<(WhilePlaying, SharedString)> = ways
            .into_iter()
            .map(|(way, label)| (way, tr(label).into()))
            .collect();
        let this = cx.entity();
        let picker = div().child(choices(
            "alt-while-playing",
            &ways,
            current,
            move |way, _, cx| {
                this.update(cx, |this, cx| {
                    this.settings.while_playing = way;
                    this.save_settings();
                    cx.notify();
                })
            },
        ));
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
                secondary("alt-minecraft-folder", tr("Choose…"))
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
        let args = self.wide_row(
            tr("Game arguments"),
            tr("Extra arguments for the game, passed through as they are. They apply the next time it starts."),
            args.w(px(260.)),
        );
        let asking = self.ui.settings.setup;
        let mut setup = row(
            tr("Profiles"),
            tr("A profile keeps its own mods, worlds and servers. Switch between profiles in the sidebar."),
            div().child(
                secondary("alt-create", tr(if asking { "Cancel" } else { "New profile…" })).on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.ui.settings.setup = !this.ui.settings.setup;
                        this.ui.settings.note = None;
                        cx.notify();
                    }),
                ),
            ),
        );
        if asking {
            let input = self.field(
                servers_page::Field::InstanceName,
                tr("Profile name"),
                250.,
                cx,
            );
            setup = div().flex().flex_col().gap(px(12.)).child(setup).child(
                div().flex().items_center().gap(px(8.)).child(input).child(
                    secondary("alt-create-empty", tr("Create profile")).on_click(cx.listener(
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
                                        tr("Profile created. Select it in the sidebar.").into(),
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
        // Deleting the profile is the one danger on the page: its own card, tinted red.
        let delete = self
            .selected_instance()
            .filter(|p| p.kind == instances::Kind::Own && !self.location_unavailable)
            .map(|profile| {
                let target = profile.clone();
                let ink = c(palette().danger);
                div()
                    .px(px(16.))
                    .py(px(14.))
                    .rounded(crate::tokens::r(crate::tokens::R_CARD))
                    .border_1()
                    .border_color(ink.opacity(0.5))
                    .bg(mix(palette().card(), ink, 0.15))
                    .child(row(
                        trf("Selected profile: {name}", &[("name", &target.name)]),
                        tr("This profile and its worlds and mods will move to the Trash."),
                        div().child(
                            crate::widgets::button::danger(
                                "settings-delete-profile",
                                tr("Delete profile…"),
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.confirm_profile_delete(&target);
                                    cx.notify();
                                },
                            )),
                        ),
                    ))
            });
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(group(vec![while_playing, args]))
            .child(group(vec![source]))
            .child(group(vec![setup]))
            .children(delete)
    }
}
