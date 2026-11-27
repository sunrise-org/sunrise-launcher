//! What Search lists and how it ranks: the entries, their groups and targets, and the matching.
use super::*;

/// Rows listed at most for a query, and at most from one group.
pub(super) const SHOWN: usize = 12;

pub(super) const PER_GROUP: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Group {
    Navigate,
    Profiles,
    Worlds,
    Mods,
    Packs,
    Settings,
    /// The launcher's keys: said here, never printed on a button.
    Shortcuts,
}

impl Group {
    pub(super) fn heading(self) -> &'static str {
        crate::i18n::tr_key(match self {
            Group::Navigate => "Go to",
            Group::Profiles => "Profiles",
            Group::Worlds => "Worlds",
            Group::Mods => "Installed mods",
            Group::Packs => "Installed resource and shader packs",
            Group::Settings => "Settings",
            Group::Shortcuts => "Keyboard shortcuts",
        })
    }
}

/// Where a row goes.
#[derive(Clone, PartialEq)]
pub(crate) enum Target {
    Page(Page),
    Section(Section),
    /// Settings › Appearance, at the "Worlds on Home" choice.
    WorldsOnHome,
    /// A profile, by its id.
    Profile(String),
    /// A world of the selected profile: Home, where it is played from.
    World,
    /// An installed mod, by its name: Mods › Installed, showing it.
    Mod(String),
    Pack(PackKind, String),
    /// The search itself: it is open already, and stays.
    Search,
    /// Shows or hides the sidebar.
    Sidebar,
}

/// A row's picture.
#[derive(Clone)]
pub(crate) enum Art {
    /// One of the launcher's own icons.
    Glyph(&'static str),
    Profile(String),
    /// A world's `icon.png`, and when the world was last saved (a new picture is read again).
    World(Option<PathBuf>, u64),
    /// A mod, by its id, and its icon's address in the registry.
    Mod(String, Option<String>),
    /// A pack, by its file's name, and its picture.
    Pack(String, Option<Arc<gpui::Image>>),
}

#[derive(Clone)]
pub(crate) struct Entry {
    pub group: Group,
    pub label: String,
    /// What it is, said at the row's end.
    pub hint: String,
    pub target: Target,
    pub art: Art,
    /// Listed only when the query finds it: a Settings section or row.
    pub quiet: bool,
}

impl Entry {
    pub(super) fn page(label: String, icon: &'static str, page: Page) -> Entry {
        Entry {
            group: Group::Navigate,
            label,
            hint: tr("Page").to_string(),
            target: Target::Page(page),
            art: Art::Glyph(icon),
            quiet: false,
        }
    }
}

/// The rows of Settings found by name, as their sections title them.
pub(super) const SETTINGS_ROWS: [(&str, Section); 17] = [
    (crate::i18n::tr_key("While you play"), Section::Game),
    (crate::i18n::tr_key("Minecraft folder"), Section::Game),
    (crate::i18n::tr_key("Profiles"), Section::Game),
    (crate::i18n::tr_key("Appearance"), Section::Appearance),
    (crate::i18n::tr_key("Theme"), Section::Appearance),
    (crate::i18n::tr_key("Accent"), Section::Appearance),
    (
        crate::i18n::tr_key("Custom background"),
        Section::Appearance,
    ),
    (crate::i18n::tr_key("Worlds on Home"), Section::Appearance),
    (crate::i18n::tr_key("Your themes"), Section::Appearance),
    (crate::i18n::tr_key("Show the cat"), Section::Appearance),
    (
        crate::i18n::tr_key("Installed game builds"),
        Section::Storage,
    ),
    (crate::i18n::tr_key("Import a game build"), Section::Storage),
    (
        crate::i18n::tr_key("Remove old game builds automatically"),
        Section::Storage,
    ),
    (
        crate::i18n::tr_key("Game files and worlds"),
        Section::Storage,
    ),
    (
        crate::i18n::tr_key("Downloads kept for reuse"),
        Section::Storage,
    ),
    (crate::i18n::tr_key("Your data"), Section::Privacy),
    (crate::i18n::tr_key("Language"), Section::Launcher),
];

/// The launcher's own places: its pages, then the sections and rows of Settings.
pub(crate) fn destinations() -> Vec<Entry> {
    let under = |parent: &str, child: &str| format!("{} › {}", tr(parent), tr(child));
    let mut all = vec![
        Entry::page(tr("Home").to_string(), "home", Page::Home),
        Entry::page(tr("Mods").to_string(), "puzzle", Page::Discover),
        Entry::page(tr("Packs").to_string(), "package", Page::Packs),
        Entry::page(
            under(crate::i18n::tr_key("Servers"), crate::i18n::tr_key("Join")),
            "globe",
            Page::Join,
        ),
        Entry::page(
            under(crate::i18n::tr_key("Servers"), crate::i18n::tr_key("Host")),
            "server",
            Page::Hosting,
        ),
        Entry::page(tr("Skins").to_string(), "shirt", Page::Skins),
        Entry::page(tr("Settings").to_string(), "sliders", Page::Settings),
    ];
    all.extend(Section::ALL.map(|section| Entry {
        target: Target::Section(section),
        quiet: true,
        ..Entry::page(
            under(crate::i18n::tr_key("Settings"), section.label()),
            "sliders",
            Page::Settings,
        )
    }));
    all.extend(SETTINGS_ROWS.map(|(title, section)| Entry {
        group: Group::Settings,
        label: tr(title).to_string(),
        hint: under(crate::i18n::tr_key("Settings"), section.label()),
        target: match title {
            "Worlds on Home" => Target::WorldsOnHome,
            _ => Target::Section(section),
        },
        art: Art::Glyph("sliders"),
        quiet: true,
    }));
    all
}

/// The key held with a letter for the launcher's shortcuts, as its platform names it.
pub(super) fn keys(letter: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("⌘{letter}")
    } else {
        format!("Ctrl {letter}")
    }
}

