//! The native build's log and how its failures read: copied from the game's `sunrise-native-build` (tests/test_launcher_copies.py).
use crate::i18n::{tr, trf};
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::{path::PathBuf, sync::Arc};

/// The native build's folder, under the launcher's build folder.
pub(crate) fn root() -> PathBuf {
    crate::backend::build_dir().join("native")
}

/// The build's log, kept until the next build.
pub fn log_path() -> PathBuf {
    root().join("build.log")
}

/// Snapshot the exact attempt while its lease excludes another writer. Keep the final
/// 256 KiB if a very large build log would otherwise travel through the UI.
pub(crate) fn snapshot() -> Option<Arc<str>> {
    const LIMIT: u64 = 256 << 10;
    let mut file = fs::File::open(log_path()).ok()?;
    let skip = file.metadata().ok()?.len().saturating_sub(LIMIT);
    file.seek(SeekFrom::Start(skip)).ok()?;
    let mut bytes = Vec::new();
    file.take(LIMIT).read_to_end(&mut bytes).ok()?;
    if bytes.is_empty() {
        return None;
    }
    let text = String::from_utf8_lossy(&bytes);
    Some(if skip == 0 {
        Arc::from(text.as_ref())
    } else {
        Arc::from(format!(
            "{}\n\n{text}",
            tr("Earlier build output was omitted.")
        ))
    })
}

/// Makes the build log fresh for a new attempt; the error says why it couldn't, for the Build log page.
pub fn clear_log() -> Result<(), String> {
    fs::create_dir_all(root()).map_err(|e| e.to_string())?;
    fs::write(log_path(), "").map_err(|e| format!("{}: {e}", log_path().display()))
}

/// Appends `message` (with a newline) to the build log, so a failure before any command is recorded.
pub fn append_log(message: &str) -> Result<(), String> {
    fs::create_dir_all(root()).map_err(|e| format!("{}: {e}", root().display()))?;
    let mut log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path())
        .map_err(|e| format!("{}: {e}", log_path().display()))?;
    writeln!(log, "{message}").map_err(|e| e.to_string())
}

/// Appends `text` to the build log, the way a compiler warning reads.
pub fn log_warning(text: &str) {
    if let Ok(mut log) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path())
    {
        let _ = writeln!(log, "{}", trf("warning: {text}", &[("text", text)]));
    }
}

/// What a cancelled build fails with: the card shows no error for it.
pub const CANCELLED: &str = "Build cancelled";

/// A failed step's error: the `errors` lines not already among the last ones (`tail`), then those.
pub fn failure(what: &str, status: &str, errors: &[String], tail: &[String]) -> String {
    let end = diagnostic(&tail.join("\n")).unwrap_or_else(|| tail.join("\n"));
    let first: Vec<&str> = errors
        .iter()
        .filter(|e| !tail.contains(e))
        .map(String::as_str)
        .collect();
    let (shown, first) = if first.is_empty() {
        (tr("The end of its output"), String::new())
    } else {
        (
            tr("Its first errors and the end of its output"),
            format!("{}\n…\n", first.join("\n")),
        )
    };
    let whole = trf(
        "{shown}, the whole of it in {log}",
        &[("shown", shown), ("log", &log_path().display().to_string())],
    );
    format!("{} ({status}). {whole}:\n{first}{end}", failed(what))
}

/// How [`failure`] starts for `what`, which is how a failure is traced to a mod.
pub fn failed(what: &str) -> String {
    trf("{what} failed", &[("what", what)])
}

/// Extracts the mod id from a mixin manifest or mod file path (e.g. `.../mixins.incoming/<mod_id>/...`).
fn mod_id_from_file(file: &str) -> Option<&str> {
    for prefix in ["mixins.incoming", "mods.incoming"] {
        if let Some(pos) = file.find(prefix) {
            let rest = &file[pos + prefix.len()..];
            let sub = rest.trim_start_matches(['/', '\\']);
            if let Some(end) = sub.find(['/', '\\']) {
                let id = &sub[..end];
                if !id.is_empty() {
                    return Some(id);
                }
            }
        }
    }
    None
}

/// A horizon-mixins diagnostic (`{"code", "message", "file"}`) in one line, led by the mod it names.
fn diagnostic(text: &str) -> Option<String> {
    let start = text.find('{')?;
    let value: serde_json::Value = serde_json::from_str(&text[start..]).ok()?;
    let mod_id = value["mod_id"]
        .as_str()
        .or_else(|| value["file"].as_str().and_then(mod_id_from_file));
    let culprit = match (mod_id, value["injection_id"].as_str()) {
        (Some(m), Some(i)) => format!("{m} ({i}): "),
        (Some(m), None) => format!("{m}: "),
        _ => String::new(),
    };
    let mut line = format!(
        "{culprit}{}: {}",
        value["code"].as_str()?,
        value["message"].as_str()?
    );
    if let Some(file) = value["file"].as_str() {
        line += &format!(" ({file})");
    }
    Some(line)
}

/// What the build log and a failure call compiling mod `id`.
pub fn compiling(id: &str) -> String {
    trf("Compiling {mod}", &[("mod", id)])
}

/// Why a build can't use the prebuilt game.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// A mod declares Cargo features, or asks the mod API for features of its own.
    Features { mod_id: String },
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Self::Features { mod_id } => trf(
                "E_NATIVE_FEATURES: {mod} requests Cargo features; use the pinned SDK feature set",
                &[("mod", mod_id)],
            ),
        };
        f.write_str(&text)
    }
}

impl Refusal {
    /// What the player reads about it; the code and its remedy for the mod's author stay in the build log.
    pub fn plain(&self) -> String {
        let why = match self {
            Self::Features { mod_id } => trf(
                "{mod} can't be built here: it asks for build options Sunrise's game doesn't offer.",
                &[("mod", mod_id)],
            ),
        };
        format!(
            "{why} {}",
            tr(
                "The problem is in the mod, not in your game. Switch it off to play, or send the log to its author."
            )
        )
    }
}
