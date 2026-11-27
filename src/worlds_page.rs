//! The selected instance's worlds on Home (`sunrise_launcher::worlds`): each with its picture,
//! mode, version, when it was last played and its size, and a "…" menu (Data packs, Open folder,
//! Copy, Rename, Delete to the trash after a confirmation) in `actions`. Reading sizes, copying,
//! deleting and adding run in the background.
use super::*;
use std::path::PathBuf;
use sunrise_launcher::worlds::{self, World};

mod actions;
pub(crate) use actions::WorldAction;

pub(super) struct Worlds {
    /// The game folder the list is of, and the list as last read: kept on screen while it is
    /// read again, so the page doesn't jump.
    pub(super) home: Option<PathBuf>,
    pub(super) list: Vec<World>,
    /// Which of the list's rows Home's glass Worlds tab has built and measured (`cull`).
    pub(super) cull: cull::Cull,
    pub(super) fresh: bool,
    reading: bool,
    /// Home's tiles: the focus of each world's picture, by folder, so what sits on the picture
    /// can show while the keyboard is on it.
    pub(super) tile_focus: std::collections::HashMap<String, FocusHandle>,
    /// The "…" menu or the dialog open over the list.
    pub(super) action: Option<WorldAction>,
    /// The name typed in the rename dialog.
    pub(super) rename: crate::line_edit::LineEdit,
}

impl Default for Worlds {
    fn default() -> Self {
        Self {
            home: None,
            list: Vec::new(),
            cull: cull::Cull::default(),
            fresh: false,
            reading: false,
            tile_focus: std::collections::HashMap::new(),
            action: None,
            rename: crate::line_edit::LineEdit::new(String::new(), 128, 32),
        }
    }
}

impl Worlds {
    /// Read again on the next frame: the game may have saved, or an action changed the list.
    pub(super) fn stale(&mut self) {
        self.fresh = false;
    }
}

/// "today", "yesterday", "3 days ago", else the date.
pub(super) fn ago(when: std::time::SystemTime) -> String {
    let when: chrono::DateTime<chrono::Local> = when.into();
    let days = (chrono::Local::now().date_naive() - when.date_naive()).num_days();
    match days {
        ..=0 => tr("today").into(),
        1 => tr("yesterday").into(),
        2..=13 => trf("{days} days ago", &[("days", &days.to_string())]),
        _ => when.format("%Y-%m-%d").to_string(),
    }
}

impl Launcher {
    /// Reads the worlds of `home` in the background when the list is missing or of another
    /// instance.
    pub(super) fn load_worlds(&mut self, home: PathBuf, cx: &mut Context<Self>) {
        if self.worlds.home.as_ref() != Some(&home) {
            self.worlds = Worlds {
                home: Some(home.clone()),
                ..Worlds::default()
            };
        }
        if self.worlds.fresh || self.worlds.reading {
            return;
        }
        self.worlds.reading = true;
        let read = home.clone();
        let task = cx
            .background_executor()
            .spawn(async move { worlds::list(&read) });
        cx.spawn(async move |this, cx| {
            let list = task.await;
            let _ = this.update(cx, |this, cx| {
                // Another instance may have been picked meanwhile.
                if this.worlds.home.as_ref() == Some(&home) {
                    this.worlds
                        .tile_focus
                        .retain(|folder, _| list.iter().any(|w| &w.folder == folder));
                    this.worlds.list = list;
                    this.worlds.cull.reset();
                    this.worlds.fresh = true;
                    this.worlds.reading = false;
                }
                cx.notify();
            });
        })
        .detach();
    }
}
