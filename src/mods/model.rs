//! What the Mods screens show, worked out from the registry index and the installed list:
//! pure data in and out, no files, no network, no window. Everything here only reads.
use super::build_run::State as BuildState;
use std::collections::{BTreeSet, HashSet};
use sunrise_launcher::browser::installed::Entry;
use sunrise_launcher::i18n::{tr, trf};
use sunrise_launcher::native_mods;
use sunrise_launcher_smod::index_v2;

pub(super) mod discover_v2;

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

/// Badge shown on Home's Mods tab and Installed for unreviewed mods added outside the registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ManualBadge {
    FromSource,
    Local,
}

impl ManualBadge {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::FromSource => tr("From source"),
            Self::Local => tr("Local"),
        }
    }
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

// ---------------------------------------------------------------- the project page

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

// ---------------------------------------------------------------- installed

/// What stands between an installed mod and working, shown on its row.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Need {
    /// Taken down by the registry: it can't be turned on.
    TakenDown(String),
    /// A native mod, on, whose build for this computer isn't made yet.
    Build,
    /// On, but served by this native mod (its name, or its id when not installed), which is off:
    /// the game leaves it out.
    Host(String),
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct CollectionGroup {
    pub(super) id: String,
    pub(super) name: String,
}

impl From<&sunrise_launcher_smod::index::Collection> for CollectionGroup {
    fn from(collection: &sunrise_launcher_smod::index::Collection) -> Self {
        Self {
            id: collection.id.clone(),
            name: collection.name.clone(),
        }
    }
}

/// A switch's hold on a mod's exact archive.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Hold {
    /// The build it was held for, by the name players read.
    pub(super) build: String,
    pub(super) reason: sunrise_launcher::managed_play::HeldReason,
    /// Local or source bytes: turning it on checks them against the build again.
    pub(super) retry: bool,
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
    /// Why a registry-installed mod's status couldn't be checked: the registry was unreachable
    /// or refused. None while the first read is out.
    pub(super) registry_issue: Option<String>,
    /// The version an update would install.
    pub(super) update: Option<String>,
    /// Installed because another mod needed it.
    pub(super) dependency: bool,
    /// The player chose this version (`sunrise::chosen`): a batch leaves it alone.
    pub(super) pinned: bool,
    /// Not installed by the registry; no registry update.
    pub(super) local: Option<Local>,
    pub(super) icon: Option<String>,
    pub(super) source_listing: Option<sunrise_launcher::browser::sunrise::sources::Source>,
    pub(super) collection: Option<CollectionGroup>,
    pub(super) entry: Option<Entry>,
    /// It can't run on the game this setup plays: the build left it out.
    pub(super) unfit: Option<Unfit>,
    /// The exact Switch receipt keeps the native archive off until a proved update.
    pub(super) held_for: Option<Hold>,
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
    pub(in super::super) const VERSION: f32 = 72.;
    pub(in super::super) const STATUS: f32 = 176.;
    pub(in super::super) const ACTION: f32 = 96.;
    pub(in super::super) const SWITCH: f32 = 44.;
}

/// `row`'s cells. `error` is why its last update failed. The status is the first thing that
/// applies, most pressing first; whatever else applies is in its tooltip.
pub(super) fn cells(row: &Row, error: Option<&str>, building: Option<&BuildState>) -> Cells {
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
        Some(Need::Host(host)) => say(
            "information-circle",
            trf("Needs {host}", &[("host", host)]),
            Tone::Warning,
            trf(
                "{host} serves this mod's code. Turn {host} on for the game to load it.",
                &[("host", host)],
            ),
        ),
        Some(Need::Build) | None => {}
    }
    if let Some(hold) = &row.held_for {
        let vars = [("mod", row.title.as_str()), ("build", hold.build.as_str())];
        say(
            "information-circle",
            hold.reason.label().into(),
            Tone::Warning,
            match hold.reason.detail() {
                Some(detail) => detail.to_owned(),
                None if hold.reason.unproved() => {
                    trf("{mod}'s fit with {build} could not be verified.", &vars)
                }
                None => trf("{mod} needs an update from its author for {build}.", &vars),
            },
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
                tr(super::ADDED_PENDING).into(),
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
            } else {
                trf("Version {version} is out", &[("version", to)])
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
        action: row.update.as_ref().map(|_| RowAction::Update),
        switch: row.held_for.as_ref().is_none_or(|hold| hold.retry)
            && !matches!(row.need, Some(Need::TakenDown(_)))
            && !row.local.as_ref().is_some_and(|l| l.unreadable),
        // A retryable hold is a switch the player may try, not a mod the build dropped.
        unavailable: row.held_for.as_ref().is_some_and(|hold| !hold.retry)
            || row.unfit.is_some()
            || row.left_out.is_some()
            || matches!(
                building,
                Some(BuildState::Failed(_) | BuildState::NeedsUpdate { .. })
            ),
    }
}

