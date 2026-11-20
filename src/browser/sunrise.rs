//! Sunrise's own registry (docs/modding/src/reference/registry-index.md): mods, modpacks and resource packs made for
//! Sunrise, reviewed as pull requests in sunrise-org/sunrise-mods. One file lists them all,
//! `index.json`; every download is checked against it before it reaches the game folder
//! ([`index::check_file`]: size, SHA-256, the author's signature and the file's own manifest; the game refuses
//! content it can't load, with its own error), and files the index says were taken down are refused and turned off.
use super::{
    details::{Details, GitHub, Link},
    download, installed, mod_list, FileRef, Kind, Project, Query, Source, SourceId, Version,
};
use crate::i18n::{tr, trf};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use sunrise_launcher_smod::index::{self, Index, Trust};

/// Where the registry publishes its index (GitHub Pages of sunrise-org/sunrise-mods).
pub const INDEX_URL: &str = sunrise_launcher_addresses::MOD_INDEX;
/// The registry's pages on the website, `<SITE>/<id>`.
const SITE: &str = sunrise_launcher_addresses::MOD_PAGES;
/// The report form of a project.
const REPORT: &str = sunrise_launcher_addresses::MOD_REPORT;
/// The loader name the browser gives registry versions, so `support` and `pick` know them.
pub const LOADER: &str = "sunrise";
/// How long a fetched index is used before it is fetched again.
const FRESH: Duration = Duration::from_secs(600);
/// Larger than any index should be.
const MAX_INDEX: u64 = 16 << 20;
/// The last index fetched, kept in the game folder for when the registry can't be reached.
const CACHE: &str = "mod-browser/sunrise-index.json";
/// Players' ratings of every project (the registry service, crates/sunrise-mod-registry).
const RATINGS_URL: &str = "";
/// Where a player rates a project: the site's GitHub sign-in, which comes back to its page.
/// The launcher has no GitHub account of its own.
const RATE: &str = "";

/// Where the index comes from: `SUNRISE_MOD_INDEX` (a file or an `https://` address, for a
/// test registry), else [`INDEX_URL`].
fn location() -> String {
    std::env::var("SUNRISE_MOD_INDEX")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| INDEX_URL.to_owned())
}

/// True when the index is a local file: its files may then be local too (`file://`), for
/// trying a registry out before it is published.
pub fn local() -> bool {
    !location().starts_with("https://")
}

/// The index read last, and whether the registry itself gave it in this run. The copy kept in
/// the game folder is a file anything there can write (a modpack, a mod, the player), so it is
/// good for showing what was installed and never for saying a file is the registry's.
struct Loaded {
    at: Instant,
    index: Arc<Index>,
    fetched: bool,
}

impl Loaded {
    /// The index as [`fetched_index`] gives it.
    fn fetched(&self) -> Option<Arc<Index>> {
        self.fetched.then(|| self.index.clone())
    }
}

static LOADED: Mutex<Option<Loaded>> = Mutex::new(None);

/// The index, asked for again now whatever was read last: "Check for update".
pub fn refreshed_index() -> Result<Arc<Index>, String> {
    if let Some(loaded) = LOADED.lock().unwrap().as_mut() {
        loaded.at = Instant::now().checked_sub(FRESH).unwrap_or(loaded.at);
    }
    index()
}

/// The registry mods `entries` hold, as the install planner reads them.
pub fn held_mods(entries: &[installed::Entry]) -> Vec<index::resolve::InstalledMod> {
    entries
        .iter()
        .filter(|e| e.source == SourceId::Sunrise)
        .map(|e| index::resolve::InstalledMod {
            id: e.project.clone(),
            version: e.version.clone(),
            sha256: e.checksum.strip_prefix("SHA-256:").map(str::to_owned),
        })
        .collect()
}

/// Whether the registry's `version` is one this build can install for the Minecraft version
/// `game`: made for it, and asking for no capability this build doesn't know. The planner's
/// native fit decision.
pub fn fits_game(version: &index::Version, game: &str) -> bool {
    version.game == game && version.capabilities.unknown().is_empty()
}

/// The dependencies `ids` need, by the version the index would choose of each for `game`,
/// all the way down.
fn needed<'a>(index: &'a Index, ids: &[String], game: &str) -> Vec<&'a index::Dependency> {
    let mut found: Vec<&index::Dependency> = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    let mut todo: Vec<&str> = ids.iter().map(String::as_str).collect();
    while let Some(id) = todo.pop() {
        if seen.contains(&id) {
            continue;
        }
        seen.push(id);
        let Some(version) = index
            .project(id)
            .and_then(|project| index.installable(project, game))
        else {
            continue;
        };
        for dep in version
            .dependencies
            .iter()
            .flatten()
            .filter(|d| !d.optional)
        {
            found.push(dep);
            todo.push(&dep.id);
        }
    }
    found
}

/// Whether the index has a version of what `dep` asks for that runs on `game`.
fn available(index: &Index, dep: &index::Dependency, game: &str) -> bool {
    let Ok(range) = semver::VersionReq::parse(&dep.version) else {
        return true;
    };
    index.project(&dep.id).is_some_and(|project| {
        project.versions.iter().any(|v| {
            fits_game(v, game)
                && index.revoked(&project.id, &v.file.sha256).is_none()
                && semver::Version::parse(&v.version).is_ok_and(|n| range.matches(&n))
        })
    })
}

/// The installed mod `m`'s own requirement among `deps`, when it pins one of them.
fn pins<'a>(
    index: &'a Index,
    m: &index::resolve::InstalledMod,
    deps: &[&index::Dependency],
) -> Option<&'a index::Dependency> {
    index
        .project(&m.id)?
        .versions
        .iter()
        .find(|v| v.version == m.version)?
        .dependencies
        .iter()
        .flatten()
        .find(|d| !d.optional && deps.iter().any(|needed| needed.id == d.id))
}

/// Why the planner refused `ids` (its words are `reason`), by what the index says: a mod they
/// need that has no version for `game`, or installed mods that pin what they need. Anything
/// else keeps the planner's words.
pub fn explain(
    index: &Index,
    ids: &[String],
    held: &[index::resolve::InstalledMod],
    game: &str,
    reason: String,
) -> installed::InstallError {
    let deps = needed(index, ids, game);
    let name = |id: &str| index.project(id).map_or(id, |p| p.name.as_str()).to_owned();
    if let Some(dep) = deps.iter().find(|d| !available(index, d, game)) {
        return installed::InstallError::Unavailable {
            dependency: name(&dep.id),
            reason,
        };
    }
    let mods: Vec<String> = held
        .iter()
        .filter(|m| !ids.contains(&m.id))
        .filter(|m| pins(index, m, &deps).is_some())
        .map(|m| name(&m.id))
        .collect();
    if mods.is_empty() {
        installed::InstallError::Refused(reason)
    } else {
        installed::InstallError::Conflict { mods, reason }
    }
}

/// Plans every one of `ids` together, once, and refuses the whole set with the planner's reason
/// if any of them can't be installed with the others: nothing is downloaded before that.
pub fn check_set(
    index: &Index,
    ids: &[String],
    held: &[index::resolve::InstalledMod],
    game: &str,
) -> Result<index::resolve::InstallPlan, installed::InstallError> {
    let requests: Vec<_> = ids
        .iter()
        .map(|id| index::resolve::Request {
            id: id.clone(),
            version: None,
        })
        .collect();
    let plan = index::resolve::install(
        index,
        &requests,
        held,
        game,
        sunrise_launcher_smod::manifest::Side::Both,
        |_, v| fits_game(v, game),
    );
    for id in ids {
        if let Some(index::resolve::RequestResult::Refused { reason }) = plan.requested.get(id) {
            let name = index.project(id).map_or(id.as_str(), |p| p.name.as_str());
            return Err(explain(
                index,
                ids,
                held,
                game,
                format!("{name} can't be installed with the others: {reason}"),
            ));
        }
    }
    Ok(plan)
}

