//! Switch plans against an exact prepared build and keeps the result private through approval.
use super::*;
use crate::{managed_play::Active, worlds::switch::Question};

pub struct SwitchHooks<'a> {
    pub cancellation: &'a native_mods::Cancellation,
    pub progress: &'a dyn Fn(InstallProgress),
    pub approve: &'a dyn Fn(&SwitchQuestion) -> Result<SwitchDecision, String>,
    pub at_cutoff: &'a dyn Fn() -> Result<(), String>,
}

/// The complete decision for one prepared candidate, never a request to update saves.
#[derive(Clone, Debug)]
pub struct SwitchQuestion {
    pub build: String,
    pub worlds: Question,
    pub held: Vec<String>,
    pub unproved: Vec<String>,
    /// What the fit check or build said, by the name shown in `held` or `unproved`.
    pub reasons: BTreeMap<String, String>,
    pub removed: Vec<(String, String)>,
    /// Of `held`, `unproved` and `removed`, the mods the player had on: what a switch that
    /// keeps every wanted mod refuses to leave out.
    pub wanted_left_out: Vec<String>,
}

/// How a switch treats the player's mods. The window and its tests take it from here, so a test
/// of Home's switch runs the same rule the button does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SwitchPolicy {
    /// The picker's and Update game's: the player answers any question the switch asks.
    #[default]
    Ask,
    /// Home's switch to the build the mods have releases for: the registry mods an earlier
    /// switch held off while they were on come back, and it stops before any change when its
    /// fresh check would still leave out one the player had on.
    KeepEveryMod,
}

impl SwitchPolicy {
    /// Whether the switch offers held-off registry mods again (`V2SwitchHooks::recover_held`).
    pub fn recover_held(self) -> bool {
        self == Self::KeepEveryMod
    }

    /// The answer the policy gives on its own, before the player is asked; None asks them.
    pub fn answer(self, question: &SwitchQuestion) -> Option<SwitchDecision> {
        match self {
            Self::Ask => None,
            Self::KeepEveryMod => question.all_mods_or_cancel(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwitchDecision {
    Cancel,
    TurnOff,
    Remove,
}

impl SwitchQuestion {
    pub fn needed(&self) -> bool {
        self.worlds.needed() || self.has_omissions() || !self.removed.is_empty()
    }

    pub fn has_omissions(&self) -> bool {
        !self.held.is_empty() || !self.unproved.is_empty()
    }

    /// The answer for a switch that has to keep every wanted mod (Home's switch to the build
    /// the mods have releases for): Cancel, before anything is published, when it would hold
    /// or remove one the player had on; None leaves the question to the player (only saved
    /// worlds, or mods they had turned off, are at stake).
    pub fn all_mods_or_cancel(&self) -> Option<SwitchDecision> {
        (!self.wanted_left_out.is_empty()).then_some(SwitchDecision::Cancel)
    }

    pub fn omission_summary(&self) -> String {
        let count = self.held.len() + self.unproved.len();
        if count == 1 {
            tr("1 mod can’t move to this build yet. You can keep it installed but turned off, or remove it.").into()
        } else {
            trf(
                "{count} mods can’t move to this build yet. You can keep them installed but turned off, or remove them.",
                &[("count", &count.to_string())],
            )
        }
    }

    pub fn hold_lines(&self) -> impl Iterator<Item = String> + '_ {
        let line = |name: &String, said: &str, with_reason: &str| match self.reasons.get(name) {
            Some(reason) => trf(with_reason, &[("mod", name), ("reason", reason)]),
            None => trf(said, &[("mod", name)]),
        };
        self.held
            .iter()
            .map(move |name| {
                line(
                    name,
                    "{mod} — Not compatible with this build.",
                    "{mod} — Not compatible with this build. {reason}",
                )
            })
            .chain(self.unproved.iter().map(move |name| {
                line(
                    name,
                    "{mod} — Fit could not be verified.",
                    "{mod} — Fit could not be verified. {reason}",
                )
            }))
    }

    pub fn world_lines(&self) -> Vec<String> {
        #[derive(Default)]
        struct Summary<'a> {
            newer: bool,
            unknown_order: bool,
            mods: Option<&'a [String]>,
        }
        let mut by_world: BTreeMap<&str, Summary<'_>> = BTreeMap::new();
        for risk in &self.worlds.affected {
            if !risk.mods.is_empty() {
                by_world.entry(risk.world.as_str()).or_default().mods = Some(&risk.mods);
            }
        }
        for world in &self.worlds.newer {
            by_world.entry(world.as_str()).or_default().newer = true;
        }
        for world in &self.worlds.unknown_order {
            by_world.entry(world.as_str()).or_default().unknown_order = true;
        }
        let omitted: BTreeSet<&str> = self
            .held
            .iter()
            .chain(&self.unproved)
            .map(String::as_str)
            .collect();
        by_world
            .into_iter()
            .filter_map(|(world, summary)| {
                let age = if summary.newer {
                    Some(tr("last played on a newer game"))
                } else if summary.unknown_order {
                    Some(tr("last game build is unknown"))
                } else {
                    None
                };
                let mods = summary.mods.map(|list| {
                    let listed: BTreeSet<&str> = list.iter().map(String::as_str).collect();
                    if !omitted.is_empty() && listed == omitted {
                        tr("content from the mods above may be lost").into()
                    } else if !omitted.is_empty() && omitted.is_subset(&listed) {
                        let extra: Vec<_> = list
                            .iter()
                            .map(String::as_str)
                            .filter(|name| !omitted.contains(*name))
                            .collect();
                        trf(
                            "content from the mods above and {mods} may be lost",
                            &[("mods", &extra.join(", "))],
                        )
                    } else {
                        trf(
                            "content from these mods may be lost: {mods}",
                            &[("mods", &list.join(", "))],
                        )
                    }
                });
                match (age, mods) {
                    (Some(age), Some(mods)) => Some(trf(
                        "{world}: {age}; {mods}.",
                        &[("world", world), ("age", age), ("mods", &mods)],
                    )),
                    (Some(age), None) => {
                        Some(trf("{world}: {age}.", &[("world", world), ("age", age)]))
                    }
                    (None, Some(mods)) => Some(trf(
                        "{world}: {mods}.",
                        &[("world", world), ("mods", &mods)],
                    )),
                    (None, None) => None,
                }
            })
            .collect()
    }
}

pub enum Switched {
    AlreadySelected,
    Stayed,
    Changed(Box<Active>),
}

pub use crate::managed_play::BuildChoice;

pub fn save_build_choice(instance_id: &str, tag: &str, choice: BuildChoice) -> Result<(), String> {
    crate::instances::edit(instance_id, |meta| match choice {
        BuildChoice::Follow(channel) => {
            meta.version = None;
            meta.channel = Some(channel);
        }
        BuildChoice::Keep => {
            meta.version = Some(tag.into());
            if let Some(channel) = crate::instances::Channel::of_build(tag) {
                meta.channel = Some(channel);
            }
        }
    })
    .map(|_| ())
}

pub(super) fn sha256_bytes(hex: &str) -> Result<[u8; 32], String> {
    let mut bytes = [0; 32];
    if hex.len() != 64 || !hex.is_ascii() {
        return Err(unproved_switch());
    }
    for (i, out) in bytes.iter_mut().enumerate() {
        *out = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(|_| unproved_switch())?;
    }
    Ok(bytes)
}

fn unproved_switch() -> String {
    tr("Sunrise couldn't verify the full mod set for this build. Nothing was switched.").into()
}
