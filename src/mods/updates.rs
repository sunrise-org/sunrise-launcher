//! Check for updates on Home's Mods tab: ask the registry again, list the installed mods that
//! have a newer release, and install the ones the player keeps ticked as one change.
//!
//! What counts as an update is the row's own (the newest release fitting the build played, a mod
//! whose version the player chose left to its own page), so the list and the card's Update
//! agree. The pure parts, the list and the skipped versions, are here and tested; the worker and
//! the sheet use them.
use super::*;
use crate::home::tabs::Collection;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

mod packs;
mod skips;
pub(crate) use skips::Skips;

#[cfg(test)]
mod tests;

/// How long "Up to date" stays on the button after a check that found nothing.
pub(crate) const UP_TO_DATE_FOR: Duration = Duration::from_secs(4);

/// One installed mod with a newer release, as the sheet lists it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Candidate {
    pub(crate) id: String,
    pub(crate) name: String,
    /// Its project in the registry, for its icon and its page.
    pub(crate) project: Option<String>,
    pub(crate) current: String,
    pub(crate) next: String,
    /// The download's size in bytes, when the registry says.
    pub(crate) size: Option<u64>,
    /// Why it can't be ticked: a bulk update leaves it alone.
    pub(crate) blocked: Option<String>,
}

/// What the check reads of one installed mod.
#[derive(Clone, Debug, Default)]
pub(crate) struct Found {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) project: Option<String>,
    pub(crate) current: String,
    pub(crate) update: Option<String>,
    /// The player chose this version: its update waits for its own page.
    pub(crate) pinned: bool,
    /// Not from the registry: nothing to update to.
    pub(crate) local: bool,
    /// The registry took it down.
    pub(crate) taken_down: bool,
    pub(crate) size: Option<u64>,
}

/// What a check found: the mods to offer, and how many more the player skipped.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Offer {
    pub(crate) items: Vec<Candidate>,
    pub(crate) hidden: usize,
}

/// The mods with an update the player has not skipped, by name.
pub(crate) fn offer(found: Vec<Found>, skips: &Skips) -> Offer {
    let mut offer = Offer::default();
    for mod_ in found.into_iter().filter(|found| !found.local) {
        let Some(next) = mod_.update else { continue };
        if skips.is_skipped(&mod_.id, &next) {
            offer.hidden += 1;
            continue;
        }
        let blocked = if mod_.taken_down {
            Some(tr("The registry took it down.").to_owned())
        } else if mod_.pinned {
            Some(tr("You chose this version. Update it from its own page.").to_owned())
        } else {
            None
        };
        offer.items.push(Candidate {
            id: mod_.id,
            name: mod_.name,
            project: mod_.project,
            current: mod_.current,
            next,
            size: mod_.size,
            blocked,
        });
    }
    offer
        .items
        .sort_by_key(|item| (item.name.to_lowercase(), item.id.clone()));
    offer
}

/// What is ticked when the sheet opens: every mod that can be.
pub(crate) fn default_picks(items: &[Candidate]) -> BTreeSet<String> {
    items
        .iter()
        .filter(|item| item.blocked.is_none())
        .map(|item| item.id.clone())
        .collect()
}

/// Where the check is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Phase {
    #[default]
    Idle,
    Checking,
    /// A check found nothing, when: the button says so for a few seconds.
    UpToDate(Instant),
}

/// The check and the sheet it opens: what was found and what is ticked.
#[derive(Default)]
pub(crate) struct Check {
    pub(crate) phase: Phase,
    /// Counts checks, so the answer of one that was left behind is dropped.
    generation: u64,
    pub(crate) items: Vec<Candidate>,
    pub(crate) picks: BTreeSet<String>,
    /// Updates the player skipped that this check left out.
    pub(crate) hidden: usize,
    /// Why the last check couldn't ask the registry.
    pub(crate) failure: Option<String>,
    /// The kind of pack the check is of (Packs or Shaders tab); None for the mods.
    pub(crate) scope: Option<Kind>,
}

impl Check {
    pub(crate) fn is_checking(&self) -> bool {
        self.phase == Phase::Checking
    }

    /// Whether the check under way, or the last answer, is for the tab of `scope`.
    pub(crate) fn is_for(&self, scope: Option<Kind>) -> bool {
        self.scope == scope
    }