/// Whether installing `ids` would put native code in, by the plan the install itself follows:
/// one of them, or one of what they need, that the plan installs or updates is a native mod.
/// A native mod already installed that the plan keeps puts nothing in. False when the plan
/// can't be made: the install says why.
pub fn brings_native(
    index: &Index,
    ids: &[String],
    held: &[index::resolve::InstalledMod],
    game: &str,
) -> bool {
    check_set(index, ids, held, game).is_ok_and(|plan| {
        let installs = plan.install.iter().map(|m| &m.id);
        let updates = plan.update.iter().map(|u| &u.id);
        installs.chain(updates).any(|id| {
            index
                .project(id)
                .is_some_and(|p| p.kind == index::Kind::NativeMod)
        })
    })
}

/// An installed mod that installing would break: one the plan updates a dependency out from
/// under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Broken {
    pub id: String,
    /// What it needs that the plan would replace, as the registry names it ("flywheel ^0.3").
    pub needs: String,
}

/// The installed mods that installing `ids` would break: the plan refuses because of what they
/// need, and installing works without them. By the index; one installed file the index doesn't
/// know is not judged. Empty when the plan works, or fails for another reason.
pub fn would_break(
    index: &Index,
    ids: &[String],
    held: &[index::resolve::InstalledMod],
    game: &str,
) -> Vec<Broken> {
    if check_set(index, ids, held, game).is_ok() {
        return Vec::new();
    }
    let deps = needed(index, ids, game);
    let works_without = |gone: &[&index::resolve::InstalledMod]| {
        let rest: Vec<_> = held
            .iter()
            .filter(|h| !gone.iter().any(|g| g.id == h.id))
            .cloned()
            .collect();
        check_set(index, ids, &rest, game).is_ok()
    };
    // Every installed mod that needs what the install needs is a suspect; two may pin the
    // same dependency, so removing one alone doesn't make the plan work.
    let mut gone: Vec<&index::resolve::InstalledMod> = held
        .iter()
        .filter(|m| !ids.contains(&m.id))
        .filter(|m| pins(index, m, &deps).is_some())
        .collect();
    if !works_without(&gone) {
        return Vec::new();
    }
    // Then only those the plan needs gone: one that can stay is put back.
    let mut i = 0;
    while i < gone.len() {
        let mut fewer = gone.clone();
        fewer.remove(i);
        if works_without(&fewer) {
            gone = fewer;
        } else {
            i += 1;
        }
    }
    gone.into_iter()
        .filter_map(|m| {
            let dep = pins(index, m, &deps)?;
            Some(Broken {
                id: m.id.clone(),
                needs: format!("{} {}", dep.id, dep.version),
            })
        })
        .collect()
}

/// The index read last in this run, without asking the network: for what must not wait.
pub fn known_index() -> Option<Arc<Index>> {
    LOADED.lock().unwrap().as_ref().map(|l| l.index.clone())
}

/// [`known_index`], only when the registry answered over HTTPS in this run: neither the local
/// test registry nor the game-folder cache can vouch for a file or a takedown decision.
pub fn fetched_index() -> Option<Arc<Index>> {
    LOADED.lock().unwrap().as_ref().and_then(Loaded::fetched)
}

/// Whether the registry's project `id` is a native mod, by the index already read (never the
/// network); false when none was read.
pub fn is_native_mod(id: &str) -> bool {
    known_index()
        .and_then(|index| Some(index.project(id)?.kind == index::Kind::NativeMod))
        .unwrap_or(false)
}

/// How many of the registry's projects this Sunrise left out because none of their versions
/// passes the index's rules (mods published for the earlier mod system), by the index already
/// read (never the network), for the Minecraft version `game`: None when no index was read.
pub fn projects_left_out(game: &str) -> Option<usize> {
    let index = known_index()?;
    // With those listed whose versions for this game all are (`Blocked::EarlierSystem`), which
    // the list hides too.
    let mut ids: Vec<&str> = index
        .left_out
        .iter()
        .filter(|l| l.version.is_none() && index.project(&l.id).is_none())
        .map(|l| l.id.as_str())
        .chain(
            index
                .projects
                .iter()
                .filter(|p| matches!(blocked(&index, p, game), Some(Blocked::EarlierSystem)))
                .map(|p| p.id.as_str()),
        )
        .collect();
    ids.sort_unstable();
    ids.dedup();
    Some(ids.len())
}

/// The GitHub stars of the registry's project `id`, by the index already read (never the
/// network): None when it has none, when its source isn't a GitHub repository, or when no index
/// was read.
pub fn github_stars(id: &str) -> Option<u64> {
    let index = known_index()?;
    let p = index.project(id)?;
    (p.stars > 0 && github_repo(&p.source).is_some()).then_some(p.stars)
}

/// The index: the one fetched in the last ten minutes, else fetched again (and kept in the game
/// folder), else the copy kept last time.
pub fn index() -> Result<Arc<Index>, String> {
    let at = location();
    if at.is_empty() {
        return Err("The Sunrise mod registry is not available yet.".into());
    }
    if let Some(loaded) = LOADED.lock().unwrap().as_ref() {
        if loaded.at.elapsed() < FRESH {
            return Ok(loaded.index.clone());
        }
    }
    let cache = crate::backend::game_home().join(CACHE);
    let local = !at.starts_with("https://");
    let loaded = load(fetch_index(&at, local), &cache, local)?;
    let index = loaded.index.clone();
    *LOADED.lock().unwrap() = Some(loaded);
    Ok(index)
}

/// What [`index`] keeps: the registry's answer `fetched` (written to `cache` for next time), else
/// the copy in `cache`, marked as not the registry's own answer.
fn load(
    fetched: Result<(Index, String), String>,
    cache: &Path,
    local: bool,
) -> Result<Loaded, String> {
    let (index, fetched) = match fetched {
        Ok((index, text)) => {
            if let Some(dir) = cache.parent() {
                let _ = fs::create_dir_all(dir);
            }
            let _ = fs::write(cache, text);
            (index, !local)
        }
        Err(e) => {
            let kept = fs::read_to_string(cache).map_err(|_| e.clone())?;
            let index = match local {
                true => Index::parse_local(&kept),
                false => Index::parse(&kept),
            }
            .map_err(|_| e)?;
            (index, false)
        }
    };
    Ok(Loaded {
        at: Instant::now(),
        index: Arc::new(index),
        fetched,
    })
}

fn fetch_index(at: &str, local: bool) -> Result<(Index, String), String> {
    if local {
        let text = fs::read_to_string(at).map_err(|e| format!("{at}: {e}"))?;
        return Ok((Index::parse_local(&text)?, text));
    }
    let text = fetch_live_text(at)?;
    Ok((parse_live(&text)?, text))
}

fn fetch_live_text(at: &str) -> Result<String, String> {
    use std::io::Read;
    let spec = crate::proxy::spec_for(at);
    let agent = crate::net::keep("sunrise-index", spec.as_deref(), |proxy| {
        let builder = ureq::AgentBuilder::new()
            .https_only(true)
            .timeout(Duration::from_secs(10));
        match proxy {
            Some(proxy) => builder.proxy(proxy).build(),
            None => builder.build(),
        }
    });
    let response = agent.get(at).call().map_err(|e| match e {
        ureq::Error::Status(code, _) => format!("The mod registry answered {code}."),
        ureq::Error::Transport(t) => format!("The mod registry can't be reached: {t}"),
    })?;
    if response.status() != 200 || response.header("Content-Range").is_some() {
        return Err(tr("The mod registry didn't send the full index.").to_string());
    }
    let mut text = String::new();
    response
        .into_reader()
        .take(MAX_INDEX + 1)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() as u64 > MAX_INDEX {
        return Err("The mod registry index is too large.".into());
    }
    Ok(text)
}

