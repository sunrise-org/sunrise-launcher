//! What the Mods screens show, worked out from the registry index and the installed list:
//! pure data in and out, no files, no network, no window. Everything here only reads.
use super::build_run::State as BuildState;
use std::collections::{BTreeSet, HashMap, HashSet};
use sunrise_launcher::browser::installed::Entry;
use sunrise_launcher::i18n::{tr, trf};
use sunrise_launcher::native_mods;
use sunrise_launcher_smod::index::{self, Index};
use sunrise_launcher_smod::manifest::Side;

pub(super) mod collection;

/// What a mod is made of: a native source bundle or a content-only `.smod`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum Make {
    Native,
    Content,
}

impl Make {
    pub(super) const ALL: [Make; 2] = [Make::Native, Make::Content];

    pub(super) fn label(self) -> &'static str {
        match self {
            Make::Native => tr("Native"),
            Make::Content => tr("Content"),
        }
    }
}

/// Whether the version the player would get runs on the build they play.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Fit {
    Fits,
    /// Versions exist, none for this build: the newest build one was made for.
    OtherBuild(String),
    /// Taken down, or asks for something this Sunrise doesn't have.
    Refused(String),
    /// No registry fit evidence, as for a local archive's details.
    Unknown,
}

impl Fit {
    pub(super) fn fits(&self) -> bool {
        matches!(self, Fit::Fits)
    }

    pub(super) fn reason(&self) -> Option<String> {
        match self {
            Fit::Fits => None,
            Fit::OtherBuild(game) => Some(trf("Made for {game}", &[("game", game)])),
            Fit::Refused(why) => Some(why.clone()),
            Fit::Unknown => Some(tr("Fit could not be verified").to_owned()),
        }
    }
}

/// One mod in Discover.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Card {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) authors: String,
    pub(super) summary: String,
    pub(super) downloads: u64,
    pub(super) make: Make,
    pub(super) categories: Vec<String>,
    pub(super) icon: Option<String>,
    /// The newest version's day, for sorting.
    pub(super) updated: String,
    pub(super) first: String,
    pub(super) fit: Fit,
    /// A native mod with no prebuilt binary for this game: it is built on this computer.
    pub(super) builds_locally: bool,
    /// A collection's card: how many mods it lists.
    pub(super) members: Option<usize>,
    /// The collection that lists this mod, by name: it shows in Discover only as that card.
    pub(super) part_of: Option<String>,
    /// A library (`collection::libraries`): found by
    /// searching, never in the browse list.
    pub(super) library: bool,
}

pub(super) fn make_of(p: &index::Project) -> Option<Make> {
    match p.kind {
        index::Kind::NativeMod => Some(Make::Native),
        index::Kind::Mod => Some(Make::Content),
        _ => None,
    }
}

pub(super) fn fit_of(index: &Index, p: &index::Project, game: &str) -> Fit {
    if index.installable(p, game).is_some() {
        return Fit::Fits;
    }
    let for_game: Vec<&index::Version> = p.versions.iter().filter(|v| v.game == game).collect();
    if for_game.is_empty() {
        return Fit::OtherBuild(
            p.versions
                .first()
                .map(|v| v.game.clone())
                .unwrap_or_default(),
        );
    }
    if let Some(r) = for_game
        .iter()
        .find_map(|v| index.revoked(&p.id, &v.file.sha256))
    {
        return Fit::Refused(trf("Taken down: {reason}", &[("reason", &r.reason)]));
    }
    Fit::Refused(tr("Made for Sunrise's earlier mod system").to_string())
}

