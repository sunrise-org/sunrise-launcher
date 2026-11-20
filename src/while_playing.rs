//! What the launcher does while the game runs (`Settings::keep_open_while_playing`): it stays
//! open, or (the default) it closes once the game has started, the way closing its window does
//! (`shut_down`). The game runs on without it: it is its own process in its own process group,
//! writes its own `latest.log`, reads its account from the session file and refreshes it itself.
use super::*;
use std::path::PathBuf;

/// How long the game has to have run before its start counts as successful: a game that fails
/// at once is still reported here, in the window that started it.
const STARTED: Duration = Duration::from_secs(10);
/// How often a watched start is looked at.
const LOOK: Duration = Duration::from_millis(500);

#[derive(Default)]
pub(super) struct Aside {
    /// Counts the starts watched: a watch whose game ended or was followed by another one stops.
    watch: u64,
}

impl Launcher {
    /// Whether the launcher stays open for the game about to start. Read from `settings.json`,
    /// so `sunrise-launcher settings keep-open-while-playing …` counts at once.
    pub(super) fn keep_open_while_playing(&mut self) -> bool {
        // Keep a choice whose save failed available for Retry instead of replacing it from disk.
        if self.settings_save_error.is_none() {
            self.settings.keep_open_while_playing = Settings::load().keep_open_while_playing;
        }
        self.settings.keep_open_while_playing
    }

    /// The game started, with its output in its own file (`backend::spawn_logged`): the launcher
    /// closes once the game has run for [`STARTED`] and has read its session file (`session`),
    /// so no token is left on disk.
    pub(super) fn step_aside(&mut self, session: Option<PathBuf>, cx: &mut Context<Self>) {
        self.aside.watch += 1;
        let (watch, since) = (self.aside.watch, Instant::now());
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(LOOK).await;
            let Ok(watching) = this.update(cx, |this, cx| {
                // The game ended (its error is shown as before), or another one started.
                if this.aside.watch != watch || this.client.is_none() {
                    return false;
                }
                let read = session.as_ref().is_none_or(|file| !file.exists());
                // Closing now would stop work in progress, or a server this launcher hosts.
                let free = !this.launch_busy() && !this.hosting_here();
                if since.elapsed() >= STARTED && read && free {
                    shut_down(cx);
                    return false;
                }
                true
            }) else {
                break;
            };
            if !watching {
                break;
            }
        })
        .detach();
    }
}