impl Row {
    /// Installed from the signed v2 index, so its updates go through the v2 installer.
    pub(super) fn v2_installed(&self) -> bool {
        self.entry.as_ref().is_some_and(|e| e.mod_id.is_some())
    }

    pub(super) fn archive_file(&self) -> Option<&str> {
        self.local
            .as_ref()
            .map(|l| l.file.as_str())
            .or_else(|| self.entry.as_ref().map(|e| e.file.as_str()))
    }

    pub(super) fn source_label(&self) -> &str {
        use sunrise_launcher::github_mods::AddedFrom;
        let Some(local) = &self.local else {
            return tr("Sunrise mods");
        };
        let Some(origin) = &local.github else {
            return tr("Local file");
        };
        match &origin.added_from {
            Some(AddedFrom::Collection { name, .. }) => name,
            Some(AddedFrom::Discover) => tr("Sunrise mods"),
            Some(AddedFrom::Github) => tr("From GitHub"),
            Some(AddedFrom::LocalFolder { .. }) => tr("From source"),
            // Only the paste flow installed GitHub packages without a source tracker.
            None if origin.source.is_none() => tr("From GitHub"),
            None => tr("Source"),
        }
    }

    /// Badge for mods added outside the Sunrise registry: "From source" (GitHub link or local
    /// repository folder) or "Local" (a local `.smod` file).
    pub(crate) fn manual_badge(&self) -> Option<ManualBadge> {
        let local = self.local.as_ref()?;
        match &local.github {
            None => Some(ManualBadge::Local),
            Some(origin) if origin.is_manual() => Some(ManualBadge::FromSource),
            Some(_) => None,
        }
    }

    /// Whether this mod was added manually (via Add from source or as a local `.smod`) and must
    /// not receive automatic or Update all updates.
    pub(super) fn is_manual(&self) -> bool {
        self.manual_badge().is_some()
    }

    /// The native mod that serves this content package's code, when it names one.
    pub(super) fn served_by(&self) -> Option<&str> {
        self.local.as_ref().and_then(|l| l.served_by.as_deref())
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
    /// The signed v2 index, which owns entries keyed by a v2 project ID.
    pub(super) v2: Option<&'a index_v2::Index>,
    /// Why the last read of the v2 index failed; none while it reads or once it has read.
    pub(super) v2_failure: Option<&'a str>,
    /// The exact build the profile plays, which v2 `fits` lists name.
    pub(super) build: Option<&'a str>,
    /// Verified archive hashes already rejected by the SDK for this base build.
    pub(super) unfit_updates: &'a BTreeSet<String>,
    pub(super) enabled: &'a HashSet<String>,
    /// Native mods whose build isn't made.
    pub(super) unbuilt: &'a HashSet<String>,
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
    /// Native code, which needs a build, rather than content only.
    pub(super) native: bool,
    /// The native mod whose code serves this content package (`served_by`).
    pub(super) served_by: Option<String>,
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
            let project =
                w.v2.and_then(|i| sunrise_launcher::browser::sunrise::v2::installed_project(i, e));
            if e.mod_id.is_some() || project.is_some() {
                return Some(v2_row(e, id, project, w));
            }
            // Installed from the old registry, which has closed, and no v2 release has its
            // archive: the v2 index can still take it down by its hash, and it gets no updates.
            let native = sunrise_launcher::native_status::is_native_file(&e.file);
            let on = w.enabled.contains(&id);
            let taken_down = w.v2.and_then(|i| {
                let hash = e.checksum.strip_prefix("SHA-256:").unwrap_or_default();
                i.revoked_by_sha256(hash).map(|r| r.reason.clone())
            });
            let need = if let Some(reason) = taken_down {
                Some(Need::TakenDown(reason))
            } else if native && on && w.unbuilt.contains(&id) {
                Some(Need::Build)
            } else {
                None
            };
            Some(Row {
                title: e.title.clone(),
                id,
                authors: String::new(),
                make: Some(if native { Make::Native } else { Make::Content }),
                version: e.number.clone(),
                on,
                need,
                registry_issue: None,
                update: None,
                dependency: e.dependency,
                pinned: w.chosen.contains_key(&e.project),
                local: None,
                icon: None,
                source_listing: None,
                collection: None,
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
            true if w.unbuilt.contains(&l.id) => Some(Need::Build),
            _ => l
                .served_by
                .as_ref()
                .filter(|host| on && !w.enabled.contains(*host))
                .map(|host| {
                    let local = w.local.iter().find(|h| &h.id == host);
                    Need::Host(local.map_or(host, |h| &h.name).clone())
                }),
        };
        Row {
            id: l.id.clone(),
            title: l.name.clone(),
            authors: String::new(),
            make: match (l.native, l.unreadable) {
                (true, _) => Some(Make::Native),
                (false, false) => Some(Make::Content),
                (false, true) => None,
            },
            version: l.version.clone(),
            on,
            need,
            registry_issue: None,
            update: None,
            dependency: false,
            pinned: false,
            local: Some(l.clone()),
            icon: None,
            source_listing: None,
            collection: None,
            entry: None,
            unfit: None,
            held_for: None,
            left_out: None,
        }
    }));
    sort(&mut out);
    out
}