/// Every mod and native mod of the registry, for the build `game`. Packs are left out.
pub(super) fn cards(index: &Index, game: &str) -> Vec<Card> {
    let part_of = collection::part_of(index);
    let libraries = collection::libraries(index);
    let mut cards: Vec<Card> = index
        .projects
        .iter()
        .filter_map(|p| {
            let make = make_of(p)?;
            Some(Card {
                id: p.id.clone(),
                name: p.name.clone(),
                authors: p.authors.join(", "),
                summary: p.summary.clone(),
                downloads: p.downloads,
                make,
                categories: p.categories.clone(),
                icon: p.icon.clone(),
                updated: p
                    .versions
                    .iter()
                    .map(|v| v.published.as_str())
                    .max()
                    .unwrap_or("")
                    .into(),
                first: p
                    .versions
                    .iter()
                    .map(|v| v.published.as_str())
                    .min()
                    .unwrap_or("")
                    .into(),
                fit: fit_of(index, p, game),
                // Only claimed when the registry lists no prebuilt at all: which one fits this
                // computer is decided at install, by build and platform.
                builds_locally: make == Make::Native
                    && index
                        .installable(p, game)
                        .is_some_and(|v| v.native_binaries.is_empty()),
                members: None,
                part_of: part_of.get(p.id.as_str()).map(|name| (*name).to_owned()),
                library: libraries.contains(&p.id),
            })
        })
        .collect();
    let collections = collection::cards(index, &cards);
    cards.extend(collections);
    cards
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Sort {
    Downloads,
    Updated,
    Newest,
    Name,
}

impl Sort {
    pub(super) const ALL: [Sort; 4] = [Sort::Downloads, Sort::Updated, Sort::Newest, Sort::Name];

    pub(super) fn label(self) -> &'static str {
        match self {
            Sort::Downloads => tr("Most downloaded"),
            Sort::Updated => tr("Recently updated"),
            Sort::Newest => tr("Newest"),
            Sort::Name => tr("Name"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Filters {
    pub(super) text: String,
    pub(super) category: Option<String>,
    pub(super) fits_only: bool,
    pub(super) make: Option<Make>,
    pub(super) sort: Sort,
}

impl Default for Filters {
    fn default() -> Self {
        Self {
            text: String::new(),
            category: None,
            fits_only: true,
            make: None,
            sort: Sort::Downloads,
        }
    }
}

/// Indexes into `cards` that pass `f`, in the order asked for.
pub(super) fn discover(cards: &[Card], f: &Filters) -> Vec<usize> {
    let words: Vec<String> = f.text.split_whitespace().map(str::to_lowercase).collect();
    let mut shown: Vec<usize> = (0..cards.len())
        .filter(|&i| {
            let c = &cards[i];
            let hay = format!("{} {} {} {}", c.id, c.name, c.summary, c.authors).to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
                && f.category.as_ref().is_none_or(|k| c.categories.contains(k))
                && f.make.is_none_or(|m| c.make == m)
                && (!f.fits_only || c.fit.fits())
                // Browsing shows a collection's card for its mods and no libraries; a search
                // finds each of them.
                && (!f.text.is_empty() || (c.part_of.is_none() && !c.library))
        })
        .collect();
    match f.sort {
        Sort::Downloads => shown.sort_by_key(|&i| std::cmp::Reverse(cards[i].downloads)),
        Sort::Updated => shown.sort_by(|&a, &b| {
            cards[b]
                .updated
                .cmp(&cards[a].updated)
                .then(cards[b].downloads.cmp(&cards[a].downloads))
        }),
        Sort::Newest => shown.sort_by(|&a, &b| {
            cards[b]
                .first
                .cmp(&cards[a].first)
                .then(cards[b].downloads.cmp(&cards[a].downloads))
        }),
        Sort::Name => shown.sort_by_key(|&i| cards[i].name.to_lowercase()),
    }
    shown
}

/// The categories the mods carry, most used first.
pub(super) fn categories(cards: &[Card]) -> Vec<String> {
    let mut uses: HashMap<&str, usize> = HashMap::new();
    for c in cards {
        for k in &c.categories {
            *uses.entry(k).or_default() += 1;
        }
    }
    let mut all: Vec<(&str, usize)> = uses.into_iter().collect();
    all.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    all.into_iter().map(|(k, _)| k.to_owned()).collect()
}

/// `6814` → `6.8k downloads`.
pub(super) fn downloads(n: u64) -> String {
    let n = match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.1}k", n as f64 / 1e3),
        _ => format!("{:.1}M", n as f64 / 1e6),
    };
    trf("{count} downloads", &[("count", &n)])
}

pub(super) fn size(bytes: u64) -> String {
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{:.1} KiB", bytes as f64 / 1024.),
        _ => format!("{:.1} MiB", bytes as f64 / 1_048_576.),
    }
}

pub(super) fn side_label(side: Option<&Side>) -> &'static str {
    match side {
        None | Some(Side::Client) => tr("Not needed on servers"),
        Some(Side::Both) => tr("Servers need it too"),
        Some(Side::Server) => tr("Server only"),
        Some(_) => tr("Server only"),
    }
}

