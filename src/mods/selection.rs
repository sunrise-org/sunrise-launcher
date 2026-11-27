//! Ticking several rows of a list at once: which rows are ticked, how a click, Ctrl/Cmd-click
//! or Shift-click changes that, and the bar that offers what to do with them. The Home tabs,
//! Installed and the pack shelves share it; each owns its own [`Selection`] and its own actions.
use std::collections::BTreeSet;

mod actions;
pub(crate) mod bar;

pub(crate) use actions::{ModOp, PackOp};

#[cfg(test)]
mod tests;

/// How a press on a row was meant, from the keys held with it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Gesture {
    /// Shift: everything from the last row pressed to this one.
    pub(crate) range: bool,
    /// Ctrl (Cmd on a Mac): this row alone, in or out of the selection.
    pub(crate) toggle: bool,
}

impl Gesture {
    pub(crate) fn of(keys: &gpui::Modifiers) -> Self {
        Self {
            range: keys.shift,
            toggle: keys.control || keys.platform,
        }
    }

    /// A press that selects, where a plain one would open the row.
    pub(crate) fn selects(self) -> bool {
        self.range || self.toggle
    }
}

/// The ticked rows of one list, by the key each row has (a mod's id, a pack's file).
#[derive(Clone, Debug, Default)]
pub(crate) struct Selection {
    chosen: BTreeSet<String>,
    /// The row pressed last, where a Shift range starts.
    anchor: Option<String>,
    /// The selection a Remove was asked for: a second press on it confirms. Any change to the
    /// selection makes it stale, so it is never cleared by hand.
    armed: Option<BTreeSet<String>>,
}

impl Selection {
    pub(crate) fn len(&self) -> usize {
        self.chosen.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.chosen.is_empty()
    }

    pub(crate) fn contains(&self, key: &str) -> bool {
        self.chosen.contains(key)
    }

    pub(crate) fn keys(&self) -> &BTreeSet<String> {
        &self.chosen
    }

    /// How many of the rows in `order` are ticked.
    pub(crate) fn count_in(&self, order: &[String]) -> usize {
        order
            .iter()
            .filter(|key| self.chosen.contains(*key))
            .count()
    }

    /// Whether every row in `order` is ticked (and there is one).
    pub(crate) fn all_of(&self, order: &[String]) -> bool {
        !order.is_empty() && self.count_in(order) == order.len()
    }

    /// A press on `key`, among the rows `order` shows. With `range` and a row pressed before
    /// that is still shown, everything between them joins; otherwise the row flips.
    pub(crate) fn click(&mut self, order: &[String], key: &str, range: bool) {
        let Some(at) = order.iter().position(|shown| shown == key) else {
            return;
        };
        let from = self
            .anchor
            .as_ref()
            .filter(|_| range)
            .and_then(|anchor| order.iter().position(|shown| shown == anchor));
        match from {
            Some(from) => self
                .chosen
                .extend(order[from.min(at)..=from.max(at)].iter().cloned()),
            None => {
                if !self.chosen.remove(key) {
                    self.chosen.insert(key.to_owned());
                }
            }
        }
        self.anchor = Some(key.to_owned());
    }

    /// Every row shown, and no others: what Ctrl+A makes of it.
    pub(crate) fn select_all(&mut self, order: &[String]) {
        self.chosen = order.iter().cloned().collect();
        self.anchor = None;
    }

    /// The "Select all" / "Clear" toggle: everything shown, or nothing when all of it is ticked.
    pub(crate) fn toggle_all(&mut self, order: &[String]) {
        if self.all_of(order) {
            self.clear();
        } else {
            self.select_all(order);
        }
    }

    pub(crate) fn clear(&mut self) {
        self.chosen.clear();
        self.anchor = None;
    }

    /// Drops what the list no longer has: a row removed, or a profile left.
    pub(crate) fn retain(&mut self, order: &[String]) {
        let before = self.chosen.len();
        self.chosen.retain(|key| order.contains(key));
        if self.chosen.len() != before {
            self.anchor = None;
        }
    }

    /// Asks for the selection to be removed: true when this is the second press on the same
    /// selection, so the removal is confirmed.
    pub(crate) fn confirm_removal(&mut self) -> bool {
        if self.armed.as_ref() == Some(&self.chosen) {
            self.armed = None;
            return true;
        }
        self.armed = Some(self.chosen.clone());
        false
    }

    /// Whether a Remove is waiting for its confirmation.
    pub(crate) fn removal_armed(&self) -> bool {
        !self.chosen.is_empty() && self.armed.as_ref() == Some(&self.chosen)
    }
}