/// The authoritative wire response must distinguish a missing list from an empty list.
/// Keep the tolerant reader for display, but never mark a projection as fetched.
fn parse_live(text: &str) -> Result<Index, String> {
    #[derive(serde::Deserialize)]
    struct Complete {
        #[serde(rename = "projects")]
        _projects: Vec<serde::de::IgnoredAny>,
        #[serde(rename = "revoked")]
        _revoked: Vec<serde::de::IgnoredAny>,
    }
    serde_json::from_str::<Complete>(text).map_err(|error| {
        trf(
            "The mod registry didn't send a complete index: {error}",
            &[("error", &error.to_string())],
        )
    })?;
    Index::parse(text)
}

/// No cached catalogue, display filtering, or locally configured substitute can authorize a
/// switch. The complete response seals negative evidence too (a missing version/takedown).
fn authoritative_index() -> Result<(Index, String), String> {
    let text = fetch_live_text(INDEX_URL)?;
    let index = parse_authoritative(&text)?;
    Ok((index, index::sha256(text.as_bytes())))
}

fn parse_authoritative(text: &str) -> Result<Index, String> {
    #[derive(serde::Deserialize)]
    struct Complete {
        schema: u32,
        #[serde(rename = "projects")]
        _projects: Vec<serde::de::IgnoredAny>,
        #[serde(rename = "revoked")]
        _revoked: Vec<serde::de::IgnoredAny>,
    }
    let complete: Complete = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if complete.schema != index::SCHEMA {
        return Err(tr("The mod registry didn't send the full index.").into());
    }
    let index: Index = serde_json::from_str(text).map_err(|e| e.to_string())?;
    // Do not silently discard any row: it can name a dependency or a revocation.
    index.check()?;
    Ok(index)
}

/// A project's ratings: 1 to 5 stars, their mean and how many.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rating {
    pub average: f32,
    pub count: u64,
}

/// When the ratings were fetched last, and every rated project's [`Rating`] (None: that failed).
type Ratings = Option<Arc<HashMap<String, Rating>>>;
static RATINGS: Mutex<Option<(Instant, Ratings)>> = Mutex::new(None);

/// A project's rating as fetched last, without asking the network: None when the ratings
/// aren't known (not fetched, or the service didn't answer), `Some(None)` when it has none.
pub fn rating(id: &str) -> Option<Option<Rating>> {
    let ratings = RATINGS.lock().unwrap();
    let known = ratings.as_ref()?.1.as_ref()?;
    Some(known.get(id).copied())
}

/// The page that signs the player in with GitHub on the site and lets them rate `id`.
pub fn rate_url(id: &str) -> String {
    if RATE.is_empty() {
        return String::new();
    }
    format!("{RATE}{id}")
}

/// Fetches the ratings again when the last try is ten minutes old: from `SUNRISE_MOD_RATINGS`
/// (a file or an `https://` address, for a test registry), else [`RATINGS_URL`]; a local test
/// registry has none unless given. After a failure the browser shows downloads only.
fn refresh_ratings() {
    if RATINGS
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|(at, _)| at.elapsed() < FRESH)
    {
        return;
    }
    let given = std::env::var("SUNRISE_MOD_RATINGS")
        .ok()
        .filter(|v| !v.is_empty());
    if given.is_none() && local() {
        return;
    }
    let at = given.unwrap_or_else(|| RATINGS_URL.to_owned());
    let text = if at.starts_with("https://") {
        download::agent(&at).get(&at).call().ok().and_then(|r| {
            use std::io::Read;
            let mut text = String::new();
            r.into_reader()
                .take(MAX_INDEX)
                .read_to_string(&mut text)
                .ok()
                .map(|_| text)
        })
    } else {
        fs::read_to_string(&at).ok()
    };
    let fetched = text.map(|t| Arc::new(parse_ratings(&t)));
    *RATINGS.lock().unwrap() = Some((Instant::now(), fetched));
}

/// `{"projects": {"<id>": {"average", "count"}}}`, keeping only well-formed entries.
fn parse_ratings(text: &str) -> HashMap<String, Rating> {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(text) else {
        return HashMap::new();
    };
    let Some(projects) = json["projects"].as_object() else {
        return HashMap::new();
    };
    projects
        .iter()
        .filter_map(|(id, r)| {
            let average = r["average"].as_f64().filter(|a| (1.0..=5.0).contains(a))?;
            let count = r["count"].as_u64().filter(|&c| c > 0)?;
            index::valid_id(id).then(|| {
                (
                    id.clone(),
                    Rating {
                        average: average as f32,
                        count,
                    },
                )
            })
        })
        .collect()
}

/// The browser's kind for a registry kind; None for one the launcher doesn't install yet
/// (kinds newer than this launcher).
fn kind_of(kind: &index::Kind) -> Option<Kind> {
    match kind {
        index::Kind::Mod => Some(Kind::Mod),
        index::Kind::ResourcePack => Some(Kind::ResourcePack),
        index::Kind::Modpack => Some(Kind::Modpack),
        // Both go in shaderpacks/: the game reads `.sshader` packs and shaders/-folder zips.
        index::Kind::Shader | index::Kind::ShaderPack => Some(Kind::Shader),
        // Mixin mods: installed beside the others, compiled into the game by `native_mods`.
        index::Kind::NativeMod => Some(Kind::Mod),
        // Into a world the player picks ([`installed::install_datapack`]).
        index::Kind::DataPack => Some(Kind::DataPack),
        index::Kind::Other(_) => None,
    }
}

/// The registry kind whose category list a browser kind filters by (`page::categories`): a
/// native mod's list is a mod's, a shaders/-folder pack's a shader pack's.
fn categories_of(kind: Kind) -> &'static [&'static str] {
    index::page::categories(&match kind {
        Kind::Mod => index::Kind::Mod,
        Kind::ResourcePack => index::Kind::ResourcePack,
        Kind::Modpack => index::Kind::Modpack,
        Kind::Shader => index::Kind::Shader,
        Kind::DataPack => index::Kind::DataPack,
    })
}

/// The most tags the Filters panel offers for a kind.
const MAX_TAGS_SHOWN: usize = 30;

/// The tags the registry's projects of `kind` carry, plain ones only (as the site keeps them):
/// the most used first, then by name.
fn tags_of(index: &Index, kind: Kind) -> Vec<String> {
    let plain = |t: &str| {
        t.len() <= index::page::MAX_TAG
            && t.bytes().next().is_some_and(|b| b != b'-')
            && t.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    };
    let mut uses: HashMap<&str, usize> = HashMap::new();
    for p in index
        .projects
        .iter()
        .filter(|p| kind_of(&p.kind) == Some(kind))
    {
        let mut own: Vec<&str> = p
            .tags
            .iter()
            .map(String::as_str)
            .filter(|t| plain(t))
            .collect();
        own.sort_unstable();
        own.dedup();
        for t in own {
            *uses.entry(t).or_default() += 1;
        }
    }
    let mut tags: Vec<(&str, usize)> = uses.into_iter().collect();
    tags.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    tags.into_iter()
        .take(MAX_TAGS_SHOWN)
        .map(|(t, _)| t.to_owned())
        .collect()
}

/// Where the registry's mod policy is: who owns and answers for a project, takedowns.
pub const POLICY: &str = sunrise_launcher_addresses::MOD_POLICY;

/// Whether the Sunrise project made `p` (the mod policy's "Official": author Sunrise, source in
/// the horizon-rust organisation).
pub fn official(p: &index::Project) -> bool {
    p.authors == ["Sunrise"] && p.source.starts_with("https://github.com/horizon-rust/")
}

