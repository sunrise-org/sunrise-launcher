//! Discover's cards: what an installed mod says, its fit facts and install status.
use super::*;

impl Launcher {
    /// Never label cards from another profile or from an incomplete installed read.
    pub(in crate::mods) fn discover_installed(&self) -> Option<&InstalledRows> {
        let home = self.mods_home();
        self.mods_state
            .snapshot
            .as_ref()
            // While a re-read runs, the profile's last confirmed rows stay shown.
            .filter(|snap| snap.home == home && snap.error.is_none())
            .map(|snap| &snap.rows)
    }
}

impl InstalledRows {
    pub(in crate::mods) fn registry_entry(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|entry| {
            entry.source == SourceId::Sunrise && entry.kind == Kind::Mod && entry.project == id
        })
    }

    pub(in crate::mods) fn registry_pending(&self, id: &str) -> bool {
        self.registry_entry(id)
            .and_then(sunrise::mod_id)
            .is_some_and(|id| self.unbuilt.contains(id))
    }
}

/// A fit fact: a tag that says the mod fits this build.
pub(in crate::mods) fn fit_pill(label: String) -> Div {
    tag(label, chip::Kind::Neutral)
}

/// A fit fact for a mod that doesn't fit: the same tag, its words quiet.
pub(in crate::mods) fn unfit_pill(label: String) -> Div {
    tag(label, chip::Kind::Neutral).text_color(c(palette().muted))
}

pub(in crate::mods) fn install_fact(version: &str, pending: bool) -> Div {
    let text = if pending {
        trf(
            "Installed {version} · awaiting build",
            &[("version", version)],
        )
    } else {
        trf("Installed {version}", &[("version", version)])
    };
    status(
        if pending { "clock" } else { "tick-02" },
        text,
        if pending {
            palette().warning
        } else {
            palette().success
        },
    )
}