// ---------------------------------------------------------------- the project page

#[derive(Clone, Debug, PartialEq)]
pub(super) struct VersionLine {
    pub(super) number: String,
    pub(super) published: String,
    pub(super) game: String,
    pub(super) fit: Fit,
    pub(super) size: u64,
}

/// A browser destination checked at the registry or installed-origin boundary.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum SourceLink {
    GitHub(String),
    Other(String),
}

impl SourceLink {
    pub(super) fn read(text: &str) -> Option<Self> {
        let (scheme, rest) = text.split_once("://")?;
        let authority = rest.split(['/', '?', '#']).next()?;
        if !scheme.eq_ignore_ascii_case("https")
            || authority.is_empty()
            || authority.contains('@')
            || text.bytes().any(|b| b.is_ascii_control() || b == b'\\')
        {
            return None;
        }
        let url = url::Url::parse(text).ok()?;
        if url.host_str()? != "github.com" {
            return Some(Self::Other(url.into()));
        }
        // Take owner/repo before the URL parser can normalize any dot segments.
        let path = rest.strip_prefix(authority)?.strip_prefix('/')?;
        let mut parts = path.split(['?', '#']).next()?.split('/');
        let owner = parts.next()?;
        let repo = parts.next()?;
        Self::github(owner, repo.strip_suffix(".git").unwrap_or(repo))
    }

    pub(super) fn github(owner: &str, repo: &str) -> Option<Self> {
        let repo_part = |part: &str| {
            !part.is_empty()
                && !matches!(part, "." | "..")
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
        };
        (repo_part(owner) && repo_part(repo))
            .then(|| Self::GitHub(format!("https://github.com/{owner}/{repo}")))
    }
}

/// Facts only the registry can provide.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct About {
    pub(super) license: String,
    pub(super) downloads: u64,
    pub(super) updated: String,
    pub(super) side: &'static str,
    pub(super) signed: bool,
}

/// Everything the shared details screen shows, from the registry or a local archive.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Page {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) authors: Vec<String>,
    pub(super) description: String,
    pub(super) source: Option<SourceLink>,
    pub(super) about: Option<About>,
    pub(super) categories: Vec<String>,
    pub(super) screenshots: Vec<String>,
    pub(super) icon: Option<String>,
    pub(super) make: Option<Make>,
    pub(super) versions: Vec<VersionLine>,
    pub(super) fit: Fit,
    /// The newest version that fits: what Add installs.
    pub(super) will_install: Option<VersionLine>,
    /// What the registry says about who made it and what it may do (`trust_and_capabilities`).
    pub(super) trust: Vec<String>,
    /// Native code made for another game build than the one played: checked before it runs.
    pub(super) another_build: bool,
    /// Mods the newest version lists as part of itself (a registry modpack's rule; a mod rarely has any).
    pub(super) requires: Vec<(String, String)>,
}

pub(super) fn page(
    index: &Index,
    id: &str,
    game: &str,
    trust: Vec<String>,
    another_build: bool,
) -> Option<Page> {
    let p = index.project(id)?;
    let make = make_of(p)?;
    let lines: Vec<VersionLine> = p
        .versions
        .iter()
        .map(|v| VersionLine {
            number: v.version.clone(),
            published: v.published.clone(),
            game: v.game.clone(),
            fit: match (v.game == game, index.revoked(&p.id, &v.file.sha256)) {
                (_, Some(r)) => Fit::Refused(trf("Taken down: {reason}", &[("reason", &r.reason)])),
                _ if !v.capabilities.unknown().is_empty() => {
                    Fit::Refused(tr("Made for Sunrise's earlier mod system").to_string())
                }
                (true, None) => Fit::Fits,
                (false, None) => Fit::OtherBuild(v.game.clone()),
            },
            size: v.file.size,
        })
        .collect();
    let will_install = index
        .installable(p, game)
        .and_then(|v| lines.iter().find(|l| l.number == v.version).cloned());
    Some(Page {
        id: p.id.clone(),
        name: p.name.clone(),
        authors: p.authors.clone(),
        description: p.description.clone(),
        source: SourceLink::read(&p.source),
        about: Some(About {
            license: p.license.clone(),
            downloads: p.downloads,
            updated: lines
                .iter()
                .map(|l| l.published.clone())
                .max()
                .unwrap_or_default(),
            side: side_label(p.versions.first().and_then(|v| v.side.as_ref())),
            signed: p.key.is_some(),
        }),
        categories: p.categories.clone(),
        screenshots: p.screenshots.clone(),
        icon: p.icon.clone(),
        make: Some(make),
        fit: fit_of(index, p, game),
        will_install,
        versions: lines,
        trust,
        another_build,
        requires: p
            .versions
            .first()
            .map(|v| {
                v.requires
                    .iter()
                    .map(|r| {
                        let name = index
                            .project(&r.id)
                            .map_or(r.id.clone(), |m| m.name.clone());
                        (name, r.version.clone())
                    })
                    .collect()
            })
            .unwrap_or_default(),
    })
}