/// Whether the registry's project `id` is Sunrise's own ([`official`]), by the index already read
/// (never the network); false when none was read.
pub fn is_official(id: &str) -> bool {
    official_in(known_index().as_deref(), id)
}

fn official_in(index: Option<&Index>, id: &str) -> bool {
    index.and_then(|i| i.project(id)).is_some_and(official)
}

/// What the player reads about a registry project before installing it: who made it, whether its
/// files are signed, and for a mod what it may do, in the game's words (Options > Mods). A native
/// mod says it runs with the player's rights, and how its version for `game` fits `instance`.
pub fn trust_and_capabilities(
    id: &str,
    instance: Option<&crate::instances::Instance>,
    game: &str,
) -> Vec<String> {
    let Ok(index) = index() else {
        return Vec::new();
    };
    let Some(p) = index.project(id) else {
        return Vec::new();
    };
    // Shown to players: in their words, through `tr`; the policy is a link beside them
    // ([`POLICY`]), not a bare address in the text.
    let mut lines = vec![if official(p) {
        tr("Made by Sunrise").to_string()
    } else {
        trf(
            "Made by {authors}, not by the Sunrise project",
            &[("authors", &p.authors.join(", "))],
        )
    }];
    lines.push(match &p.key {
        Some(_) => tr("Signed by its author: Sunrise checks it really is theirs").to_string(),
        None => tr(
            "Not signed by its author: Sunrise checks the download arrives complete and unchanged",
        )
        .to_string(),
    });
    if p.kind == index::Kind::NativeMod {
        lines.push(
            tr("Native code: it is built into the game and runs with full access to your computer. Listed after a person reviewed its entry and file").to_string(),
        );
        if let Some(v) = p.versions.iter().find(|v| v.game == game) {
            let pin = v.api.split('/').nth(1).unwrap_or_default();
            lines.push(
                instance
                    .and_then(|i| crate::native_fit::line(i, pin))
                    .unwrap_or_else(|| {
                        trf(
                            "Built for {build}",
                            &[("build", &crate::native_mods::sdk_cache::known_name(pin))],
                        )
                    }),
            );
        }
    }
    // What Install installs ([`newest`]), so the page describes that version.
    if let Some(v) = index
        .installable(p, game)
        .filter(|_| p.kind == index::Kind::Mod)
    {
        lines.extend(
            v.capabilities
                .known()
                .iter()
                .map(|c| trf("May {what}", &[("what", tr(c.describe()))])),
        );
        // Capabilities this Sunrise lacks are the project's verdict (`Project::blocked`), said once.
        if v.capabilities.is_empty() {
            lines.push(tr("Asks for nothing beyond its own log").to_string());
        }
    }
    lines
}

/// Whether native mod `id`'s version for `game` was made for another game build than the one
/// `instance` plays: the build then checks whether it fits before it runs. False when either
/// side can't be told.
pub fn made_for_another_build(id: &str, instance: &crate::instances::Instance, game: &str) -> bool {
    let Ok(index) = index() else {
        return false;
    };
    let pin = index
        .project(id)
        .filter(|p| p.kind == index::Kind::NativeMod)
        .and_then(|p| p.versions.iter().find(|v| v.game == game))
        .and_then(|v| v.api.split('/').nth(1).map(str::to_owned));
    match (pin, crate::native_fit::plays(instance)) {
        (Some(pin), Some(plays)) => {
            crate::native_fit::fit(&pin, &plays) != crate::native_fit::Fit::Fits
        }
        _ => false,
    }
}

/// How long the registry lets a summary be: it cuts a longer one there, mid-word.
const SUMMARY_CUT: usize = 160;

/// A registry summary as players read it: one the registry cut mid-word ends at its last whole
/// word, with "…".
fn summary(text: &str) -> String {
    let cut = text.chars().count() >= SUMMARY_CUT
        && text.chars().last().is_some_and(char::is_alphanumeric);
    match text.rfind(char::is_whitespace).filter(|_| cut) {
        Some(end) => format!("{}…", text[..end].trim_end_matches([',', ';', ':'])),
        None => text.to_owned(),
    }
}

/// Why a project has versions for this game but none this Sunrise may install.
enum Blocked {
    /// They ask for capabilities this Sunrise doesn't have: made for the earlier mod system.
    EarlierSystem,
    /// They were taken down, for this reason.
    TakenDown(String),
}

fn blocked(index: &Index, p: &index::Project, game: &str) -> Option<Blocked> {
    let for_game: Vec<&index::Version> = p.versions.iter().filter(|v| v.game == game).collect();
    if for_game.is_empty() || index.installable(p, game).is_some() {
        return None;
    }
    let revoked = for_game
        .iter()
        .find_map(|v| index.revoked(&p.id, &v.file.sha256));
    Some(match revoked {
        Some(r) if for_game.iter().all(|v| v.capabilities.unknown().is_empty()) => {
            Blocked::TakenDown(r.reason.clone())
        }
        _ => Blocked::EarlierSystem,
    })
}

fn project_of(index: &Index, p: &index::Project, game: &str) -> Option<Project> {
    let mut game_versions: Vec<String> = p.versions.iter().map(|v| v.game.clone()).collect();
    game_versions.dedup();
    // Versions for this game, none of which this Sunrise may install: say why, once, here.
    let blocked = blocked(index, p, game).map(|why| match why {
        Blocked::TakenDown(reason) => {
            trf("Taken down: {reason}", &[("reason", &reason)]).to_string()
        }
        Blocked::EarlierSystem => {
            tr("Made for Sunrise's earlier mod system: it can't be installed yet.").to_string()
        }
    });
    Some(Project {
        source: SourceId::Sunrise,
        id: p.id.clone(),
        slug: p.id.clone(),
        title: p.name.clone(),
        summary: summary(&p.summary),
        author: p.authors.join(", "),
        downloads: p.downloads,
        page: format!("{SITE}/{}", p.id),
        icon: p.icon.clone(),
        license: p.license.clone(),
        kind: kind_of(&p.kind)?,
        game_versions,
        loaders: vec![LOADER.into()],
        blocked,
    })
}

/// Version `v` of `p` as the browser shows it; blocked when taken down.
fn version_of(index: &Index, p: &index::Project, v: &index::Version) -> Version {
    Version {
        id: v.version.clone(),
        number: v.version.clone(),
        published: v.published.clone(),
        game_versions: vec![v.game.clone()],
        loaders: vec![LOADER.into()],
        files: vec![FileRef {
            url: v.file.url.clone(),
            name: v.file.name.clone(),
            size: v.file.size,
            sha512: None,
            sha256: Some(v.file.sha256.clone()),
            sha1: None,
            primary: true,
        }],
        required: v.requires.iter().map(|r| r.id.clone()).collect(),
        blocked: index
            .revoked(&p.id, &v.file.sha256)
            .map(|r| format!("taken down: {}", r.reason))
            .or_else(|| unsupported_capabilities(&v.capabilities)),
    }
}

fn unsupported_capabilities(capabilities: &index::Capabilities) -> Option<String> {
    let unknown = capabilities.unknown();
    (!unknown.is_empty()).then(|| {
        trf(
            "Can't install: it was made for Sunrise's earlier mod system ({names})",
            &[("names", &unknown.join(", "))],
        )
    })
}

/// A GitHub login as GitHub allows it (letters, digits, single dashes, at most 39), so an index
/// entry can't turn a link into something else.
fn github_login(login: &str) -> bool {
    (1..=39).contains(&login.len())
        && login
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && !login.starts_with('-')
        && !login.ends_with('-')
        && !login.contains("--")
}