/// The launcher's keyboard shortcuts, each a row that does what its key does.
pub(crate) fn shortcuts() -> Vec<Entry> {
    [
        (crate::i18n::tr_key("Search"), "K", Target::Search, "search"),
        (
            crate::i18n::tr_key("Show or hide the sidebar"),
            "B",
            Target::Sidebar,
            "home",
        ),
    ]
    .map(|(label, letter, target, icon)| Entry {
        group: Group::Shortcuts,
        label: tr(label).to_string(),
        hint: keys(letter),
        target,
        art: Art::Glyph(icon),
        quiet: false,
    })
    .into()
}

/// How well `label` answers `query`, lower being better; none when a word of the query is not
/// in it. A word that is the whole label beats one that starts it, which beats one that starts
/// a later word of it, which beats one found anywhere.
pub(crate) fn rank(query: &str, label: &str) -> Option<u32> {
    let label = label.to_lowercase();
    query
        .to_lowercase()
        .split_whitespace()
        .map(|word| {
            if label == word {
                Some(0)
            } else if label.starts_with(word) {
                Some(1)
            } else if label
                .split(|c: char| !c.is_alphanumeric())
                .any(|part| part.starts_with(word))
            {
                Some(2)
            } else {
                label.contains(word).then_some(3)
            }
        })
        .sum()
}

/// The rows for `query`, as indexes into `entries`, in the order they are listed: group after
/// group, the group holding the best match first, each group's best matches first. With no
/// query: the pages and the profiles, then the keyboard shortcuts, which are never crowded out.
pub(crate) fn results(entries: &[Entry], query: &str) -> Vec<usize> {
    let query = query.trim();
    if query.is_empty() {
        let of = |shortcuts: bool| {
            (0..entries.len()).filter(move |&i| {
                let e = &entries[i];
                !e.quiet
                    && match e.group {
                        Group::Navigate | Group::Profiles => !shortcuts,
                        Group::Shortcuts => shortcuts,
                        _ => false,
                    }
            })
        };
        let keys = of(true).count();
        return of(false)
            .take(SHOWN.saturating_sub(keys))
            .chain(of(true))
            .collect();
    }
    let mut groups: Vec<(Group, Vec<(u32, usize)>)> = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        let Some(score) = rank(query, &entry.label) else {
            continue;
        };
        match groups.iter_mut().find(|(group, _)| *group == entry.group) {
            Some((_, found)) => found.push((score, i)),
            None => groups.push((entry.group, vec![(score, i)])),
        }
    }
    for (_, found) in &mut groups {
        // Stable: rows that match equally keep the order they were gathered in.
        found.sort_by_key(|(score, _)| *score);
        found.truncate(PER_GROUP);
    }
    groups.sort_by_key(|(_, found)| found[0].0);
    groups
        .into_iter()
        .flat_map(|(_, found)| found)
        .map(|(_, i)| i)
        .take(SHOWN)
        .collect()
}