// ---------------------------------------------------------------- installed

/// What stands between an installed mod and working, shown on its row.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Need {
    /// Taken down by the registry: it can't be turned on.
    TakenDown(String),
    /// A native mod, on, whose build for this computer isn't made yet.
    Build,
    /// A native mod that can't run until the player allows native code.
    Permission,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Row {
    pub(super) id: String,
    pub(super) title: String,
    pub(super) authors: String,
    pub(super) make: Option<Make>,
    pub(super) version: String,
    pub(super) on: bool,
    pub(super) need: Option<Need>,
    /// A browser-installed mod whose registry status couldn't be checked this run.
    pub(super) registry_unchecked: bool,
    /// The version an update would install.
    pub(super) update: Option<String>,
    /// Installed because another mod needed it.
    pub(super) dependency: bool,
    /// The player chose this version (`sunrise::chosen`): a batch leaves it alone.
    pub(super) pinned: bool,
    /// Not installed by the registry; no registry update.
    pub(super) local: Option<Local>,
    pub(super) icon: Option<String>,
    pub(super) entry: Option<Entry>,
    /// It can't run on the game this setup plays: the build left it out.
    pub(super) unfit: Option<Unfit>,
    /// The exact Switch receipt keeps the native archive off until a proved update.
    pub(super) held_for: Option<String>,
    /// The build left it out for another reason (it didn't build, it needs a mod that was left
    /// out, it clashes with another): what the player reads, in a sentence.
    pub(super) left_out: Option<Omission>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Omission {
    Failed(String),
    Skipped(String),
}

/// Why a mod is not on the game build its setup plays, in parts the row says in its own words.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Unfit {
    /// The build played, as people read it ("Nov 17").
    pub(super) on: String,
    /// The build it was made for: its version and its name ("Nov 16"), when known.
    pub(super) made_for: Option<(String, String)>,
    /// Each thing the game changed that it uses, for its author.
    pub(super) changes: Vec<String>,
}

/// Marks the rows of mods the last build left out: those the game was made without (another
/// version, or not fit for it) get their [`Unfit`] words, every other reason (it didn't build,
/// it needs a mod that was left out, it clashes) its sentence. A row left out has nothing to
/// build, so it no longer asks for a build. `on` is the build played, `name` how a build tag is
/// read by people.
pub(super) fn mark_left_out(
    rows: &mut [Row],
    left: &[native_mods::LeftOut],
    on: &str,
    name: impl Fn(&str) -> String,
) {
    for row in rows.iter_mut().filter(|r| r.on) {
        let Some(l) = left
            .iter()
            .find(|l| l.id == row.id && l.version == row.version)
        else {
            continue;
        };
        if !matches!(row.need, Some(Need::TakenDown(_))) {
            row.need = None;
        }
        match l.why {
            native_mods::Why::Unfit(_) | native_mods::Why::OtherVersion => {
                let detail = l.detail.clone().unwrap_or_default();
                row.unfit = Some(Unfit {
                    on: on.to_owned(),
                    made_for: detail.made_for.map(|tag| (tag.clone(), name(&tag))),
                    changes: detail.changes,
                });
            }
            native_mods::Why::DidNotBuild => row.left_out = Some(Omission::Failed(l.label())),
            _ => row.left_out = Some(Omission::Skipped(l.label())),
        }
    }
}