/// A row for an entry the signed v2 index installed: its project names it, a takedown of the
/// installed archive needs action, and an update is the newest release listed as fitting the
/// profile's exact build.
/// `p` is the row's v2 project, when the index lists it.
fn v2_row(e: &Entry, id: String, p: Option<&index_v2::Project>, w: &World) -> Row {
    let native = sunrise_launcher::native_status::is_native_file(&e.file);
    let on = w.enabled.contains(&id);
    let hash = e.checksum.strip_prefix("SHA-256:").unwrap_or_default();
    let project_id = p.map_or(e.project.as_str(), |p| p.id.as_str());
    let taken_down =
        w.v2.and_then(|i| i.revoked(project_id, Some(hash)))
            .map(|r| r.reason.clone());
    let need = if let Some(reason) = taken_down {
        Some(Need::TakenDown(reason))
    } else if native && on && w.unbuilt.contains(&id) {
        Some(Need::Build)
    } else {
        None
    };
    let update = w.v2.zip(p).zip(w.build).and_then(|((i, p), build)| {
        sunrise_launcher::browser::sunrise::v2::newer_release(
            i,
            p,
            &e.version,
            build,
            w.unfit_updates,
        )
        .map(|r| r.version.clone())
    });
    Row {
        title: p.map_or_else(|| e.title.clone(), |p| p.name.clone()),
        id,
        authors: p.map(|p| p.authors.join(", ")).unwrap_or_default(),
        make: match (native, p.map(|p| &p.kind)) {
            (true, _) | (_, Some(index_v2::Kind::NativeMod)) => Some(Make::Native),
            (false, Some(_)) => Some(Make::Content),
            (false, None) => None,
        },
        version: e.number.clone(),
        on,
        need,
        registry_issue: w.v2_failure.map(str::to_owned),
        update,
        dependency: e.dependency,
        pinned: w.chosen.contains_key(&e.project),
        local: None,
        icon: None,
        source_listing: None,
        collection: None,
        entry: Some(e.clone()),
        unfit: None,
        held_for: None,
        left_out: None,
    }
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

/// Installed uses collection order first, keeping the status and name order within each group.
pub(super) fn group_installed(rows: &mut [Row]) {
    sort(rows);
    rows.sort_by(|a, b| match (&a.collection, &b.collection) {
        (Some(a), Some(b)) => a.id.cmp(&b.id),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
}

pub(super) fn installed_groups<'a>(
    rows: &'a [Row],
    visible: &'a [usize],
) -> impl Iterator<Item = (Option<&'a str>, &'a [usize])> {
    let collection = |i: usize| rows[i].collection.as_ref();
    visible
        .chunk_by(move |a, b| collection(*a).map(|c| &c.id) == collection(*b).map(|c| &c.id))
        .map(move |group| (collection(group[0]).map(|c| c.name.as_str()), group))
}

pub(super) struct FilterCounts {
    pub(super) shows: Vec<(Show, usize)>,
    pub(super) kinds: Vec<(Make, usize)>,
}

/// Omit empty filters and their controls; a filter that disappears cannot remain selected.
pub(super) fn counted_filters(rows: &[Row], filter: &mut RowFilter) -> FilterCounts {
    let on = rows.iter().filter(|row| row.on).count();
    let shows: Vec<_> = [
        (Show::All, rows.len()),
        (Show::On, on),
        (Show::Off, rows.len() - on),
    ]
    .into_iter()
    .filter(|(_, count)| *count != 0)
    .collect();
    if !shows.iter().any(|(show, _)| *show == filter.show) {
        filter.show = Show::All;
    }
    let kinds: Vec<_> = Make::ALL
        .into_iter()
        .map(|kind| {
            (
                kind,
                rows.iter().filter(|row| row.make == Some(kind)).count(),
            )
        })
        .filter(|(_, count)| *count != 0)
        .collect();
    filter
        .makes
        .retain(|make| kinds.iter().any(|(kind, _)| kind == make));
    FilterCounts { shows, kinds }
}

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
pub(super) fn turn_on_plan<'a>(chosen: &[&'a Row]) -> (Vec<&'a Row>, Vec<(&'a Row, String)>) {
    let (mut ok, mut no) = (Vec::new(), Vec::new());
    for r in chosen {
        match &r.need {
            Some(Need::TakenDown(why)) => {
                no.push((*r, trf("taken down: {reason}", &[("reason", why)])))
            }
            _ => ok.push(*r),
        }
    }
    // A served mod can't come on in a batch that leaves its host off.
    let mut kept = Vec::new();
    for r in ok {
        let host = r
            .served_by()
            .and_then(|host| no.iter().find(|(held, _)| held.id == host));
        match host.map(|(held, _)| held.title.clone()) {
            Some(host) => no.push((r, trf("{host} serves it and stays off", &[("host", &host)]))),
            None => kept.push(r),
        }
    }
    (kept, no)
}

#[cfg(test)]
mod tests;
