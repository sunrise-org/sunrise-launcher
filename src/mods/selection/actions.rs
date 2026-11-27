//! What the selection bar does: a change to every ticked mod or pack as one background job with
//! one toast, through the same actions and locks a single row uses. Home shows no notice line,
//! so a change started there says its result as a toast.
use super::super::*;
use std::path::Path;
use sunrise_launcher::packs::{self, Kind as PackKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ModOp {
    Enable,
    Disable,
    Update,
    Remove,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PackOp {
    Enable,
    Disable,
    Remove,
}

/// What the ticked mods allow, so the bar offers only what applies.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ModFacts {
    /// Installed has been read: until it has, nothing is offered.
    pub(crate) ready: bool,
    /// Ticked mods that are on.
    pub(crate) on: usize,
    /// Ticked registry mods that are off (a local one is turned on from its own row).
    pub(crate) off: usize,
    /// Ticked mods a bulk update would take.
    pub(crate) updates: usize,
    /// Ticked mods with a package that can be removed.
    pub(crate) removable: usize,
}

impl Launcher {
    /// What the ticked mods `ids` allow. Asking also keeps Installed's read of the profile
    /// current, which the actions below work from.
    pub(crate) fn mod_selection_facts(&mut self, ids: &BTreeSet<String>) -> ModFacts {
        self.mods_refresh();
        let ready = self
            .mods_state
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.ready());
        if !ready {
            return ModFacts::default();
        }
        let mut facts = ModFacts {
            ready,
            ..ModFacts::default()
        };
        for row in self.rows().iter().filter(|row| ids.contains(&row.id)) {
            if row.on {
                facts.on += 1;
            } else if row.local.is_none() {
                facts.off += 1;
            }
            if model::bulk_updatable(row) {
                facts.updates += 1;
            }
            if row.selectable_archive() {
                facts.removable += 1;
            }
        }
        facts
    }

    /// Does `op` to the mods `ids` in one change. The result shows on the Mods pages' notice
    /// line, or as a toast when it was asked from Home.
    pub(crate) fn run_mod_op(&mut self, op: ModOp, ids: &BTreeSet<String>) {
        self.mods_state.bulk_toast = self.ui.page == Page::Home;
        match op {
            ModOp::Disable => {
                let on: Vec<String> = self
                    .rows()
                    .into_iter()
                    .filter(|row| row.on && ids.contains(&row.id))
                    .map(|row| row.id)
                    .collect();
                if !on.is_empty() {
                    self.turn_off(on);
                }
            }
            ModOp::Enable | ModOp::Update => {
                // The bulk actions read Installed's own selection: lend it ours for the call.
                let kept = std::mem::replace(&mut self.mods_state.selection, ids.clone());
                if op == ModOp::Enable {
                    self.bulk_on();
                } else {
                    self.ask_update(true);
                }
                self.mods_state.selection = kept;
            }
            ModOp::Remove => {
                let home = self.mods_home();
                let (rows, labels) = self.labeled_installed_rows();
                let names = installed_view::remove_names(&rows, &labels);
                let chosen: Vec<String> = rows
                    .iter()
                    .filter(|row| row.selectable_archive() && ids.contains(&row.id))
                    .map(|row| row.id.clone())
                    .collect();
                self.ask_remove(&home, chosen, names);
            }
        }
    }

    /// Says what a change started from Home did, once it has ended. True when it said it.
    pub(in crate::mods) fn say_bulk_result(&mut self) -> bool {
        if !self.mods_state.bulk_toast {
            return false;
        }
        if self.ui.page != Page::Home {
            self.mods_state.bulk_toast = false;
            return false;
        }
        if self.mods_state.busy.is_some()
            || self.registry_stage().is_some()
            || self.mods.installs.question.is_some()
            || self.mods.installs.world.is_some()
        {
            return false;
        }
        let Some(notice) = self.notice() else {
            return false;
        };
        self.mods_state.bulk_toast = false;
        self.dismiss_notice();
        // A switch flipped on a card shows its own result: only a failure is told.
        let quiet = std::mem::take(&mut self.ui.home.tabs.quiet);
        match notice {
            Ok(_) if quiet => {}
            Ok(text) | Err(text) => crate::toast::say(text),
        }
        true
    }

    /// Why packs can't be changed now, if they can't: the game or a change is running, or the
    /// selected profile isn't `home` any more.
    pub(crate) fn pack_refusal(&self, home: &Path) -> Option<&'static str> {
        if let Some(why) = self.mods_locked() {
            Some(why)
        } else if self.mutating() || self.location_unavailable {
            Some(tr("Wait for the current operation to finish."))
        } else if !self
            .selected_instance()
            .is_some_and(|instance| instance.home == home)
        {
            Some(tr("The selected profile changed. Try again."))
        } else {
            None
        }
    }

    /// Does `op` to the packs `files` of `kind` in one background job, and says the result
    /// in one toast. Refused, with the reason said, while the game or a change is running.
    pub(crate) fn run_pack_op(
        &mut self,
        home: PathBuf,
        kind: PackKind,
        files: Vec<String>,
        op: PackOp,
        cx: &mut Context<Self>,
    ) {
        self.run_pack_change(home, kind, files, op, true, cx);
    }

    /// [`Self::run_pack_op`], with `say_done` off for a change the screen already shows (a
    /// switch flipped on a card): only a refusal or a failure is told.
    pub(crate) fn run_pack_change(
        &mut self,
        home: PathBuf,
        kind: PackKind,
        files: Vec<String>,
        op: PackOp,
        say_done: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(why) = self.pack_refusal(&home) {
            crate::toast::say(why);
            return;
        }
        self.mods_state.busy = Some((tr("Working…").to_string(), 0.));
        let task = cx
            .background_executor()
            .spawn(async move { change_packs(&home, kind, &files, op) });
        cx.spawn(async move |this, cx| {
            let said = task.await;
            let _ = this.update(cx, |this, cx| {
                this.mods_state.busy = None;
                this.game_packs.stale();
                this.home_inventory_changed();
                match said {
                    Ok(_) if !say_done => {}
                    Ok(text) | Err(text) => crate::toast::say(text),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Add file on the Packs tab: picks pack files and copies them into the profile's
    /// `resourcepacks/`, as the shelf's "Import from file" does, and says the result in a toast.
    pub(crate) fn add_pack_files(&mut self, cx: &mut Context<Self>) {
        let Some(home) = self
            .selected_instance()
            .map(|instance| instance.home.clone())
        else {
            return;
        };
        if let Some(why) = self.pack_refusal(&home) {
            crate::toast::say(why);
            return;
        }
        let pick = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(tr("Import from file").into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = pick.await {
                let _ = this.update(cx, |this, cx| this.add_pack_paths(home, paths, cx));
            }
        })
        .detach();
    }

    /// Copies the picked `paths` in the background, under the profile's lock. The dialog may
    /// outlive the moment it was asked in, so the profile is checked again.
    fn add_pack_paths(&mut self, home: PathBuf, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        if paths.is_empty() || self.pack_refusal(&home).is_some() {
            return;
        }
        self.mods_state.busy = Some((tr("Importing packs…").to_string(), 0.));
        let task = cx
            .background_executor()
            .spawn(async move { import_packs(&home, &paths) });
        cx.spawn(async move |this, cx| {
            let said = task.await;
            let _ = this.update(cx, |this, cx| {
                this.mods_state.busy = None;
                this.game_packs.stale();
                this.home_inventory_changed();
                crate::toast::say(said.unwrap_or_else(|text| text));
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The ids of the rows Installed shows now (its filters and search applied) that can be
    /// ticked.
    fn installed_shown_ids(&self) -> Vec<String> {
        let (rows, _) = self.labeled_installed_rows();
        let visible = model::visible(&rows, &self.mods_state.row_filter);
        visible
            .iter()
            .filter_map(|&i| {
                let row = &rows[i];
                (row.archive_file().is_some() && row.selectable_archive()).then(|| row.id.clone())
            })
            .collect()
    }

    /// The files of the packs the shelf shows now (its search applied).
    pub(crate) fn shown_pack_files(&self, kind: PackKind) -> Vec<String> {
        let query = self.mods_state.have.text().trim().to_lowercase();
        let matches = |pack: &packs::Pack| {
            query.is_empty()
                || pack.title.to_lowercase().contains(&query)
                || pack.file.to_lowercase().contains(&query)
        };
        match kind {
            PackKind::Resource => self
                .game_packs
                .resource
                .iter()
                .map(|(pack, _)| pack)
                .filter(|pack| matches(pack))
                .map(|pack| pack.file.clone())
                .collect(),
            PackKind::Shader => self
                .game_packs
                .shader
                .iter()
                .filter(|pack| matches(pack))
                .map(|pack| pack.file.clone())
                .collect(),
        }
    }

    /// Ctrl/Cmd+A and Escape on a list with rows to tick. True when it took the key.
    pub(crate) fn selection_key(&mut self, ev: &KeyDownEvent, window: &Window) -> bool {
        if self.layer_open() {
            return false;
        }
        let keys = &ev.keystroke;
        let command = if cfg!(target_os = "macos") {
            keys.modifiers.platform
        } else {
            keys.modifiers.control
        };
        match keys.key.as_str() {
            "escape" => self.untick_all(),
            "a" if command && !keys.modifiers.shift && !keys.modifiers.alt => {
                // In a search box it selects the text.
                let typing = [&self.mods_state.have_focus, &self.mods_state.find_focus]
                    .into_iter()
                    .flatten()
                    .any(|focus| focus.is_focused(window));
                !typing && self.tick_all()
            }
            _ => false,
        }
    }

    fn untick_all(&mut self) -> bool {
        match self.ui.page {
            Page::Installed if !self.mods_state.selection.is_empty() => {
                self.mods_state.selection.clear();
                self.mods_state.anchor = None;
                true
            }
            Page::Packs if !self.mods_state.pack_selection.is_empty() => {
                self.mods_state.pack_selection.clear();
                true
            }
            Page::Home => {
                // Escape shuts a card's menu before it clears the ticks under it.
                if self.close_item_menu() {
                    return true;
                }
                let Some(packs) = self.home_pick_scope() else {
                    return false;
                };
                let picks = self.home_pick(packs);
                let any = !picks.is_empty();
                picks.clear();
                any
            }
            _ => false,
        }
    }

    fn tick_all(&mut self) -> bool {
        match self.ui.page {
            Page::Installed => {
                let ids = self.installed_shown_ids();
                if ids.is_empty() {
                    return false;
                }
                self.mods_state.selection = ids.into_iter().collect();
                self.mods_state.anchor = None;
                true
            }
            Page::Packs => {
                let Some(kind) = self.mods_state.shelf.kind() else {
                    return false;
                };
                let files = self.shown_pack_files(kind);
                self.mods_state.pack_selection.select_all(&files);
                !files.is_empty()
            }
            Page::Home => {
                let Some(packs) = self.home_pick_scope() else {
                    return false;
                };
                let ids = self.home_list_ids(packs);
                self.home_pick(packs).select_all(&ids);
                !ids.is_empty()
            }
            _ => false,
        }
    }
}

/// `one` for a count of 1, else `many` with its `{count}` filled in.
fn counted(count: usize, one: &'static str, many: &'static str) -> String {
    match count {
        1 => tr(one).to_string(),
        n => trf(many, &[("count", &n.to_string())]),
    }
}

/// Copies the pack files `paths` into the profile's resource pack folder, under the profile's
/// lock, and says what happened. A file that fails doesn't stop the rest.
fn import_packs(home: &Path, paths: &[PathBuf]) -> Result<String, String> {
    let _lock = sunrise_launcher::browser::try_lock_home(home).ok_or_else(|| {
        tr("This profile is busy. Try again when the operation finishes.").to_string()
    })?;
    let mut added = Vec::new();
    let mut refused = Vec::new();
    for path in paths {
        match packs::add_local(home, PackKind::Resource, path) {
            Ok(pack) => added.push(pack.title),
            Err(why) => refused.push(why),
        }
    }
    let mut parts = Vec::new();
    if !added.is_empty() {
        parts.push(trf("Imported {name}.", &[("name", &added.join(", "))]));
    }
    if refused.is_empty() {
        return Ok(parts.join(" "));
    }
    parts.extend(refused);
    Err(parts.join("\n"))
}

/// Changes the packs one after another under one hold of the profile, and says what happened.
/// A pack that is already as asked is left alone; one that fails doesn't stop the rest.
fn change_packs(
    home: &Path,
    kind: PackKind,
    files: &[String],
    op: PackOp,
) -> Result<String, String> {
    let _lock = sunrise_launcher::browser::try_lock_home(home).ok_or_else(|| {
        tr("This profile is busy. Try again when the operation finishes.").to_string()
    })?;
    let mut changed = 0;
    let mut failed = Vec::new();
    for file in files {
        match change_pack(home, kind, file, op) {
            Ok(true) => changed += 1,
            Ok(false) => {}
            Err(why) => failed.push(format!("{file}: {why}")),
        }
    }
    let said = match (op, changed) {
        (_, 0) if failed.is_empty() => tr("Nothing needed changing.").to_string(),
        (_, 0) => String::new(),
        (PackOp::Enable, n) => counted(n, "Turned on 1 pack.", "Turned on {count} packs."),
        (PackOp::Disable, n) => counted(n, "Turned off 1 pack.", "Turned off {count} packs."),
        (PackOp::Remove, n) => counted(n, "Removed 1 pack.", "Removed {count} packs."),
    };
    if failed.is_empty() {
        return Ok(said);
    }
    let list = failed.join("; ");
    Err(
        format!("{said} {}", trf("Not done: {list}", &[("list", &list)]))
            .trim()
            .to_owned(),
    )
}

/// One pack changed as asked: true when it changed, false when it already was as asked.
fn change_pack(home: &Path, kind: PackKind, file: &str, op: PackOp) -> Result<bool, String> {
    let pack = packs::find(home, kind, file)?;
    if pack.file != file {
        return Err(tr("This pack changed. Check the refreshed list.").into());
    }
    match op {
        PackOp::Enable | PackOp::Disable => {
            let on = op == PackOp::Enable;
            if pack.enabled == on {
                return Ok(false);
            }
            packs::set_enabled(home, &pack, on)?;
        }
        PackOp::Remove => {
            packs::remove(home, &pack)?;
            // Otherwise Discover thinks this version is still installed and will not
            // download it again. The caller holds the profile's lock.
            let mut kept = sunrise_launcher::browser::installed::Installed::load(home);
            let before = kept.entries.len();
            kept.entries
                .retain(|entry| entry.path(home).as_deref() != Some(&pack.path));
            if kept.entries.len() != before {
                kept.save(home)?;
            }
        }
    }
    Ok(true)
}