/// How a status reads: which of the theme's colours its glyph takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tone {
    Warning,
    Info,
    Accent,
    Success,
    Muted,
}

/// A row's status: a glyph, a few words, and the whole sentence for the tooltip.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Status {
    pub(super) icon: &'static str,
    pub(super) label: String,
    pub(super) tone: Tone,
    pub(super) tip: String,
}

/// The one button a row may hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RowAction {
    Allow,
    Update,
}

/// What a row puts in the list's shared columns. Every row has every column: one with nothing
/// to say leaves its status and action cells empty, and the switch's place is always kept, so
/// nothing in a row moves with what another row says.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Cells {
    /// "Native" or "Content"; empty when the registry doesn't say.
    pub(super) kind: &'static str,
    pub(super) version: String,
    pub(super) status: Option<Status>,
    /// The status opens the "needs an update" popover.
    pub(super) popover: bool,
    pub(super) action: Option<RowAction>,
    /// The switch can be flipped (a taken-down mod's is drawn dim).
    pub(super) switch: bool,
    /// The switch records the player's preference, but the mod isn't in the build.
    pub(super) unavailable: bool,
}

/// The widths of the list's fixed columns on a roomy window.
pub(super) mod column {
    pub(in super::super) const KIND: f32 = 64.;
    pub(in super::super) const VERSION: f32 = 72.;
    pub(in super::super) const STATUS: f32 = 176.;
    pub(in super::super) const ACTION: f32 = 96.;
    pub(in super::super) const SWITCH: f32 = 44.;
}