/// The GitHub repository a project's `source` is in, `https://github.com/<owner>/<repo>`: a
/// deeper path (a folder of a monorepo) or a trailing `.git` names the same repository. None
/// when the source isn't on GitHub, or its owner or name isn't one GitHub allows.
pub fn github_repo(source: &str) -> Option<String> {
    let path = source.strip_prefix("https://github.com/")?;
    let mut parts = path.split(['/', '?', '#']);
    let owner = parts.next().filter(|o| github_login(o))?;
    let repo = parts.next()?;
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    let allowed = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.');
    if repo.is_empty() || repo.len() > 100 || repo.starts_with('.') || !repo.bytes().all(allowed) {
        return None;
    }
    Some(format!("https://github.com/{owner}/{repo}"))
}

/// The words of a query found in a project's id, name, summary, authors or tags.
fn matches(p: &index::Project, text: &str) -> bool {
    let hay = format!(
        "{} {} {} {} {}",
        p.id,
        p.name,
        p.summary,
        p.authors.join(" "),
        p.tags.join(" ")
    )
    .to_lowercase();
    text.split_whitespace()
        .all(|word| hay.contains(&word.to_lowercase()))
}

/// The sorts the registry can honor: it has downloads and each version's publish day, but no
/// follows and no relevance score (its search is a word match).
pub const SORTS: [super::details::Sort; 3] = [
    super::details::Sort::Downloads,
    super::details::Sort::Newest,
    super::details::Sort::Updated,
];

/// The projects of the query's kind whose words match and that have every category and tag the
/// filters ask for, for this game unless `everything`: the
/// most downloaded first, or the newest (first published) or most recently updated (latest
/// version) when the query asks, ties by downloads; those no version of which installs here are
/// left out, or last with `everything`.
fn search(index: &Index, query: &Query) -> Vec<Project> {
    use super::details::Sort;
    let mut found: Vec<&index::Project> = index
        .projects
        .iter()
        .filter(|p| kind_of(&p.kind) == Some(query.kind) && matches(p, &query.text))
        .filter(|p| {
            query
                .filters
                .categories
                .iter()
                .all(|c| p.categories.contains(c))
                && query.filters.tags.iter().all(|t| p.tags.contains(t))
        })
        .filter(|p| query.everything || p.versions.iter().any(|v| v.game == query.game))
        .collect();
    let day = |p: &index::Project, newest: bool| {
        let days = p.versions.iter().map(|v| v.published.as_str());
        if newest { days.min() } else { days.max() }
            .unwrap_or("")
            .to_string()
    };
    match query.filters.sort {
        Some(Sort::Newest) => found.sort_by_cached_key(|p| {
            (
                std::cmp::Reverse(day(p, true)),
                std::cmp::Reverse(p.downloads),
            )
        }),
        Some(Sort::Updated) => found.sort_by_cached_key(|p| {
            (
                std::cmp::Reverse(day(p, false)),
                std::cmp::Reverse(p.downloads),
            )
        }),
        _ => found.sort_by_key(|p| std::cmp::Reverse(p.downloads)),
    }
    // What can't be installed here is left out (the note above the list counts it), or with
    // `everything` goes after what can, each in the order asked for.
    let mut found: Vec<Project> = found
        .into_iter()
        .filter_map(|p| project_of(index, p, &query.game))
        .filter(|p| query.everything || p.blocked.is_none())
        .collect();
    found.sort_by_key(|p| p.blocked.is_some());
    found
        .into_iter()
        .skip(query.offset)
        .take(query.limit)
        .collect()
}

/// A project's screenshots as its page's gallery, in the index's order: those the registry
/// mirrored (`screenshots/<id>/`, served beside its icons), never another host's.
fn gallery_of(p: &index::Project) -> Vec<super::details::Image> {
    p.screenshots
        .iter()
        .filter(|url| super::images::refuse(url).is_none())
        .map(|url| super::details::Image {
            url: url.clone(),
            title: String::new(),
            description: String::new(),
            featured: false,
        })
        .collect()
}

/// The registry as a browser source.
pub struct Registry;

impl Source for Registry {
    fn id(&self) -> SourceId {
        SourceId::Sunrise
    }

    fn search(&self, query: &Query) -> Result<Vec<Project>, String> {
        let found = search(&*index()?, query);
        refresh_ratings();
        Ok(found)
    }

    fn project(&self, id: &str) -> Result<Project, String> {
        let index = index()?;
        let p = index
            .project(id)
            .ok_or_else(|| format!("{id} isn't in Sunrise's registry."))?;
        project_of(&index, p, &super::game()).ok_or_else(|| {
            format!(
                "{} is a {} in Sunrise's registry: this launcher can't install those yet.",
                p.name,
                p.kind.name()
            )
        })
    }

    fn versions(&self, project: &Project) -> Result<Vec<Version>, String> {
        let index = index()?;
        let p = index
            .project(&project.id)
            .ok_or_else(|| format!("{} isn't in Sunrise's registry.", project.title))?;
        Ok(p.versions
            .iter()
            .map(|v| version_of(&index, p, v))
            .collect())
    }

    /// Only categories and tags: every registry file is for Sunrise on the version it plays. The
    /// tags come from the index the search already read (never the network).
    fn tags(&self, kind: Kind) -> Result<super::details::Tags, String> {
        Ok(super::details::Tags {
            tags: known_index().map(|i| tags_of(&i, kind)).unwrap_or_default(),
            categories: categories_of(kind)
                .iter()
                .map(|name| super::details::Tag {
                    name: (*name).to_owned(),
                    header: "categories".into(),
                })
                .collect(),
        })
    }

    fn details(&self, project: &Project) -> Result<Details, String> {
        refresh_ratings();
        let index = index()?;
        let p = index
            .project(&project.id)
            .ok_or_else(|| format!("{} isn't in Sunrise's registry.", project.title))?;
        let newest = p.versions.first();
        let mut body = p.description.clone();
        if let Some(v) = newest.filter(|v| !v.capabilities.is_empty()) {
            body.push_str("\n\n## What it may do\n\n");
            for c in v.capabilities.known() {
                body.push_str(&format!("- May {}\n", c.describe()));
            }
        }
        if let Some(v) = newest.filter(|v| !v.requires.is_empty()) {
            body.push_str(
                "\n\n## Its mods\n\nInstalled with it into its own profile, each checked:\n\n",
            );
            for r in &v.requires {
                let found = index
                    .project(&r.id)
                    .and_then(|m| Some((m, m.versions.iter().find(|w| w.version == r.version)?)));
                let Some((m, w)) = found else { continue };
                let may: Vec<&str> = w
                    .capabilities
                    .known()
                    .iter()
                    .map(|c| c.describe())
                    .collect();
                let may = match may.is_empty() {
                    true => String::new(),
                    false => format!(": may {}", may.join(", ")),
                };
                body.push_str(&format!("- {} {}{may}\n", m.name, w.version));
            }
        }
        // Who publishes it: the GitHub accounts the registry checked at submission.
        let owners = p.owners.iter().filter(|o| github_login(o)).map(|o| Link {
            label: format!("@{o}"),
            url: format!("https://github.com/{o}"),
        });
        Ok(Details {
            body,
            links: [
                Link {
                    label: "Source".into(),
                    url: p.source.clone(),
                },
                Link {
                    label: "Page".into(),
                    url: project.page.clone(),
                },
                Link {
                    label: "Report".into(),
                    url: format!("{REPORT}&title={}", p.id),
                },
            ]
            .into_iter()
            .chain(owners)
            .collect(),
            authors: p.authors.clone(),
            published: p
                .versions
                .last()
                .map(|v| v.published.clone())
                .unwrap_or_default(),
            updated: newest.map(|v| v.published.clone()).unwrap_or_default(),
            categories: p.categories.clone(),
            gallery: gallery_of(p),
            github: github_repo(&p.source).map(|url| GitHub {
                url,
                stars: p.stars,
            }),
            ..Details::default()
        })
    }