    /// Whether the button still says "Up to date" at `now`.
    pub(crate) fn up_to_date(&self, now: Instant) -> bool {
        matches!(self.phase, Phase::UpToDate(at) if now.saturating_duration_since(at) < UP_TO_DATE_FOR)
    }

    pub(crate) fn begin(&mut self) -> u64 {
        self.generation += 1;
        self.phase = Phase::Checking;
        self.failure = None;
        self.generation
    }

    /// Takes an answer if it is the latest check's.
    pub(crate) fn accepts(&self, generation: u64) -> bool {
        self.generation == generation && self.is_checking()
    }

    /// Shows what a check found: the sheet's list with every mod that can be ticked ticked.
    pub(crate) fn show(&mut self, offer: Offer, now: Instant) {
        self.phase = if offer.items.is_empty() && offer.hidden == 0 {
            Phase::UpToDate(now)
        } else {
            Phase::Idle
        };
        self.picks = default_picks(&offer.items);
        self.items = offer.items;
        self.hidden = offer.hidden;
    }

    pub(crate) fn toggle(&mut self, id: &str) {
        let Some(item) = self.items.iter().find(|item| item.id == id) else {
            return;
        };
        if item.blocked.is_some() {
            return;
        }
        if !self.picks.remove(id) {
            self.picks.insert(id.to_owned());
        }
    }

    pub(crate) fn select_all(&mut self) {
        self.picks = default_picks(&self.items);
    }

    pub(crate) fn select_none(&mut self) {
        self.picks.clear();
    }

    /// Drops `id` from the list, as a skipped version is.
    pub(crate) fn drop_item(&mut self, id: &str) {
        self.items.retain(|item| item.id != id);
        self.picks.remove(id);
        self.hidden += 1;
    }

    /// How many of the list can be ticked at all.
    pub(crate) fn tickable(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.blocked.is_none())
            .count()
    }
}