/// `row`'s cells. `error` is why its last update failed. The status is the first thing that
/// applies, most pressing first; whatever else applies is in its tooltip.
pub(super) fn cells(
    row: &Row,
    error: Option<&str>,
    building: Option<&BuildState>,
    native_allowed: bool,
) -> Cells {
    let mut said: Vec<Status> = Vec::new();
    let mut say = |icon, label: String, tone, tip: String| {
        said.push(Status {
            icon,
            label,
            tone,
            tip,
        })
    };
    if let Some(why) = error {
        say(
            "information-circle",
            tr("Update failed").into(),
            Tone::Warning,
            why.to_owned(),
        );
    }
    match &row.need {
        Some(Need::TakenDown(why)) => say(
            "cancel-01",
            tr("Taken down").into(),
            Tone::Warning,
            trf("Taken down: {reason}", &[("reason", why)]),
        ),
        Some(Need::Permission) => say(
            "shield",
            tr("Needs permission").into(),
            Tone::Warning,
            tr("Needs your permission to run native code").into(),
        ),
        Some(Need::Build) | None => {}
    }
    if let Some(build) = &row.held_for {
        say(
            "information-circle",
            tr("Needs an update from its author").into(),
            Tone::Warning,
            trf(
                "{mod} needs an update from its author for {build}.",
                &[("mod", &row.title), ("build", build)],
            ),
        );
    } else if let Some(unfit) = &row.unfit {
        say(
            "information-circle",
            tr("Needs an update").into(),
            Tone::Warning,
            trf(
                "{mod} needs an update for {build}. It comes back when its author releases one.",
                &[("mod", &row.title), ("build", &unfit.on)],
            ),
        );
    } else if let Some(omission) = &row.left_out {
        match omission {
            Omission::Failed(why) => {
                say("cancel-01", tr("Failed").into(), Tone::Warning, why.clone())
            }
            Omission::Skipped(why) => say(
                "information-circle",
                tr("Not in this build").into(),
                Tone::Warning,
                why.clone(),
            ),
        }
    } else {
        match (building, &row.need) {
            (Some(BuildState::NeedsUpdate { why, .. }), _) => say(
                "information-circle",
                tr("Needs an update").into(),
                Tone::Warning,
                why.clone(),
            ),
            (Some(BuildState::Failed(why)), _) => {
                say("cancel-01", tr("Failed").into(), Tone::Warning, why.clone())
            }
            (Some(BuildState::Building), Some(Need::Build)) => say(
                "hammer",
                tr("Building").into(),
                Tone::Accent,
                tr("Its build is running").into(),
            ),
            (Some(BuildState::Built), Some(Need::Build)) => say(
                "tick-02",
                tr("Built").into(),
                Tone::Success,
                tr("Built. It is in the game once the build is done.").into(),
            ),
            (Some(BuildState::Waiting), Some(Need::Build)) => say(
                "clock",
                tr("Waiting").into(),
                Tone::Muted,
                tr("Its build is running").into(),
            ),
            (_, Some(Need::Build)) => say(
                "clock",
                tr("Needs a build").into(),
                Tone::Warning,
                tr("Not built for this computer yet").into(),
            ),
            _ => {}
        }
    }
    if let Some(to) = &row.update {
        say(
            "refresh",
            trf("{version} is out", &[("version", to)]),
            Tone::Info,
            if row.pinned {
                trf(
                    "Version {version} is out. You chose its version: update it on its own.",
                    &[("version", to)],
                )
            } else if row.native() && !native_allowed {
                trf(
                    "Version {version} is out. Updating may ask you to allow native code.",
                    &[("version", to)],
                )
            } else {
                trf("Version {version} is out", &[("version", to)])
            },
        );
    }
    if let Some(local) = &row.local {
        say(
            "folder-01",
            match local.github.is_some() {
                true => tr("From GitHub").into(),
                false => tr("Local file").into(),
            },
            Tone::Muted,
            match local.github.is_some() {
                true => tr("Get updates from GitHub, not the registry.").into(),
                false => tr("Replace the local file to update this mod.").into(),
            },
        );
    }
    let popover = row.unfit.is_some()
        && said
            .first()
            .is_some_and(|s| s.label == tr("Needs an update"));
    let tip = said
        .iter()
        .map(|s| s.tip.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    Cells {
        kind: row.make.map_or("", Make::label),
        version: row.version.clone(),
        status: said.into_iter().next().map(|first| Status { tip, ..first }),
        popover,
        action: match (&row.need, &row.update) {
            (Some(Need::Permission), _) => Some(RowAction::Allow),
            (_, Some(_)) => Some(RowAction::Update),
            _ => None,
        },
        switch: row.held_for.is_none()
            && !matches!(row.need, Some(Need::TakenDown(_)))
            && !row.local.as_ref().is_some_and(|l| l.unreadable),
        unavailable: row.held_for.is_some()
            || row.unfit.is_some()
            || row.left_out.is_some()
            || matches!(
                building,
                Some(BuildState::Failed(_) | BuildState::NeedsUpdate { .. })
            ),
    }
}

impl Row {
    pub(super) fn installed_details(&self) -> Page {
        Page {
            id: self.id.clone(),
            name: self.title.clone(),
            authors: Vec::new(),
            description: String::new(),
            source: self
                .local
                .as_ref()
                .and_then(|local| local.github.as_ref())
                .and_then(|origin| SourceLink::github(&origin.owner, &origin.repo)),
            about: None,
            categories: Vec::new(),
            screenshots: Vec::new(),
            icon: self.icon.clone(),
            make: self.make,
            versions: Vec::new(),
            fit: Fit::Unknown,
            will_install: None,
            trust: Vec::new(),
            another_build: false,
            requires: Vec::new(),
        }
    }

    pub(super) fn native(&self) -> bool {
        self.make == Some(Make::Native)
    }

    pub(super) fn selectable_archive(&self) -> bool {
        self.local.as_ref().is_none_or(|local| !local.unreadable)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Group {
    NeedsYou,
    Updates,
    Rest,
}

impl Row {
    pub(super) fn group(&self) -> Group {
        if self.need.is_some() || self.held_for.is_some() {
            Group::NeedsYou
        } else if self.update.is_some() {
            Group::Updates
        } else {
            Group::Rest
        }
    }
}

/// What `rows` needs to know about the computer, read by the caller.
pub(super) struct World<'a> {
    pub(super) index: Option<&'a Index>,
    pub(super) game: &'a str,
    /// Verified archive hashes already rejected by the SDK for this base build.
    pub(super) unfit_updates: &'a BTreeSet<String>,
    pub(super) enabled: &'a HashSet<String>,
    /// Native mods whose build isn't made.
    pub(super) unbuilt: &'a HashSet<String>,
    pub(super) native_allowed: bool,
    /// The versions the player picked, by project id.
    pub(super) chosen: &'a std::collections::BTreeMap<String, String>,
    /// Native files the browser didn't install.
    pub(super) local: &'a [Local],
}

/// A `.smod` in the game folder that the registry browser didn't install (a local file or a
/// GitHub install): it can be turned on or off but is not managed by the registry.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Local {
    pub(super) id: String,
    /// The exact row, even if another archive claims the same mod ID.
    pub(super) file: String,
    pub(super) name: String,
    pub(super) version: String,
    pub(super) github: Option<sunrise_launcher::github_mods::Origin>,
    /// Native code, which needs permission and a build, rather than content only.
    pub(super) native: bool,
    /// The file can't be read: it has no id of its own (the id is only its file name), so it is
    /// never on, needs nothing and has no switch that could act on a working mod's id.
    pub(super) unreadable: bool,
}

/// The registry mods the browser installed, as rows. Packs, files of other sources and files
/// the browser didn't install are not rows (the caller says how many were left alone).
pub(super) fn rows(entries: &[Entry], w: &World) -> Vec<Row> {
    use sunrise_launcher::browser::{sunrise::mod_id, Kind, SourceId};
    let mut out: Vec<Row> = entries
        .iter()
        .filter(|e| e.source == SourceId::Sunrise && e.kind == Kind::Mod)
        .filter_map(|e| {
            let id = mod_id(e)?.to_owned();
            let p = w.index.and_then(|i| i.project(&e.project));
            let native = sunrise_launcher::native_status::is_native_file(&e.file);
            let make = match (p.and_then(make_of), native) {
                (_, true) => Some(Make::Native),
                (m, false) => m,
            };
            let on = w.enabled.contains(&id);
            let taken_down = w.index.and_then(|i| {
                let hash = e.checksum.strip_prefix("SHA-256:").unwrap_or_default();
                i.revoked(&e.project, hash).map(|r| r.reason.clone())
            });
            let need = if let Some(reason) = taken_down {
                Some(Need::TakenDown(reason))
            } else if native && on && !w.native_allowed {
                Some(Need::Permission)
            } else if native && on && w.unbuilt.contains(&id) {
                Some(Need::Build)
            } else {
                None
            };
            let update = w
                .index
                .zip(p)
                .and_then(|(i, p)| newer(i, p, &e.version, w.game, w.unfit_updates));
            Some(Row {
                title: p.map_or_else(|| id.clone(), |p| p.name.clone()),
                id,
                authors: p.map(|p| p.authors.join(", ")).unwrap_or_default(),
                make,
                version: e.number.clone(),
                on,
                need,
                registry_unchecked: w.index.is_none(),
                update,
                dependency: e.dependency,
                pinned: w.chosen.contains_key(&e.project),
                local: None,
                icon: p.and_then(|p| p.icon.clone()),
                entry: Some(e.clone()),
                unfit: None,
                held_for: None,
                left_out: None,
            })
        })
        .collect();
    out.extend(w.local.iter().map(|l| {
        let on = !l.unreadable && w.enabled.contains(&l.id);
        let need = match on && l.native {
            true if !w.native_allowed => Some(Need::Permission),
            true if w.unbuilt.contains(&l.id) => Some(Need::Build),
            _ => None,
        };
        Row {
            id: l.id.clone(),
            title: l.name.clone(),
            authors: String::new(),
            make: l.native.then_some(Make::Native),
            version: l.version.clone(),
            on,
            need,
            registry_unchecked: false,
            update: None,
            dependency: false,
            pinned: false,
            local: Some(l.clone()),
            icon: None,
            entry: None,
            unfit: None,
            held_for: None,
            left_out: None,
        }
    }));
    sort(&mut out);
    out
}

/// Rows in the order the list shows them: by group, then by name.
pub(super) fn sort(rows: &mut [Row]) {
    rows.sort_by(|a, b| {
        (a.group(), a.title.to_lowercase()).cmp(&(b.group(), b.title.to_lowercase()))
    });
}

impl PartialOrd for Group {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Group {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (*self as u8).cmp(&(*other as u8))
    }
}

/// A catalogue offer, not native fit proof. Exclude releases already proved not to fit.
pub(super) fn newer(
    index: &Index,
    p: &index::Project,
    installed: &str,
    game: &str,
    unfit_updates: &BTreeSet<String>,
) -> Option<String> {
    p.versions.iter().find(|v| v.version == installed)?;
    newer_for_hold(index, p, installed, game, unfit_updates)
}

/// A Switch hold may name a version older than the catalogue retains. The Update worker
/// still checks the old archive against the receipt and proves the selected replacement.
pub(super) fn newer_for_hold(
    index: &Index,
    p: &index::Project,
    installed: &str,
    game: &str,
    unfit_updates: &BTreeSet<String>,
) -> Option<String> {
    use sunrise_launcher_smod::dependencies::parse_version;
    let old = parse_version(installed).ok()?;
    p.versions
        .iter()
        .rev()
        .filter_map(|v| {
            if !sunrise_launcher::browser::sunrise::fits_game(v, game)
                || index.revoked(&p.id, &v.file.sha256).is_some()
                || unfit_updates.contains(&v.file.sha256.to_ascii_lowercase())
            {
                return None;
            }
            let version = parse_version(&v.version).ok()?;
            version
                .cmp_precedence(&old)
                .is_gt()
                .then_some((version, &v.version))
        })
        .max_by(|(a, _), (b, _)| a.cmp_precedence(b))
        .map(|(_, version)| version.clone())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Show {
    All,
    On,
    Off,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct RowFilter {
    pub(super) text: String,
    pub(super) show: Show,
    pub(super) makes: BTreeSet<Make>,
}

impl Default for RowFilter {
    fn default() -> Self {
        Self {
            text: String::new(),
            show: Show::All,
            makes: BTreeSet::new(),
        }
    }
}

/// How many mods the one Build button builds, when the list shows it: it sits in the head of
/// the NEEDS YOU group, so a search or filter that hides every row of that group hides it.
/// `visible` are the rows shown ([`visible`]). Whoever draws a build's bar elsewhere asks
/// this, so a build is never left without one.
pub(super) fn build_in_head(rows: &[Row], visible: &[usize]) -> Option<usize> {
    let waiting = rows.iter().filter(|r| r.need == Some(Need::Build)).count();
    (waiting > 0 && visible.iter().any(|&i| rows[i].group() == Group::NeedsYou)).then_some(waiting)
}

/// Indexes into `rows` that pass the filter; the order of `rows` (grouped, by name) is kept.
pub(super) fn visible_count(rows: &[Row], visible: &[usize]) -> String {
    trf(
        "{shown} of {total} mods shown",
        &[
            ("shown", &visible.len().to_string()),
            ("total", &rows.len().to_string()),
        ],
    )
}

pub(super) fn visible(rows: &[Row], f: &RowFilter) -> Vec<usize> {
    let words: Vec<String> = f.text.split_whitespace().map(str::to_lowercase).collect();
    (0..rows.len())
        .filter(|&i| {
            let r = &rows[i];
            let hay = format!("{} {} {}", r.id, r.title, r.authors).to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
                && match f.show {
                    Show::All => true,
                    Show::On => r.on,
                    Show::Off => !r.on,
                }
                && (f.makes.is_empty() || r.make.is_some_and(|m| f.makes.contains(&m)))
        })
        .collect()
}

// ---------------------------------------------------------------- batches

/// A version the player chose stays in a bulk update; its own Update remains explicit.
pub(super) fn bulk_updatable(r: &Row) -> bool {
    r.local.is_none() && !r.pinned && r.update.is_some()
}

/// Which of the chosen mods can be turned on now, and which can't (and why).
pub(super) fn turn_on_plan<'a>(
    chosen: &[&'a Row],
    native_allowed: bool,
) -> (Vec<&'a Row>, Vec<(&'a Row, String)>) {
    let (mut ok, mut no) = (Vec::new(), Vec::new());
    for r in chosen {
        match &r.need {
            Some(Need::TakenDown(why)) => {
                no.push((*r, trf("taken down: {reason}", &[("reason", why)])))
            }
            _ if r.native() && !native_allowed => no.push((
                *r,
                tr("native code needs your permission first").to_string(),
            )),
            _ => ok.push(*r),
        }
    }
    (ok, no)
}

#[cfg(test)]
mod tests;
