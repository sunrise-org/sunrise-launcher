//! Collections in Mods: what Discover folds into a collection's card, which projects are
//! libraries, and what a collection's page lists and installs. Pure data in and out; the install
//! plan is the registry planner's own (`index::resolve`).
use super::{Card, Fit, Make};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use sunrise_launcher::i18n::{tr, trf};
use sunrise_launcher_smod::index::resolve::{self, InstallPlan, InstalledMod, RequestResult};
use sunrise_launcher_smod::index::{Collection, Index};
use sunrise_launcher_smod::manifest::Side;

/// The first collection that lists each member, by name: what a member's card says it is part of.
pub(in crate::mods) fn part_of(index: &Index) -> HashMap<&str, &str> {
    let mut of = HashMap::new();
    for c in &index.collections {
        for m in &c.members {
            of.entry(m.id.as_str()).or_insert(c.name.as_str());
        }
    }
    of
}

/// The registry's own category for a mod that exists to be depended on.
const LIBRARY: &str = "library";

/// The libraries: mods the registry puts in the "library" category that another listed mod's
/// newest version requires. Both must hold: the category alone is an author's word and a
/// standalone mod can carry it (JEI does), and being required alone makes no library of a mod
/// with content of its own. A collection's member can be one (the Sunrise Creatures library
/// is): it stays a member on its page, counted and badged as a library.
pub(in crate::mods) fn libraries(index: &Index) -> HashSet<String> {
    index
        .projects
        .iter()
        .filter_map(|p| p.versions.first().and_then(|v| v.dependencies.as_ref()))
        .flatten()
        .filter(|d| !d.optional)
        .map(|d| d.id.as_str())
        .filter_map(|id| index.project(id))
        .filter(|p| p.categories.iter().any(|k| k == LIBRARY))
        .map(|p| p.id.clone())
        .collect()
}

/// The collections' own cards for Discover: their members' downloads and newest day added up,
/// fitting when any member fits the build.
pub(in crate::mods) fn cards(index: &Index, mods: &[Card]) -> Vec<Card> {
    index
        .collections
        .iter()
        .filter_map(|c| {
            let members: Vec<&Card> = c
                .members
                .iter()
                .filter_map(|m| mods.iter().find(|card| card.id == m.id))
                .collect();
            let first = members.first()?;
            let mut categories: Vec<String> = Vec::new();
            for k in members.iter().flat_map(|m| &m.categories) {
                if !categories.contains(k) {
                    categories.push(k.clone());
                }
            }
            Some(Card {
                unsigned: false,
                id: c.id.clone(),
                name: c.name.clone(),
                authors: c.owners.join(", "),
                summary: c.summary.clone(),
                downloads: members.iter().map(|m| m.downloads).sum(),
                make: if members.iter().any(|m| m.make == Make::Native) {
                    Make::Native
                } else {
                    Make::Content
                },
                categories,
                icon: Some(c.icon.clone()),
                updated: members
                    .iter()
                    .map(|m| m.updated.clone())
                    .max()
                    .unwrap_or_default(),
                first: members
                    .iter()
                    .map(|m| m.first.clone())
                    .min()
                    .unwrap_or_default(),
                fit: members
                    .iter()
                    .find(|m| m.fit.fits())
                    .map_or_else(|| first.fit.clone(), |_| Fit::Fits),
                builds_locally: false,
                members: Some(members.len()),
                part_of: None,
                library: false,
            })
        })
        .collect()
}

/// How much of a set is ticked, as the header's box draws it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::mods) enum Ticks {
    None,
    Some,
    All,
}

pub(in crate::mods) fn ticks(ticked: usize, total: usize) -> Ticks {
    match ticked {
        0 => Ticks::None,
        n if n >= total => Ticks::All,
        _ => Ticks::Some,
    }
}

/// One line of a collection's list: a member, or a library one of the members needs.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::mods) struct Line {
    pub(in crate::mods) id: String,
    pub(in crate::mods) name: String,
    pub(in crate::mods) summary: String,
    pub(in crate::mods) icon: Option<String>,
    pub(in crate::mods) version: String,
    /// Needed by the picks and not one of the collection's mods: it can't be picked itself.
    pub(in crate::mods) library: bool,
    /// The registry's own "library" category: what the badge says.
    pub(in crate::mods) labelled: bool,
    pub(in crate::mods) ticked: bool,
    /// The ticked mods that need this one: it can't be unticked while there are any.
    pub(in crate::mods) needed_by: Vec<String>,
    pub(in crate::mods) installed: bool,
    /// Why this build can't install it.
    pub(in crate::mods) refused: Option<String>,
    /// One of the collection's mods that the collection needs (ticked by default) and this
    /// build can't install: the collection can't be installed while it stays so.
    pub(in crate::mods) needed: bool,
    /// Why it can't be installed together with the other picks.
    pub(in crate::mods) conflict: Option<String>,
}