/// "12.4 MB" for a download's size.
pub(crate) fn size_label(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024. && unit < UNITS.len() - 1 {
        value /= 1024.;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// The kind of pack the tab of `collection` lists; None for the mods.
pub(crate) fn pack_scope(collection: Collection) -> Option<Kind> {
    match collection {
        Collection::Mods => None,
        Collection::Resource => Some(Kind::ResourcePack),
        Collection::Shader => Some(Kind::Shader),
    }
}

impl Launcher {
    /// Where the skipped versions are kept: the launcher's data folder, for every profile.
    fn skips_dir() -> PathBuf {
        backend::data_dir()
    }

    /// What each installed registry mod with a newer release reads as, from memory.
    fn found_updates(&self) -> Vec<Found> {
        let index = sunrise::v2::shown_index();
        self.rows()
            .into_iter()
            .map(|row| {
                let project = row.entry.as_ref().map(|entry| entry.project.clone());
                let size = row.update.as_ref().and_then(|next| {
                    let project = index.as_ref()?.index.project(project.as_deref()?)?;
                    Some(
                        project
                            .releases
                            .iter()
                            .find(|r| &r.version == next)?
                            .file
                            .size,
                    )
                });
                Found {
                    taken_down: matches!(row.need, Some(model::Need::TakenDown(_))),
                    local: row.local.is_some(),
                    pinned: row.pinned,
                    id: row.id,
                    name: row.title,
                    project,
                    current: row.version,
                    update: row.update,
                    size,
                }
            })
            .collect()
    }

    /// "Check for updates" on the tab of `collection`: for mods it asks the registry again, for
    /// packs it asks Modrinth about each pack the browser installed; in the background, then
    /// lists what has a newer release. A check left behind (another began, or the profile
    /// changed) is dropped when it answers; the fetch itself can't be interrupted.
    pub(crate) fn check_for_updates(&mut self, collection: Collection, cx: &mut Context<Self>) {
        if self.mods_state.update_check.is_checking() {
            return;
        }
        if let Some(why) = self.mods_locked() {
            crate::toast::say(why);
            return;
        }
        let scope = pack_scope(collection);
        let generation = self.mods_state.update_check.begin();
        self.mods_state.update_check.scope = scope;
        let task = match scope {
            None => cx.background_executor().spawn(async move {
                sunrise::v2::refreshed_index()
                    .map(|_| None::<Vec<Found>>)
                    .map_err(|e| e.to_string())
            }),
            Some(kind) => {
                let (home, game) = (self.mods_home(), self.mods_game());
                cx.background_executor()
                    .spawn(async move { packs::check(&home, kind, &game).map(Some) })
            }
        };
        cx.spawn(async move |this, cx| {
            let answer = task.await;
            let _ = this.update(cx, |this, cx| {
                this.finish_update_check(generation, answer, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn finish_update_check(
        &mut self,
        generation: u64,
        answer: Result<Option<Vec<Found>>, String>,
        cx: &mut Context<Self>,
    ) {
        if !self.mods_state.update_check.accepts(generation) {
            return;
        }
        let answer = answer.map(|packs| match packs {
            Some(found) => found,
            None => {
                // The rows read the catalogue just fetched: what Home drew from the old one is
                // stale.
                self.mods_state.home_mod_labels = None;
                self.ui.home.tabs.facts = Default::default();
                self.home_inventory_changed();
                self.v2_cards_again();
                self.mods_refresh();
                self.found_updates()
            }
        });
        let found = match answer {
            Ok(found) => found,
            Err(why) => {
                self.mods_state.update_check.phase = Phase::Idle;
                self.mods_state.update_check.failure = Some(why.clone());
                crate::toast::say(trf(
                    "Couldn't check for updates: {error}",
                    &[("error", &why)],
                ));
                return;
            }
        };
        let skips = Skips::load(&Self::skips_dir());
        let offer = offer(found, &skips);
        let now = Instant::now();
        let nothing = offer.items.is_empty() && offer.hidden == 0;
        self.mods_state.update_check.show(offer, now);
        if nothing {
            // The button's "Up to date" goes after a few seconds: a frame then.
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(UP_TO_DATE_FOR).await;
                let _ = this.update(cx, |_, cx| cx.notify());
            })
            .detach();
        } else {
            self.open_sheet(crate::sheets::Sheet::ModUpdates);
        }
    }

    /// Update n: the ticked mods as one change, through the same path as the selection bar's
    /// Update, which reports it once in the status strip and a toast. Packs are updated one
    /// after another in a worker, and the result is a toast.
    pub(crate) fn install_checked_updates(&mut self, cx: &mut Context<Self>) {
        let ids: BTreeSet<String> = self.mods_state.update_check.picks.clone();
        let scope = self.mods_state.update_check.scope;
        self.close_sheet();
        if ids.is_empty() {
            return;
        }
        let Some(kind) = scope else {
            self.run_mod_op(selection::ModOp::Update, &ids);
            return;
        };
        if let Some(why) = self.mods_locked() {
            crate::toast::say(why);
            return;
        }
        if self.mutating() {
            crate::toast::say(tr("Wait for the current operation to finish."));
            return;
        }
        self.mods_state.busy = Some((tr("Updating…").to_owned(), 0.));
        let (home, game) = (self.mods_home(), self.mods_game());
        let task = cx
            .background_executor()
            .spawn(async move { packs::update(&home, kind, &game, &ids) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.packs_updated(result);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The packs' update ended: the lists read again, and what happened said once.
    fn packs_updated(&mut self, result: Result<usize, String>) {
        self.mods_state.busy = None;
        self.game_packs.stale();
        self.home_inventory_changed();
        match result {
            Ok(1) => crate::toast::say(tr("Updated 1 pack.")),
            Ok(n) => crate::toast::say(trf("Updated {count} packs.", &[("count", &n.to_string())])),
            Err(why) => crate::toast::say(why),
        }
    }

    /// "Skip this version": the mod's newer version stops being offered.
    pub(crate) fn skip_checked_update(&mut self, id: &str) {
        let Some(item) = self
            .mods_state
            .update_check
            .items
            .iter()
            .find(|item| item.id == id)
        else {
            return;
        };
        let dir = Self::skips_dir();
        let mut skips = Skips::load(&dir);
        skips.skip(&item.id, &item.next);
        if let Err(why) = skips.save(&dir) {
            crate::toast::say(why);
            return;
        }
        self.mods_state.update_check.drop_item(id);
    }

    /// Offers every skipped version again, then looks at what is offered.
    pub(crate) fn clear_skipped_updates(&mut self) {
        let dir = Self::skips_dir();
        let mut skips = Skips::load(&dir);
        skips.clear();
        if let Err(why) = skips.save(&dir) {
            crate::toast::say(why);
            return;
        }
        let offer = offer(self.found_updates(), &skips);
        let now = Instant::now();
        self.mods_state.update_check.show(offer, now);
    }
}
