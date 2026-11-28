//! The Logs panel: the sidebar's Logs row slides a panel up over the current
//! page with the launcher's own log (`sunrise_launcher::launcher_log`) and the game's output, so a
//! bug report can carry what happened. The page under it never changes. Every line shown, copied
//! or saved has been through `launcher_log::scrub`.

use super::*;
use crate::paint::palette;
use gpui::{rgb, ClipboardItem, ScrollHandle, StyledText};
use sunrise_launcher::launcher_log;

/// The launcher lines kept, as many as the log's own ring.
const KEEP: usize = 5000;

/// How much of the window's height the open panel takes.
const HEIGHT: f32 = 0.45;

/// What the panel lists.
#[derive(Clone, Copy, Default, PartialEq)]
enum Tab {
    #[default]
    Launcher,
    /// The running or last game's output: what `Launcher::prepare_log` holds.
    Game,
}

/// The panel's state, between frames.
#[derive(Default)]
pub(crate) struct LogsPanel {
    open: bool,
    /// The last open or close, so the panel slides from wherever it had got to.
    fade: Option<crate::motion::Fade>,
    tab: Tab,
    /// The launcher's lines read so far, and the ring's revision they come up to.
    launcher: VecDeque<String>,
    revision: u64,
    /// The game's lines, scrubbed, for the `(log_revision, length)` of `prepare_log` they were made from.
    game: ((u64, usize), Vec<String>),
    /// The player scrolled up: the list stays where it is until they scroll back to the bottom.
    paused: bool,
    /// What the list held when it was last drawn, to tell a scroll from a line arriving.
    drawn: Option<(u64, usize)>,
    scroll: ScrollHandle,
}

impl Launcher {
    /// Whether the panel is open (or opening), for the sidebar's Logs row.
    pub(crate) fn logs_open(&self) -> bool {
        self.ui.logs.open
    }

    /// Opens the panel, or closes it.
    pub(crate) fn toggle_logs(&mut self) {
        let logs = &mut self.ui.logs;
        logs.open = !logs.open;
        let mut fade = crate::motion::Fade::toward(logs.fade, logs.open, Instant::now());
        fade.motion = sunrise_launcher_theme::MOVE;
        logs.fade = Some(fade);
        logs.paused = false;
        logs.drawn = None;
    }

    /// Escape: closes the panel if it is open.
    pub(super) fn close_logs(&mut self) -> bool {
        let open = self.ui.logs.open;
        if open {
            self.toggle_logs();
        }
        open
    }

    /// From the tick, while the panel is open: reads the launcher lines logged since the last
    /// read. Whether there were any, so the window redraws only then.
    pub(super) fn pull_logs(&mut self) -> bool {
        let logs = &mut self.ui.logs;
        if !logs.open {
            return false;
        }
        let (revision, lines) = launcher_log::since(logs.revision);
        logs.revision = revision;
        let any = !lines.is_empty();
        for line in lines {
            // Again: a name hidden since the line was written is hidden here too.
            logs.launcher.push_back(launcher_log::scrub(&line));
            if logs.launcher.len() > KEEP {
                logs.launcher.pop_front();
            }
        }
        any
    }

    /// The tab's lines as one text, for Copy all and Save to file.
    fn logs_text(&self) -> String {
        let logs = &self.ui.logs;
        match logs.tab {
            Tab::Launcher => logs.launcher.iter().map(String::as_str).collect::<Vec<_>>(),
            Tab::Game => logs.game.1.iter().map(String::as_str).collect(),
        }
        .join("\n")
    }

    /// Asks where to save the tab's text, writes it there and says how it went.
    fn save_logs(&self, cx: &mut Context<Self>) {
        let text = self.logs_text();
        let name = match self.ui.logs.tab {
            Tab::Launcher => "sunrise-launcher.log",
            Tab::Game => "sunrise-game.log",
        };
        let directory = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(backend::data_dir);
        let save = cx.prompt_for_new_path(&directory, Some(name));
        cx.spawn(async move |_, cx| {
            let result = match save.await {
                Ok(Ok(Some(path))) => {
                    cx.background_executor()
                        .spawn(async move { std::fs::write(path, text).map_err(|e| e.to_string()) })
                        .await
                }
                Ok(Ok(None)) | Err(_) => return,
                Ok(Err(error)) => Err(error.to_string()),
            };
            crate::toast::say(match result {
                Ok(()) => tr("Log saved").to_string(),
                Err(reason) => trf("Couldn't save the log: {reason}", &[("reason", &reason)]),
            });
        })
        .detach();
    }