    fn fetch(
        &self,
        file: &FileRef,
        folder: &Path,
        progress: &dyn Fn(f32),
    ) -> Result<PathBuf, String> {
        fetch(file, folder, local(), progress)
    }
}

/// The versions the player picked in game folder `home`, by mod id.
pub fn chosen(home: &Path) -> std::collections::BTreeMap<String, String> {
    fs::read(home.join("mod-browser/chosen-versions.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Downloads a registry file, checked; `file://` files only from a local index.
fn fetch(
    file: &FileRef,
    folder: &Path,
    local: bool,
    progress: &dyn Fn(f32),
) -> Result<PathBuf, String> {
    match file.url.strip_prefix("file://").filter(|_| local) {
        Some(path) => {
            let path = local_path(path);
            let reader = fs::File::open(path).map_err(|e| format!("{path}: {e}"))?;
            download::save_from_anywhere(file, reader, folder, progress)
        }
        None => download::fetch(file, folder, progress),
    }
}

/// The path of a `file:///` URL: on Windows `/C:/x` is `C:/x`.
fn local_path(path: &str) -> &str {
    match path.strip_prefix('/') {
        Some(rest) if cfg!(windows) && rest.get(1..2) == Some(":") => rest,
        _ => path,
    }
}

/// What the registry adds to an installed entry.
fn trust_label(trust: Trust) -> &'static str {
    match trust {
        Trust::Verified => "Verified",
        Trust::Unsigned => "Unsigned",
    }
}

/// The game folder path a registry project's file goes to.
fn relative(kind: Kind, id: &str, file_name: &str) -> Result<String, String> {
    match kind {
        Kind::Mod => Ok(format!("{}/{id}.smod", mod_list::MODS_DIR)),
        Kind::ResourcePack => Ok(format!("resourcepacks/{file_name}")),
        Kind::Shader => Ok(format!("shaderpacks/{file_name}")),
        Kind::Modpack => Err("A modpack installs as its own profile.".into()),
        Kind::DataPack => Err("A data pack goes into a world (fetch_datapack).".into()),
    }
}

/// Removes registry mods in one transaction (`txn`), each only if it is still the file shown
/// (id and checksum). Returns the ids that went.
pub fn remove_checked(home: &Path, wanted: &[(String, String)]) -> Result<Vec<String>, String> {
    dependencies::remove_checked(home, wanted)
}

/// Installs a resource or shader pack into the game folder. Registry mods instead go
/// through the exact-profile candidate and cannot reach this direct writer.
pub(super) fn install(
    project: &Project,
    home: &Path,
    game: &str,
    progress: &dyn Fn(&str, f32),
) -> Result<Vec<installed::Entry>, installed::InstallError> {
    let index = index()?;
    let p = index
        .project(&project.id)
        .filter(|p| matches!(p.kind, index::Kind::ResourcePack | index::Kind::ShaderPack))
        .ok_or_else(txn::unsettled)?;
    let version = newest(&index, p, game)?;
    Ok(vec![install_version(
        &index,
        local(),
        p,
        version,
        home,
        progress,
    )?])
}

/// Exercise the dependency planner and legacy record directly in local fixtures.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn install_set_from(
    index: &Index,
    local: bool,
    projects: &[Project],
    home: &Path,
    game: &str,
    native_allowed: bool,
    turn_on: bool,
    approvals: &[installed::Approval],
    progress: &dyn Fn(&str, f32),
) -> Result<Vec<installed::Entry>, installed::InstallError> {
    let waiting = txn::waiting_packages(home)?;
    let asked = registry_candidates(
        index,
        projects.iter().map(|p| p.id.as_str()),
        game,
        &waiting,
    )?;
    dependencies::install(
        &Install {
            index,
            local,
            home,
            approvals,
            progress,
        },
        &asked,
        native_allowed,
        turn_on,
        &waiting,
    )
}

fn registry_candidates<'a, 'b>(
    index: &'a Index,
    ids: impl IntoIterator<Item = &'b str>,
    game: &str,
    waiting: &std::collections::BTreeMap<String, txn::WaitingMod>,
) -> Result<Vec<(&'a index::Project, &'a index::Version)>, String> {
    ids.into_iter()
        .map(|id| {
            let p = index
                .project(id)
                .filter(|p| matches!(p.kind, index::Kind::Mod | index::Kind::NativeMod))
                .ok_or_else(|| format!("{id} isn't a mod in Sunrise's registry."))?;
            let version = match waiting.get(&p.id) {
                Some(waiting) => waiting.replacement(index, p, game).ok_or_else(|| {
                    trf(
                        "{mod} needs an update from its author before it can be turned on.",
                        &[("mod", &p.name)],
                    )
                })?,
                None => newest(index, p, game)?,
            };
            Ok((p, version))
        })
        .collect()
}

mod registry_install;
pub use registry_install::{prepare_registry_install, RegistryInstall};

/// [`install`] from a trusted `index`.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn install_from(
    index: &Index,
    local: bool,
    project: &Project,
    home: &Path,
    game: &str,
    native_allowed: bool,
    turn_on: bool,
    approvals: &[installed::Approval],
    progress: &dyn Fn(&str, f32),
) -> Result<Vec<installed::Entry>, installed::InstallError> {
    let p = index
        .project(&project.id)
        .ok_or_else(|| format!("{} isn't in Sunrise's registry.", project.title))?;
    if matches!(p.kind, index::Kind::Mod | index::Kind::NativeMod) {
        return install_set_from(
            index,
            local,
            std::slice::from_ref(project),
            home,
            game,
            native_allowed,
            turn_on,
            approvals,
            progress,
        );
    }
    let v = newest(index, p, game)?;
    Ok(vec![install_version(index, local, p, v, home, progress)?])
}

/// Downloads the newest version of the registry's data pack `project` into `folder`, checked
/// against the index as every registry file is (size, SHA-256, signature, `pack.mcmeta` and
/// `data/`): the file and its version. [`installed::install_datapack`] puts it in a world.
pub fn fetch_datapack(
    project: &Project,
    folder: &Path,
    game: &str,
    progress: &dyn Fn(f32),
) -> Result<(PathBuf, String), String> {
    fetch_datapack_from(&*index()?, local(), project, folder, game, progress)
}

fn fetch_datapack_from(
    index: &Index,
    local: bool,
    project: &Project,
    folder: &Path,
    game: &str,
    progress: &dyn Fn(f32),
) -> Result<(PathBuf, String), String> {
    let p = index
        .project(&project.id)
        .filter(|p| p.kind == index::Kind::DataPack)
        .ok_or_else(|| format!("{} isn't a data pack in Sunrise's registry.", project.title))?;
    let v = newest(index, p, game)?;
    let file = &version_of(index, p, v).files[0];
    let path = fetch(file, folder, local, progress)?;
    index::check_file(index, p, v, &path).inspect_err(|_| {
        let _ = fs::remove_file(&path);
    })?;
    Ok((path, v.version.clone()))
}

/// A project's newest version for the Minecraft version `game` that this build can install
/// ([`Index::installable`]: not taken down, no capability it doesn't know).
fn newest<'a>(
    index: &Index,
    p: &'a index::Project,
    game: &str,
) -> Result<&'a index::Version, String> {
    if let Some(v) = index.installable(p, game) {
        return Ok(v);
    }
    // Any version for `game` taken down says why, even one this build couldn't install anyway.
    let taken_down = p
        .versions
        .iter()
        .filter(|v| v.game == game)
        .find_map(|v| index.revoked(&p.id, &v.file.sha256));
    Err(match taken_down {
        Some(r) => format!("{} was taken down: {}", p.name, r.reason),
        None => format!("{} has no version for {game} to install.", p.name),
    })
}