impl Line {
    pub(in crate::mods) fn locked(&self) -> bool {
        !self.needed_by.is_empty()
    }
}

/// What pressing Install does, counted from the plan.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::mods) struct Summary {
    pub(in crate::mods) mods: usize,
    pub(in crate::mods) libraries: usize,
    pub(in crate::mods) updates: usize,
    pub(in crate::mods) bytes: u64,
}

impl Summary {
    pub(in crate::mods) fn nothing(&self) -> bool {
        self.mods + self.libraries + self.updates == 0
    }

    /// "Installs 8 mods and 1 library", with "Updates 1" after it when an installed mod must move.
    pub(in crate::mods) fn line(&self) -> String {
        let count = |n: usize, one: &'static str, many: &'static str| {
            (n > 0).then(|| match n {
                1 => tr(one).to_string(),
                n => trf(many, &[("count", &n.to_string())]),
            })
        };
        let things: Vec<String> = [
            count(self.mods, "1 mod", "{count} mods"),
            count(self.libraries, "1 library", "{count} libraries"),
        ]
        .into_iter()
        .flatten()
        .collect();
        let installs = match things.as_slice() {
            [] => None,
            [one] => Some(trf("Installs {things}", &[("things", one)])),
            [a, b] => Some(trf(
                "Installs {things}",
                &[("things", &trf("{a} and {b}", &[("a", a), ("b", b)]))],
            )),
            _ => unreachable!("two kinds of thing"),
        };
        let updates = count(
            self.updates,
            "Updates 1 installed mod",
            "Updates {count} installed mods",
        );
        match (installs, updates) {
            (Some(i), Some(u)) => format!("{i}. {u}"),
            (Some(i), None) => i,
            (None, Some(u)) => u,
            (None, None) => tr("Everything you picked is installed.").to_string(),
        }
    }
}

/// A collection's page for the ticks made so far.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::mods) struct View {
    /// The members in the collection's order, then the libraries they need.
    pub(in crate::mods) lines: Vec<Line>,
    /// How many of the collection's mods can be picked on this build.
    pub(in crate::mods) members: usize,
    /// How many mods the collection has, available or not: what "7 of 9 selected" counts.
    pub(in crate::mods) total: usize,
    /// The names of the needed mods this build can't install.
    pub(in crate::mods) needed_missing: Vec<String>,
    pub(in crate::mods) ticked: usize,
    pub(in crate::mods) header: Ticks,
    pub(in crate::mods) summary: Summary,
    /// Picks the planner can't install together, with its reason.
    pub(in crate::mods) conflict: Option<String>,
}

/// What a collection's page needs from the planner whatever is ticked, asked once for every
/// member: the ones this build refuses, and the mods outside the collection they need.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::mods) struct Base {
    refused: BTreeMap<String, String>,
    outside: Vec<String>,
    libraries: HashSet<String>,
}

fn plan(
    index: &Index,
    c: &Collection,
    chosen: &[&str],
    held: &[InstalledMod],
    game: &str,
) -> InstallPlan {
    resolve::install(
        index,
        &c.requests(chosen),
        held,
        game,
        Side::Both,
        |_, v| sunrise_launcher::browser::sunrise::fits_game(v, game),
    )
}

pub(in crate::mods) fn base(
    index: &Index,
    c: &Collection,
    held: &[InstalledMod],
    game: &str,
) -> Base {
    let all: Vec<&str> = c.members.iter().map(|m| m.id.as_str()).collect();
    // Each member alone: what this build refuses outright, as against what only conflicts with
    // another pick (`view` says that).
    let refused: BTreeMap<String, String> = all
        .iter()
        .filter_map(
            |id| match plan(index, c, &[id], held, game).requested.remove(*id)? {
                RequestResult::Refused { reason } => Some(((*id).to_owned(), reason)),
                _ => None,
            },
        )
        .collect();
    // Without the refused: what they would have needed is not asked of the player.
    let fine: Vec<&str> = all
        .iter()
        .copied()
        .filter(|id| !refused.contains_key(*id))
        .collect();
    let outside = plan(index, c, &fine, held, game)
        .required_by
        .into_keys()
        .filter(|id| !c.members.iter().any(|m| m.id == *id))
        .collect();
    Base {
        refused,
        outside,
        libraries: libraries(index),
    }
}