    /// The panel while it is open or sliding. It lies over the pane, so no page moves.
    pub(super) fn logs_layer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let inset = crate::tokens::FRAME_INSET;
        // The pane's left edge: the frame's inset, the sidebar and the gap after it.
        let left = inset + crate::rail::rail_width(self.rail_open()) + 12.;
        let now = Instant::now();
        let fade = self.ui.logs.fade.filter(|fade| !fade.rested(now))?;
        if fade.moving(now) {
            window.request_animation_frame();
        }

        let tab = self.ui.logs.tab;
        if tab == Tab::Game {
            let key = (self.log_revision, self.prepare_log.len());
            if self.ui.logs.game.0 != key {
                let lines = self.prepare_log.iter().map(|l| launcher_log::scrub(l));
                self.ui.logs.game = (key, lines.collect());
            }
        }
        let logs = &mut self.ui.logs;
        let lines: Vec<&str> = match tab {
            Tab::Launcher => logs.launcher.iter().map(String::as_str).collect(),
            Tab::Game => logs.game.1.iter().map(String::as_str).collect(),
        };
        // Follow the newest line until the player scrolls up, and again once they are back at the
        // bottom. Only a frame with the same lines as the last one can tell: new lines make the
        // list longer than the place it was scrolled to.
        let stamp = match tab {
            Tab::Launcher => (logs.revision, lines.len()),
            Tab::Game => logs.game.0,
        };
        if logs.drawn == Some(stamp) {
            let left_to_scroll =
                f32::from(logs.scroll.offset().y) + f32::from(logs.scroll.max_offset().y);
            logs.paused = left_to_scroll >= 1.;
        }
        logs.drawn = Some(stamp);
        if !logs.paused {
            logs.scroll.scroll_to_bottom();
        }

        let rows = lines.iter().enumerate().map(|(n, line)| {
            div().child(crate::selectable::Selectable::styled(
                ("log-line", n),
                StyledText::new(line.to_string()),
            ))
        });
        let empty = lines.is_empty().then(|| match tab {
            Tab::Launcher => tr("Nothing has been logged yet."),
            Tab::Game => tr("The game's output shows here while it runs."),
        });
        let list = div()
            .id("logs-list")
            .overflow_y_scroll()
            .px(px(14.))
            .pb(px(10.))
            .font_family(theme::MONO)
            .text_size(px(crate::tokens::SMALL))
            .line_height(px(18.))
            .text_color(rgb(palette().muted))
            .children(empty)
            .children(rows);
        let list = Scrollbar::with_handle("logs", logs.scroll.clone())
            .wrap(list, &self.theme)
            .flex_1();

        let tab_button = |id: &'static str, label: &str, this: Tab, cx: &mut Context<Self>| {
            crate::frame::button(id, label.to_string())
                .when(tab == this, |d| d.bg(rgb(palette().hover)))
                .on_click(cx.listener(move |launcher, _, _, cx| {
                    launcher.ui.logs.tab = this;
                    launcher.ui.logs.paused = false;
                    launcher.ui.logs.drawn = None;
                    cx.notify();
                }))
        };
        let folder = launcher_log::dir().map(|dir| {
            crate::frame::button("logs-folder", tr("Open logs folder"))
                .on_click(cx.listener(move |_, _, _, cx| crate::toast::open(cx, &dir)))
        });
        let header = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .p(px(10.))
            .child(tab_button(
                "logs-tab-launcher",
                tr("Launcher"),
                Tab::Launcher,
                cx,
            ))
            .child(tab_button("logs-tab-game", tr("Game"), Tab::Game, cx))
            .child(div().flex_1())
            .child(
                crate::frame::button("logs-copy", tr("Copy all")).on_click(cx.listener(
                    |this, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(this.logs_text()));
                        crate::toast::say(tr("Copied the log"));
                    },
                )),
            )
            .child(
                crate::frame::button("logs-save", tr("Save to file"))
                    .on_click(cx.listener(|this, _, _, cx| this.save_logs(cx))),
            )
            .children(folder)
            .child(
                crate::frame::button("logs-close", "×").on_click(cx.listener(|this, _, _, cx| {
                    this.close_logs();
                    cx.notify();
                })),
            );

        let height = f32::from(window.viewport_size().height) * HEIGHT;
        let panel = div()
            .id("logs-panel")
            .occlude()
            .absolute()
            .left(px(left))
            .right(px(inset))
            .bottom(px(inset))
            .h(px(height))
            .flex()
            .flex_col()
            .rounded(crate::tokens::r(crate::tokens::R_PANEL))
            .border_1()
            .border_color(rgb(palette().divider))
            .bg(rgb(palette().background))
            .overflow_hidden()
            .child(header)
            .child(list);
        // It rises from below the window's edge to rest on the inset, and sinks back the same way.
        Some(fade.play(&"logs-panel".into(), panel, move |d, p| {
            d.bottom(px(inset - height * (1. - p)))
        }))
    }
}