/// Installs version `v` of the registry's mod, resource pack or shader pack `p` into the game
/// folder `home`.
fn install_version(
    index: &Index,
    local: bool,
    p: &index::Project,
    v: &index::Version,
    home: &Path,
    progress: &dyn Fn(&str, f32),
) -> Result<installed::Entry, String> {
    let entry = install_version_guarded(index, local, p, v, home, &|_| Ok(()), progress)?;
    // The player asked for this install: the mod starts on. An automatic update installs
    // through the guarded function alone, so it leaves the player's on/off choice as it was
    // (#10071).
    if entry.kind == Kind::Mod {
        mod_list::set_enabled(home, &p.id, true)?;
    }
    Ok(entry)
}

/// [`install_version`], with `guard` asked about the downloaded file once it passed the index's
/// checks and before it replaces anything: its error leaves everything as it was.
fn install_version_guarded(
    index: &Index,
    local: bool,
    p: &index::Project,
    v: &index::Version,
    home: &Path,
    guard: &dyn Fn(&Path) -> Result<(), String>,
    progress: &dyn Fn(&str, f32),
) -> Result<installed::Entry, String> {
    let _lock = super::lock_home(home).map_err(|e| e.to_string())?;
    let mut list = installed::Installed::load(home);
    let install = Install {
        index,
        local,
        home,
        approvals: &[],
        progress,
    };
    let (entry, file, unreadable) =
        install.prepare(p, v, &list, &home.join("mod-browser/downloads"), None)?;
    let Some((path, _)) = file else {
        return Ok(entry);
    };
    guard(&path).inspect_err(|_| {
        let _ = fs::remove_file(&path);
    })?;
    let target = home.join(&entry.file);
    if let Some(dir) = target.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let aside = unreadable
        .map(|broken| {
            let to = unreadable_slot(home, &broken)?;
            if let Some(dir) = to.parent() {
                fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
            fs::rename(&broken, &to).map_err(|e| format!("{}: {e}", broken.display()))?;
            Ok::<_, String>((broken, to))
        })
        .transpose()
        .inspect_err(|_| {
            let _ = fs::remove_file(&path);
        })?;
    if let Err(e) = fs::rename(&path, &target) {
        if let Some((broken, to)) = aside {
            let _ = fs::rename(to, broken);
        }
        return Err(format!("{}: {e}", target.display()));
    }
    if let Some(old) = list
        .find(SourceId::Sunrise, &p.id)
        .filter(|old| old.file != entry.file)
        .and_then(|old| old.path(home))
    {
        let _ = fs::remove_file(old);
    }
    list.entries
        .retain(|e| !(e.source == SourceId::Sunrise && e.project == p.id));
    list.entries.push(entry.clone());
    list.save(home)?;
    Ok(entry)
}

/// Where an installed file that no longer parses is kept when its update replaces it.
fn unreadable_slot(home: &Path, broken: &Path) -> Result<PathBuf, String> {
    let dir = home.join("mod-browser/unreadable");
    let name = broken.file_name().unwrap_or_default().to_string_lossy();
    Ok((0..)
        .map(|n| dir.join(format!("{name}.{n}")))
        .find(|to| !to.exists())
        .expect("an unused name"))
}

struct Install<'a> {
    index: &'a Index,
    local: bool,
    home: &'a Path,
    /// Installed registry mods removed by this install, in the same transaction.
    approvals: &'a [installed::Approval],
    progress: &'a dyn Fn(&str, f32),
}

/// A checked download and its manifest, or the already-installed entry with no download; and
/// the installed file that no longer parses, which the install moves aside.
type Prepared = (
    installed::Entry,
    Option<(PathBuf, index::Facts)>,
    Option<PathBuf>,
);

impl Install<'_> {
    fn prepare(
        &self,
        p: &index::Project,
        v: &index::Version,
        list: &installed::Installed,
        staging: &Path,
        waiting: Option<&txn::WaitingMod>,
    ) -> Result<Prepared, String> {
        let index = self.index;
        let project = project_of(index, p, &v.game)
            .ok_or_else(|| format!("{} can't be installed here.", p.name))?;
        if let Some(r) = index.revoked(&p.id, &v.file.sha256) {
            return Err(format!("{} was taken down: {}", p.name, r.reason));
        }
        let native = p.kind == index::Kind::NativeMod;
        let relative = match native {
            true => format!("{}/{}.smod", crate::native_mods::INSTALLED, p.id),
            false => relative(project.kind, &p.id, &v.file.name)?,
        };
        let ours = |e: &installed::Entry| e.source == SourceId::Sunrise && e.project == p.id;
        let old = list.entries.iter().find(|e| ours(e)).cloned();
        let mut unreadable = None;
        if let Some(old) = &old {
            let mut different = old.kind != project.kind;
            // A file that is gone has no kind to compare, and one that no longer parses is
            // never read as content: the update replaces both, the second after the install
            // moves it aside. A file that exists but can't be opened (a transient IO or
            // permission error) is not a broken one: the update refuses and leaves it.
            if !different && project.kind == Kind::Mod {
                let path = old.path(self.home).ok_or_else(|| {
                    trf(
                        "{name} is recorded outside the mod folders: remove it before installing this version.",
                        &[("name", &p.name)],
                    )
                })?;
                match fs::File::open(&path).and_then(|f| f.metadata()) {
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(format!("{}: {e}", path.display())),
                    Ok(m) if !m.is_file() => {
                        return Err(format!("{}: not a regular file", path.display()));
                    }
                    Ok(_) => {
                        let installed_native = index::native::holds(&path);
                        let kind = if installed_native {
                            index::Kind::NativeMod
                        } else {
                            index::Kind::Mod
                        };
                        match index::inspect(&kind, &path) {
                            Ok(_) => different = installed_native != native,
                            Err(_) => unreadable = Some(path),
                        }
                    }
                }
            }
            if different {
                return Err(format!(
                    "{} has a different installed kind: remove it before installing this version.",
                    p.name
                ));
            }
        }
        if old.as_ref().is_some_and(|o| o.version == v.version)
            && waiting.is_none_or(|item| item.same_package(v))
        {
            return Ok((old.expect("checked"), None, None));
        }
        let target = self.home.join(&relative);
        let taken_by = list.entries.iter().find(|e| !ours(e) && e.file == relative);
        let folder_mod = self.home.join(mod_list::MODS_DIR).join(&p.id);
        if let Some(other) = taken_by {
            return Err(format!(
                "{relative} belongs to {}: remove it first.",
                other.title
            ));
        }
        let clash = target.exists() && old.as_ref().is_none_or(|o| o.file != relative);
        if project.kind == Kind::Mod && (clash || (!native && folder_mod.exists())) {
            return Err(format!(
                "A mod with the id {} is already in {}: move it away first.",
                p.id,
                mod_list::MODS_DIR
            ));
        }
        if clash {
            return Err(format!("{relative} is already there: move it away first."));
        }
        let line = format!("{} {}", p.name, v.version);
        let file = &version_of(index, p, v).files[0];
        let path = fetch(file, staging, self.local, &|f| (self.progress)(&line, f))?;
        let (trust, facts) =
            index::check_file_with_facts(index, p, v, &path).inspect_err(|_| {
                let _ = fs::remove_file(&path);
            })?;
        let entry = installed::Entry {
            source: SourceId::Sunrise,
            project: p.id.clone(),
            slug: p.id.clone(),
            title: p.name.clone(),
            kind: project.kind,
            version: v.version.clone(),
            number: v.version.clone(),
            file: relative,
            checksum: format!("SHA-256:{}", v.file.sha256),
            page: project.page.clone(),
            dependency: false,
            verdict: Some(trust_label(trust).into()),
        };
        Ok((entry, Some((path, facts)), unreadable))
    }
}

