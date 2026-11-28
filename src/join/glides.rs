//! How "Your servers" moves: a server moved up or down glides to its place and the one it
//! swapped with glides the other way (as the sidebar's profiles do, `widgets::glide`); a server
//! added fades and slides in; one removed leaves a ghost of its card that fades and folds shut
//! while the rest glide up. Saved servers have no ids, so a server is told by its name and
//! address.
use crate::widgets::glide::swapped;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long a note of a change is kept: its animation is over long before.
const KEPT: Duration = Duration::from_millis(1500);

/// What a card showed, kept so the ghost of a removed one can show it as it fades.
#[derive(Clone)]
pub(crate) struct Seen {
    pub(crate) name: String,
    pub(crate) address: String,
    pub(crate) icon: Option<Arc<gpui::Image>>,
    /// The bundled picture of a featured server.
    pub(crate) badge: Option<super::Badge>,
}

/// A removed server's card, folding shut after the server `anchor` (None: the list's start).
#[derive(Clone)]
pub(crate) struct Ghost {
    pub(crate) key: String,
    pub(crate) seen: Seen,
    anchor: Option<String>,
    gone: usize,
    since: Instant,
}

impl Ghost {
    /// Which removal it is, so each ghost's animation plays once.
    pub(crate) fn gone(&self) -> usize {
        self.gone
    }
}

#[derive(Default)]
struct Tracker {
    /// The keys in the order the list was last drawn, and what each showed.
    order: Vec<String>,
    seen: HashMap<String, Seen>,
    /// Counts the changes: an animation plays once under its change's number.
    changes: usize,
    glides: HashMap<String, (usize, f32, Instant)>,
    born: HashMap<String, (usize, Instant)>,
    ghosts: Vec<Ghost>,
}

thread_local! {
    /// One tracker for each list that animates (`note`'s scope).
    static TRACKER: RefCell<HashMap<&'static str, Tracker>> = RefCell::new(HashMap::new());
}

/// A key for each server in `list`, in order: its name and address, numbered when the same pair
/// is listed twice.
pub(crate) fn keys(list: &[(String, String)]) -> Vec<String> {
    let mut count: HashMap<(&str, &str), usize> = HashMap::new();
    list.iter()
        .map(|(name, address)| {
            let n = count.entry((name, address)).or_insert(0);
            *n += 1;
            format!("{name}\u{1f}{address}\u{1f}{n}")
        })
        .collect()
}

/// What the animations of the list drawn now are: per key how a glide or an arrival plays, and
/// the ghosts to draw.
#[derive(Default)]
pub(crate) struct Playing {
    pub(crate) glides: HashMap<String, (usize, f32)>,
    pub(crate) born: HashMap<String, usize>,
    ghosts: Vec<Ghost>,
}

impl Playing {
    /// The ghosts that fold shut right after the server `key` (None: before the first).
    pub(crate) fn after(&self, key: Option<&str>) -> Vec<&Ghost> {
        self.ghosts
            .iter()
            .filter(|ghost| ghost.anchor.as_deref() == key)
            .collect()
    }
}

/// Notes the servers of the list `scope` drawn now (`now`: each one's key and what it shows),
/// `step` px a place apart, and says what animates: a swap of two neighbours glides, new servers
/// arrive and removed ones leave a ghost. A different profile's list, or a rename, just changes.
pub(crate) fn note(scope: &'static str, now: Vec<(String, Seen)>, step: f32) -> Playing {
    TRACKER.with_borrow_mut(|all| all.entry(scope).or_default().note(now, step))
}

impl Tracker {
    fn note(&mut self, now: Vec<(String, Seen)>, step: f32) -> Playing {
        let at = Instant::now();
        let keys: Vec<String> = now.iter().map(|(key, _)| key.clone()).collect();
        self.ghosts
            .retain(|ghost| at.duration_since(ghost.since) < KEPT);
        self.glides
            .retain(|_, (_, _, since)| at.duration_since(*since) < KEPT);
        self.born
            .retain(|_, (_, since)| at.duration_since(*since) < KEPT);
        if keys != self.order {
            // Another profile's list shares nothing with the last: it is not an edit.
            let same_list = !self.order.is_empty()
                && (keys.is_empty() || keys.iter().any(|key| self.order.contains(key)));
            if same_list {
                self.changed(&keys, step, at);
            } else {
                self.glides.clear();
                self.born.clear();
                self.ghosts.clear();
            }
        }
        self.seen = now.into_iter().collect();
        self.order = keys;
        Playing {
            glides: self
                .glides
                .iter()
                .map(|(key, (moved, from, _))| (key.clone(), (*moved, *from)))
                .collect(),
            born: self
                .born
                .iter()
                .map(|(key, (born, _))| (key.clone(), *born))
                .collect(),
            ghosts: self.ghosts.clone(),
        }
    }

    /// The list is now `keys`, a change of the last one.
    fn changed(&mut self, keys: &[String], step: f32, at: Instant) {
        if let Some((first, second)) = swapped(&self.order, keys) {
            self.changes += 1;
            // The one now above was below, so it starts `step` lower; the other the reverse.
            self.glides
                .insert(keys[first].clone(), (self.changes, step, at));
            self.glides
                .insert(keys[second].clone(), (self.changes, -step, at));
            return;
        }
        let removed: Vec<usize> = (0..self.order.len())
            .filter(|n| !keys.contains(&self.order[*n]))
            .collect();
        let added: Vec<&String> = keys
            .iter()
            .filter(|key| !self.order.contains(key))
            .collect();
        // One in place of one is an edit of that server, not a removal and an addition.
        if removed.len() == 1 && added.len() == 1 && keys.len() == self.order.len() {
            return;
        }
        self.changes += 1;
        for key in added {
            self.born.insert(key.clone(), (self.changes, at));
        }
        for n in removed {
            let key = self.order[n].clone();
            let Some(seen) = self.seen.get(&key).cloned() else {
                continue;
            };
            // It folds shut after the nearest server before it that is still listed.
            let anchor = self.order[..n]
                .iter()
                .rev()
                .find(|before| keys.contains(before))
                .cloned();
            self.ghosts.push(Ghost {
                key,
                seen,
                anchor,
                gone: self.changes,
                since: at,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(name: &str) -> (String, Seen) {
        (
            name.to_string(),
            Seen {
                name: name.to_string(),
                address: String::new(),
                icon: None,
                badge: None,
            },
        )
    }

    fn list(names: &[&str]) -> Vec<(String, Seen)> {
        names.iter().map(|name| seen(name)).collect()
    }

    #[test]
    fn a_swap_glides_both_and_a_removal_leaves_a_ghost() {
        let mut tracker = Tracker::default();
        tracker.note(list(&["a", "b", "c"]), 60.);
        tracker.note(list(&["b", "a", "c"]), 60.);
        assert_eq!(tracker.glides["b"].1, 60.);
        assert_eq!(tracker.glides["a"].1, -60.);
        tracker.note(list(&["b", "c"]), 60.);
        assert_eq!(tracker.ghosts.len(), 1);
        assert_eq!(tracker.ghosts[0].anchor.as_deref(), Some("b"));
        tracker.note(list(&["b", "c", "d"]), 60.);
        assert!(tracker.born.contains_key("d"));
    }

    #[test]
    fn the_first_list_and_an_edit_do_not_animate() {
        let mut tracker = Tracker::default();
        tracker.note(list(&["a", "b"]), 60.);
        assert!(tracker.born.is_empty() && tracker.ghosts.is_empty());
        tracker.note(list(&["a", "c"]), 60.);
        assert!(tracker.born.is_empty() && tracker.ghosts.is_empty());
    }
}
