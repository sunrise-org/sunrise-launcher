//! Settings › Game: how the game starts.
use super::*;
use crate::widgets::button::secondary;
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
        // One line for every choice, so the row keeps its height as the choice changes.
        let while_playing = row(
            tr("While you play"),
            tr("What the launcher does while the game runs. It comes back when the game closes."),
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
            72.,
            |this| &mut this.args_input,
            |this| {
                this.settings.game_args = this.args_input.text().to_string();
                this.save_settings();
            },
            cx,
        );
        // The arguments get the row's whole width under its words, with a copy button inside.
        let copy = crate::widgets::motion_bits::copy_button(
            "alt-game-args-copy",
            tr("Copy"),
            self.settings.game_args.clone(),
        );
        let args = stacked(
            tr("Game arguments"),
            tr("Extra arguments for the game, passed through as they are. They apply the next time it starts."),
            // The copy button sits over the box's top-right corner, inside it.
            div()
                .w_full()
                .relative()
                .child(args)
                .child(div().absolute().top(px(6.)).right(px(6.)).child(copy)),
        );
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(group(vec![while_playing, args]))
            .child(group(vec![source]))
    }
}
