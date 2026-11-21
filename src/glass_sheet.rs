//! Instance settings as a dialog centred over the page, drawn as Settings is (the glass look):
//! its icon, "Instance settings", the instance's name and version at the top; Settings' list of
//! sections on the left (General, Installation, Hooks, Storage) and the open one on the
//! right, under its name and what it is for, its rows in groups; Save and Cancel and the
//! instance's folder along the foot. The page behind is frosted and dimmed, as under Settings.
use super::*;

use servers_page::Field;
use sunrise_launcher::instances::{Channel, Icon, Instance};

/// The sheet's sections.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum SheetTab {
    #[default]
    General,
    Installation,
    Hooks,
}

/// What the dialog would keep, as typed or picked.
struct Draft {
    name: String,
    group: String,
    icon: Option<Icon>,
    /// A picture chosen, or the saved one removed.
    picture: bool,
    /// Sunrise's version and builds, for an instance that plays Sunrise.
    sunrise: Option<(Option<String>, Channel)>,
    /// The hook fields (before, wrapper, after, variables), once the Hooks section was opened.
    hooks: Option<[String; 4]>,
    /// The Linux switches, once one was flipped.
    switches: Option<[bool; 3]>,
}

impl Draft {
    /// The sections whose fields Save would change in `saved`.
    fn changed(&self, saved: &Instance) -> Vec<SheetTab> {
        let hooks = [
            saved.launch.pre_launch.clone(),
            saved.launch.wrapper.clone(),
            saved.launch.post_exit.clone(),
            sunrise_launcher::launch::env_line(&saved.launch.env),
        ];
        let general = self.name != saved.name
            || self.group != saved.group.clone().unwrap_or_default()
            || self.icon != saved.icon
            || self.picture;
        let installation = self
            .sunrise
            .as_ref()
            .is_some_and(|(pin, channel)| *pin != saved.version || *channel != saved.channel);
        let l = &saved.launch;
        let hooks = self.hooks.as_ref().is_some_and(|typed| *typed != hooks)
            || self
                .switches
                .is_some_and(|s| s != [l.gamemode, l.mangohud, l.dedicated_gpu]);
        [
            (general, SheetTab::General),
            (installation, SheetTab::Installation),
            (hooks, SheetTab::Hooks),
        ]
        .into_iter()
        .filter_map(|(changed, tab)| changed.then_some(tab))
        .collect()
    }
}

impl Launcher {
    /// Whether the sheet holds edits its Save has not kept.
    pub(super) fn sheet_changed(&self) -> bool {
        !self.sheet_changes().is_empty()
    }

    /// The sections holding edits Save has not kept.
    pub(super) fn sheet_changes(&self) -> Vec<SheetTab> {
        let Some(d) = self.instances.dialog.as_ref() else {
            return Vec::new();
        };
        let Some(saved) = d
            .editing
            .as_ref()
            .and_then(|id| self.instances.list.iter().find(|i| &i.id == id))
        else {
            return Vec::new();
        };
        let draft = Draft {
            name: self.servers.value(Field::InstanceName).trim().to_string(),
            group: self.servers.value(Field::InstanceGroup).to_string(),
            icon: d.icon.clone(),
            picture: match d.picture {
                Some(Some(_)) => true,
                Some(None) => saved.home.join(instances::PICTURE).is_file(),
                None => false,
            },
            sunrise: Some((d.pin.clone(), d.channel)),
            hooks: self.hooks_typed(&saved.id),
            switches: self.switches_typed(&saved.id),
        };
        draft.changed(saved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_launcher::instances::Kind as InstanceKind;

    fn saved() -> Instance {
        Instance {
            id: "mine".into(),
            name: "Mine".into(),
            kind: InstanceKind::Own,
            work: Default::default(),
            home: Default::default(),
            incarnation: None,
            icon: None,
            group: None,
            version: None,
            channel: Channel::Stable,
            native_profile: Default::default(),
            played: Default::default(),
            last_played: None,
            launch: Default::default(),
        }
    }

    fn untouched(saved: &Instance) -> Draft {
        Draft {
            name: saved.name.clone(),
            group: String::new(),
            icon: None,
            picture: false,
            sunrise: Some((None, Channel::Stable)),
            hooks: None,
            switches: None,
        }
    }

    #[test]
    fn save_waits_for_a_change_in_any_section() {
        let saved = saved();
        assert!(untouched(&saved).changed(&saved).is_empty());
        let general = SheetTab::General;
        let installation = SheetTab::Installation;
        type Edit = (fn(&mut Draft), SheetTab);
        let edits: [Edit; 8] = [
            (|d| d.name = "Renamed".into(), general),
            (|d| d.group = "Friends".into(), general),
            (|d| d.icon = Some(Icon::Colour("#6b4bff".into())), general),
            (|d| d.picture = true, general),
            (
                |d| d.sunrise = Some((Some("nightly-2026-11-13-aaaaaaa".into()), Channel::Stable)),
                installation,
            ),
            (|d| d.sunrise = Some((None, Channel::Nightly)), installation),
            // The Hooks section was opened and a command typed.
            (
                |d| {
                    d.hooks = Some([
                        "./backup.sh".into(),
                        String::new(),
                        String::new(),
                        String::new(),
                    ])
                },
                SheetTab::Hooks,
            ),
            // A Linux switch flipped waits for Save like a typed command.
            (|d| d.switches = Some([true, false, false]), SheetTab::Hooks),
        ];
        // Each edit marks its own section, and only that one.
        for (edit, section) in edits {
            let mut draft = untouched(&saved);
            edit(&mut draft);
            assert_eq!(draft.changed(&saved), [section]);
        }
        // Hooks opened and left as they were is no edit.
        let mut draft = untouched(&saved);
        draft.hooks = Some(Default::default());
        assert!(draft.changed(&saved).is_empty());
        // A switch flipped and flipped back is no edit either.
        draft.switches = Some([false, false, false]);
        assert!(draft.changed(&saved).is_empty());
    }
}