/// The mod id of an installed mod (`sunrise-mods/<id>`, `sunrise-mods/<id>.smod` or
/// `sunrise-native-mods/<id>.smod`).
pub fn mod_id(entry: &installed::Entry) -> Option<&str> {
    if let Some(rest) = entry.file.strip_prefix(crate::native_mods::INSTALLED) {
        return rest.strip_prefix('/')?.strip_suffix(".smod");
    }
    let rest = entry
        .file
        .strip_prefix(mod_list::MODS_DIR)?
        .strip_prefix('/')?;
    Some(rest.strip_suffix(".smod").unwrap_or(rest))
}

/// Turns the registry's files that were taken down off in the game folder `home`, and says
/// which. Their files stay, so the player sees them and can remove them.
pub fn turn_off_revoked(index: &Index, home: &Path) -> Result<Vec<String>, String> {
    let _lock = crate::browser::lock_home(home).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for e in installed::Installed::load(home).entries {
        let hash = e.checksum.strip_prefix("SHA-256:").unwrap_or_default();
        let Some(r) = (e.source == SourceId::Sunrise)
            .then(|| index.revoked(&e.project, hash))
            .flatten()
        else {
            continue;
        };
        if let (Kind::Mod, Some(id)) = (e.kind, mod_id(&e)) {
            if mod_list::is_enabled(home, id) {
                mod_list::set_enabled(home, id, false)?;
                out.push(format!(
                    "{} was taken down and is off: {}",
                    e.title, r.reason
                ));
            }
        } else if matches!(e.kind, Kind::ResourcePack | Kind::Shader) {
            out.push(format!(
                "{} was taken down: {} Remove it.",
                e.title, r.reason
            ));
        }
    }
    Ok(out)
}

/// Check the actual launch profile, after its folder has been applied and before starting it.
/// May contact the registry. A failed fetch permits play with a notice, not a clean verdict.
pub fn before_play(
    cmd: &mut std::process::Command,
    tag: &str,
    launch: &crate::launch::Launch,
    owner: &txn::Lease,
) -> Result<Vec<String>, String> {
    crate::launch::apply_environment(cmd, launch);
    let Some(active) = owner.active_build()? else {
        return Err(tr("This profile has no committed build to start.").into());
    };
    if active.executable() != tag {
        return Err(tr(
            "The launch command does not match this profile's saved build and folders.",
        )
        .into());
    }
    verify_launch_arguments(cmd, owner.home(), &active.resources())?;
    let source = cmd
        .get_envs()
        .find(|(key, _)| *key == crate::run_leases::SOURCE)
        .and_then(|(_, value)| value);
    if source.map(Path::new) != Some(crate::backend::version_dir(tag).as_path())
        || cmd.get_envs().any(|(key, value)| {
            key.eq_ignore_ascii_case("SUNRISE_WINDOWS_RUNTIME_SLOT") && value.is_some()
        })
    {
        return Err(tr(
            "The launch command does not match this profile's saved build and folders.",
        )
        .into());
    }
    active.verify_program(Path::new(cmd.get_program()))?;
    let included = crate::native_status::included(tag)?;
    check_before_play(owner.home(), &included, || {
        index().ok()?;
        fetched_index()
    })
}

fn verify_launch_arguments(
    cmd: &std::process::Command,
    home: &Path,
    resources: &Path,
) -> Result<(), String> {
    use sunrise_launcher_smod::{
        manifest::Side,
        selection::{Argument, LaunchArguments},
    };
    let args = LaunchArguments::for_command(cmd).map_err(|e| e.to_string())?;
    let cwd = cmd.get_current_dir().unwrap_or(Path::new("."));
    let mut resource_args = args.rest.iter().filter_map(|arg| match arg {
        Argument::Value(name, value) if name == "--resources" => Some(cwd.join(value)),
        _ => None,
    });
    let actual = resource_args
        .next()
        .and_then(|path| path.canonicalize().ok());
    let expected = resources.canonicalize().ok();
    let home_matches = home.canonicalize().is_ok_and(|expected| {
        args.context
            .home
            .canonicalize()
            .is_ok_and(|path| path == expected)
            && args
                .context
                .game_dir
                .canonicalize()
                .is_ok_and(|path| path == expected)
    });
    if args.installation_side() != Side::Client
        || !args.context.dev_mods.is_empty()
        || !home_matches
        || actual.is_none()
        || actual != expected
        || resource_args.next().is_some()
    {
        return Err(tr(
            "The launch command does not match this profile's saved build and folders.",
        )
        .into());
    }
    Ok(())
}

fn check_before_play(
    home: &Path,
    included: &[crate::native_status::Included],
    load: impl FnOnce() -> Option<Arc<Index>>,
) -> Result<Vec<String>, String> {
    let entries = installed::Installed::load(home).entries;
    let packages = txn::scan_packages(home)?;
    if included.is_empty()
        && packages.is_empty()
        && !entries.iter().any(|e| e.source == SourceId::Sunrise)
    {
        return Ok(Vec::new());
    }
    let Some(index) = load() else {
        return Ok(vec![
            tr("Mods were not checked for registry takedowns.").into()
        ]);
    };
    // Reading revocations must never turn off a package in the committed profile.
    // Identifying a file by its actual bytes also covers a game-owned update of that file.
    for (manifest, path) in packages {
        if !mod_list::is_enabled(home, &manifest.id) {
            continue;
        }
        let hash = index::sha256_file(&path)?;
        let project = entries
            .iter()
            .find(|entry| {
                entry.source == SourceId::Sunrise && entry.path(home).as_ref() == Some(&path)
            })
            .map_or(manifest.id.as_str(), |entry| entry.project.as_str());
        if let Some(revoked) = index.revoked(project, &hash) {
            return Err(trf(
                "{mod} was taken down: {reason} Turn it off in Mods before playing.",
                &[("mod", &manifest.name), ("reason", &revoked.reason)],
            ));
        }
    }
    // A revocation can reach code embedded in the selected binary as well as package files.
    for crate::native_status::Included {
        id,
        version,
        sha256,
    } in included
    {
        let published = index
            .project(id)
            .and_then(|p| p.versions.iter().find(|v| &v.version == version));
        let installed_hash = entries.iter().find_map(|e| {
            (e.source == SourceId::Sunrise && &e.project == id && &e.version == version)
                .then(|| e.checksum.strip_prefix("SHA-256:"))
                .flatten()
        });
        let revoked = index
            .revoked(id, sha256)
            .or_else(|| published.and_then(|v| index.revoked(id, &v.file.sha256)))
            .or_else(|| installed_hash.and_then(|hash| index.revoked(id, hash)));
        if let Some(revoked) = revoked {
            return Err(trf(
                "This build includes a mod the registry took down. Choose another game build.\n{mod}: {reason}",
                &[("mod", id), ("reason", &revoked.reason)],
            ));
        }
        if published.is_none() && index.revoked.iter().any(|r| &r.id == id) {
            // Older builds don't record the registry file's hash. Once a version disappears
            // from the index, an installed file with the same id can't prove what was baked in.
            return Err(trf(
                "This build cannot rule out a mod the registry took down. Choose another game build.\nMod: {mod}",
                &[("mod", id)],
            ));
        }
    }
    Ok(entries
        .iter()
        .filter_map(|entry| {
            if entry.source != SourceId::Sunrise
                || !matches!(entry.kind, Kind::ResourcePack | Kind::Shader)
            {
                return None;
            }
            let hash = entry.checksum.strip_prefix("SHA-256:")?;
            let revoked = index.revoked(&entry.project, hash)?;
            Some(format!(
                "{} was taken down: {} Remove it.",
                entry.title, revoked.reason
            ))
        })
        .collect())
}

mod content_loss;
mod dependencies;
pub mod pack;
#[cfg(test)]
mod tests;
pub mod txn;