/// What a plan depends on besides the ticks: the index read, what is installed, the build.
#[derive(Clone, PartialEq)]
struct Key {
    index: usize,
    held: Vec<InstalledMod>,
    game: String,
}

/// A collection page's plans, kept: the planner runs again only when the ticks, what is
/// installed, the index or the build change, not on every draw.
#[derive(Default)]
pub(in crate::mods) struct Planned {
    base: Option<(Key, Base)>,
    view: Option<((Key, BTreeSet<String>), View)>,
    /// How many times each was worked out: for the tests.
    pub(in crate::mods) worked: (usize, usize),
}

impl Planned {
    pub(in crate::mods) fn base(
        &mut self,
        index: &Arc<Index>,
        c: &Collection,
        held: &[InstalledMod],
        game: &str,
    ) -> &Base {
        let key = Key {
            index: Arc::as_ptr(index) as usize,
            held: held.to_vec(),
            game: game.to_owned(),
        };
        if self.base.as_ref().is_none_or(|(k, _)| *k != key) {
            self.worked.0 += 1;
            self.base = Some((key, base(index, c, held, game)));
        }
        &self.base.as_ref().expect("just set").1
    }

    pub(in crate::mods) fn view(
        &mut self,
        index: &Arc<Index>,
        c: &Collection,
        ticked: &BTreeSet<String>,
        held: &[InstalledMod],
        game: &str,
    ) -> &View {
        let base = self.base(index, c, held, game).clone();
        let key = self.base.as_ref().expect("just set").0.clone();
        let wanted = (key, ticked.clone());
        if self.view.as_ref().is_none_or(|(k, _)| *k != wanted) {
            self.worked.1 += 1;
            self.view = Some((wanted, view(index, c, &base, ticked, held, game)));
        }
        &self.view.as_ref().expect("just set").1
    }
}

/// The members a collection's page starts with ticked: its defaults, minus any this build
/// refuses.
pub(in crate::mods) fn initial(c: &Collection, base: &Base) -> BTreeSet<String> {
    c.default_members()
        .filter(|id| !base.refused.contains_key(*id))
        .map(str::to_owned)
        .collect()
}

pub(in crate::mods) fn view(
    index: &Index,
    c: &Collection,
    base: &Base,
    ticked: &BTreeSet<String>,
    held: &[InstalledMod],
    game: &str,
) -> View {
    let chosen: Vec<&str> = c
        .members
        .iter()
        .map(|m| m.id.as_str())
        .filter(|id| ticked.contains(*id) && !base.refused.contains_key(*id))
        .collect();
    let now = plan(index, c, &chosen, held, game);
    let named = |id: &str| {
        index
            .project(id)
            .map_or_else(|| id.to_owned(), |p| p.name.clone())
    };
    let needed_by = |id: &str| -> Vec<String> {
        // In the collection's order, not the planner's.
        let by = now.required_by.get(id);
        c.members
            .iter()
            .filter(|m| by.is_some_and(|by| by.contains(&m.id)))
            .map(|m| named(&m.id))
            .collect()
    };
    let have = |id: &str| held.iter().any(|h| h.id == id);
    let conflicts: BTreeMap<&str, &str> = now
        .requested
        .iter()
        .filter_map(|(id, r)| match r {
            RequestResult::Refused { reason } => Some((id.as_str(), reason.as_str())),
            _ => None,
        })
        .collect();
    let line = |id: &str, library: bool| {
        let p = index.project(id);
        let needed_by = needed_by(id);
        Line {
            id: id.to_owned(),
            name: named(id),
            summary: p.map(|p| p.summary.clone()).unwrap_or_default(),
            icon: p.and_then(|p| p.icon.clone()),
            version: p
                .and_then(|p| index.installable(p, game).or(p.versions.first()))
                .map(|v| v.version.clone())
                .unwrap_or_default(),
            library,
            labelled: base.libraries.contains(id),
            // What another pick needs is ticked whether or not it was picked itself.
            ticked: !needed_by.is_empty() || (!library && chosen.contains(&id)),
            needed_by,
            installed: have(id),
            refused: base.refused.get(id).cloned(),
            needed: !library
                && base.refused.contains_key(id)
                && c.members.iter().any(|m| m.id == id && m.default),
            conflict: conflicts.get(id).map(|why| (*why).to_owned()),
        }
    };
    let mut lines: Vec<Line> = c.members.iter().map(|m| line(&m.id, false)).collect();
    lines.extend(base.outside.iter().map(|id| line(id, true)));
    let mut summary = Summary {
        updates: now.update.len(),
        ..Summary::default()
    };
    for m in &now.install {
        let project = index.project(&m.id);
        match base.libraries.contains(&m.id) {
            true => summary.libraries += 1,
            false => summary.mods += 1,
        }
        summary.bytes += project
            .and_then(|p| p.versions.iter().find(|v| v.version == m.version))
            .map_or(0, |v| v.file.size);
    }
    let selectable = c.members.len() - base.refused.len();
    let ticked_members = lines.iter().filter(|l| !l.library && l.ticked).count();
    let conflict = lines.iter().find_map(|l| l.conflict.clone());
    let needed_missing = lines
        .iter()
        .filter(|l| l.needed)
        .map(|l| l.name.clone())
        .collect();
    View {
        members: selectable,
        total: c.members.len(),
        needed_missing,
        ticked: ticked_members,
        header: ticks(ticked_members, selectable),
        lines,
        summary,
        conflict,
    }
}

