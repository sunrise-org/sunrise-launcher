//! The Servers page's form: its fields and focus, the keyboard's Enter, the port and player
//! edits, and the player heads.

use super::*;
use crate::widgets::field::field_box;

impl Launcher {
    pub(crate) fn focus_handle(&mut self, f: Field, cx: &mut Context<Self>) -> FocusHandle {
        self.servers
            .focus
            .entry(f)
            .or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true))
            .clone()
    }

    /// A one-line text field: typing, paste, Backspace, Enter to confirm, Escape, Tab to the next.
    pub(crate) fn field(
        &mut self,
        f: Field,
        placeholder: &'static str,
        width: f32,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = self.theme;
        let focused = self.servers.focused == Some(f);
        let invalid = self.servers.invalid.is_some_and(|(g, _)| g == f);
        // Home's Add server line is in the glass look already; the other fields follow with
        // their pages.
        // The dark appearance draws every field in the glass look.
        let profile_field = matches!(
            f,
            Field::InstanceName | Field::InstanceGroup | Field::ImportSearch
        );
        let glass = t.glass;
        let base = if profile_field {
            crate::widgets::field::field_box(focused, invalid)
                .h(px(crate::tokens::FIELD))
                .px(px(12.))
                .text_size(px(crate::tokens::BODY))
                .text_color(gpui::rgb(crate::paint::palette().text))
        } else if glass {
            // The mods filter and the log search sit by buttons: the mockup's small `FIELD`, as
            // tall as they are.
            crate::widgets::field::field_box(focused, invalid)
                .when(
                    matches!(
                        f,
                        Field::Motd
                            | Field::Port
                            | Field::Token
                            | Field::AddPlayer
                            | Field::DeleteServer
                    ),
                    |d| {
                        d.h(px(36.))
                            .px(px(12.))
                            .rounded(px(crate::glass_ui::rad(radius::CONTROL)))
                    },
                )
                // The server creation dialog's are the large `FIELD_LG`: 48 px, 15 px text, white
                // at 6 %, 10 % while typed in.
                .when(!invalid && matches!(f, Field::Name | Field::Seed), |d| {
                    d.h(px(48.))
                        .text_size(px(15.))
                        .bg(crate::glass_ui::white(if focused { 0.1 } else { 0.06 }))
                })
                // The accent's hex: `h-9 w-[96px] rounded-[10px] px-3 text-[13px] font-medium`.
                .when(f == Field::AccentHex, |d| {
                    d.h(px(36.))
                        .px(px(12.))
                        .rounded(px(crate::glass_ui::rad(radius::CONTROL)))
                        .text_size(px(13.))
                        .font_weight(FontWeight::MEDIUM)
                })
                // The friend field sits in its chip, on the chip's fill.
                .when(matches!(f, Field::Players), |d| {
                    d.h_full().px(px(14.)).bg(gpui::transparent_black())
                })
                // The server's command line: the field is the line's middle, on its fill, in mono.
                .when(f == Field::Command, |d| {
                    d.h_full()
                        .px_0()
                        .rounded_none()
                        .bg(gpui::transparent_black())
                        .font_family(theme::MONO)
                        .text_size(px(13.))
                })
        } else {
            field_box(focused, invalid)
        };
        self.field_on(f, placeholder, width, base, cx)
    }

    /// `f`'s text field drawn in `base` (its frame): focus, typing, clicks and keys.
    pub(super) fn field_on(
        &mut self,
        f: Field,
        placeholder: &'static str,
        width: f32,
        base: Div,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = self.theme;
        let handle = self.focus_handle(f, cx);
        let focused = self.servers.focused == Some(f);
        let selected = focused && self.servers.selected == Some(f);
        self.servers.text(f).show(((width - 24.) / 7.5) as usize);
        let profile_field = matches!(
            f,
            Field::InstanceName | Field::InstanceGroup | Field::ImportSearch
        );
        let line_theme = if profile_field {
            crate::mods::field_theme(t)
        } else {
            t
        };
        let field = base
            .id(SharedString::from(format!("field-{f:?}")))
            .track_focus(&handle)
            .seen()
            .w(px(width))
            .cursor_text()
            .child(line_edit::content(
                self.servers.text(f),
                &line_theme,
                focused,
                selected,
                f == Field::Token,
                placeholder,
            ))
            .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                // A click puts the caret in: the suggestion is no longer selected.
                if matches!(ev, ClickEvent::Mouse(_)) {
                    this.servers.selected = None;
                }
                if let Some(h) = this.servers.focus.get(&f) {
                    window.focus(h, cx);
                }
                cx.notify();
            }))
            .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, window, cx| {
                if ev.keystroke.key != "enter" && this.servers.invalid.is_some_and(|(g, _)| g == f)
                {
                    this.servers.invalid = None;
                }
                // A suggestion is selected: what's typed or pasted replaces it.
                let selected = this.servers.selected.take() == Some(f);
                if selected {
                    this.servers.text(f).select_all();
                }
                match line_edit::handle(this.servers.text(f), ev, cx) {
                    Key::Enter if matches!(f, Field::InstanceName | Field::InstanceGroup) => {
                        this.save_instance(cx)
                    }
                    Key::Enter => this.field_enter(f),
                    Key::Escape if f == Field::NewGroup => this.close_new_group(),
                    Key::Escape => {
                        this.servers.confirm_delete = false;
                        if this.servers.create.is_some() && !this.servers.creating {
                            this.servers.create = None;
                        }
                        this.instance_escape();
                        window.blur(cx);
                    }
                    // Tab and the rest go on to the window: Tab moves the focus.
                    Key::Pass => return,
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
            }));
        line_edit::mouse(field, cx, move |this| this.servers.text(f))
    }

    pub(super) fn field_enter(&mut self, f: Field) {
        match f {
            // The glass dialog's friend field adds the name typed.
            Field::Players if self.theme.glass => self.add_create_friend(),
            Field::Name | Field::Seed | Field::Players => self.create_server(),
            Field::Command => {
                let line = self.servers.text(Field::Command).take();
                let line = line.trim().trim_start_matches('/').to_string();
                if !line.is_empty() {
                    self.api_command(line);
                }
            }
            Field::AddPlayer => self.add_player(),
            Field::Motd => {
                let motd = self.servers.value(Field::Motd).trim().to_string();
                if motd.is_empty() {
                    self.servers.problem = Some(Problem::plain(tr(
                        "Type the line players see under the name.",
                    )));
                } else {
                    self.set_setting("server.motd", motd);
                }
            }
            Field::Port => self.save_port(),
            Field::Token => self.save_token(),
            Field::Email => self.invite_friend(),
            Field::NewGroup => self.commit_new_group(),
            Field::InstanceName
            | Field::InstanceGroup
            | Field::ImportSearch
            | Field::DeleteServer
            | Field::AccentHex => {}
            Field::PreLaunch | Field::Wrapper | Field::LaunchEnv | Field::PostExit => {
                self.save_launch()
            }
        }
    }

    fn save_port(&mut self) {
        match self.servers.value(Field::Port).trim().parse::<u16>() {
            Ok(port) if port >= 1024 => self.set_setting("server.port", port.to_string()),
            _ => {
                self.servers.invalid = Some((
                    Field::Port,
                    "Use a number from 1024 to 65535 (25565 is Minecraft's own).",
                ))
            }
        }
    }

    pub(super) fn add_player(&mut self) {
        let name = self.servers.value(Field::AddPlayer).trim().to_string();
        let valid = (3..=16).contains(&name.len())
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !valid {
            self.servers.invalid = Some((
                Field::AddPlayer,
                "A Minecraft name has 3 to 16 letters, digits or underscores.",
            ));
            return;
        }
        self.servers.text(Field::AddPlayer).set(String::new());
        self.server_command(
            &trf("Letting {name} in", &[("name", &name)]),
            vec!["whitelist".into(), "add".into(), name],
        );
    }

    /// A player's face, from the default skin their UUID picks (the server knows no other).
    pub(super) fn head(&mut self, uuid: &str, size: f32) -> Div {
        let resources = servers::game_files(&servers::runtime_root(), "26.3");
        let face = self
            .servers
            .faces
            .entry(uuid.to_string())
            .or_insert_with(|| heads::default_face(&resources, uuid))
            .clone();
        let cell = size / 8.;
        let mut d = div()
            .flex_none()
            .flex()
            .flex_col()
            .size(px(size))
            .rounded(px(if self.theme.glass {
                crate::glass_ui::rad(size / 4.).min(size / 2.)
            } else {
                3.
            }))
            .overflow_hidden()
            .bg(self.theme.plate);
        if let Some(pixels) = face {
            for row in pixels.chunks(8) {
                d = d.child(div().flex().children(row.iter().map(|p| {
                    div().size(px(cell)).bg(Hsla::from(gpui::Rgba {
                        r: p[0] as f32 / 255.,
                        g: p[1] as f32 / 255.,
                        b: p[2] as f32 / 255.,
                        a: 1.,
                    }))
                })));
            }
        }
        d
    }
}