impl View {
    /// A row's box: ticks or unticks a member. A library or a mod others need follows the picks.
    pub(in crate::mods) fn flip(&self, ticked: &mut BTreeSet<String>, id: &str) {
        let Some(line) = self.lines.iter().find(|l| l.id == id) else {
            return;
        };
        if line.library || line.locked() || line.refused.is_some() {
            return;
        }
        if !ticked.remove(id) {
            ticked.insert(id.to_owned());
        }
    }

    /// The header's box: every member that can be ticked, or, when all are, none. What a pick
    /// needed is then needed by nothing and unlocks on its own.
    pub(in crate::mods) fn flip_all(&self, ticked: &mut BTreeSet<String>) {
        ticked.clear();
        if self.header != Ticks::All {
            ticked.extend(
                self.lines
                    .iter()
                    .filter(|l| !l.library && l.refused.is_none())
                    .map(|l| l.id.clone()),
            );
        }
    }

    /// What the Install card says it does: the plan, or why there is nothing to do.
    pub(in crate::mods) fn plan_line(&self) -> String {
        if !self.needed_missing.is_empty() {
            trf(
                if self.needed_missing.len() == 1 {
                    "{mods} is needed and isn't available for your version"
                } else {
                    "{mods} are needed and aren't available for your version"
                },
                &[("mods", &crate::mods::names(&self.needed_missing))],
            )
        } else if self.ticked == 0 {
            tr("Pick the mods you want.").to_string()
        } else if self.conflict.is_some() {
            tr("Some picks can't be installed together.").to_string()
        } else {
            self.summary.line()
        }
    }

    /// Whether Install does anything: something is picked, the plan has work, nothing conflicts,
    /// and every mod the collection needs can be installed on this build (one it marks
    /// optional that can't doesn't stop the rest).
    pub(in crate::mods) fn can_install(&self) -> bool {
        self.ticked > 0
            && !self.summary.nothing()
            && self.conflict.is_none()
            && self.needed_missing.is_empty()
    }

    /// Whether everything picked is installed already.
    pub(in crate::mods) fn done(&self) -> bool {
        self.ticked > 0 && self.summary.nothing()
    }

    /// What Install is given: the ticked members not yet installed. Libraries and what is
    /// refused are never asked for; the planner brings what a member needs.
    pub(in crate::mods) fn to_install(&self) -> Vec<String> {
        self.lines
            .iter()
            .filter(|l| !l.library && l.ticked && !l.installed && l.refused.is_none())
            .map(|l| l.id.clone())
            .collect()
    }
}

/// Whether a collection's card reads "Installed": every member that is picked by default and
/// can be installed on this build is. A member left unticked by default, or one this build
/// can't take, never keeps it from saying so.
pub(in crate::mods) fn installed(c: &Collection, cards: &[Card], have: &HashSet<String>) -> bool {
    let wanted: Vec<&str> = c
        .members
        .iter()
        .filter(|m| m.default)
        .map(|m| m.id.as_str())
        .filter(|id| cards.iter().any(|card| card.id == *id && card.fit.fits()))
        .collect();
    !wanted.is_empty() && wanted.iter().all(|id| have.contains(*id))
}
