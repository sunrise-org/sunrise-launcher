//! The list of servers the launcher knows (`servers.json`), and their folders.

use super::*;

/// One server the launcher knows: its name and its folder, on this computer or on `host`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    pub dir: PathBuf,
    /// The other computer it runs on, reached over SSH (`user@host`, or a `Host` from `~/.ssh/config`);
    /// `dir` is then the folder there ([`remote`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
}

impl Entry {
    /// `sunrise-server --dir <dir> <args>`, here or over SSH on the entry's computer.
    pub fn command(&self, args: &[&str]) -> Result<Command, String> {
        match &self.host {
            Some(host) => remote::command(host, &self.dir.to_string_lossy(), args),
            None => command(&self.dir, args),
        }
    }
}

/// The game mode a server's `server.toml` sets (`[world] game_mode`), read from the file
/// without asking `sunrise-server`: the Servers page's list says it for each stopped server.
pub fn game_mode(dir: &Path) -> Option<String> {
    let text = fs::read_to_string(dir.join("server.toml")).ok()?;
    let config: toml::Value = toml::from_str(&text).ok()?;
    config
        .get("world")?
        .get("game_mode")?
        .as_str()
        .map(str::to_string)
}

fn list_file() -> PathBuf {
    backend::data_dir().join("servers.json")
}

/// The servers, in the order they were added; folders on this computer that are gone are left out.
pub fn list() -> Vec<Entry> {
    let entries: Vec<Entry> = fs::read_to_string(list_file())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    entries
        .into_iter()
        .filter(|e| e.host.is_some() || e.dir.is_dir())
        .collect()
}

fn save_list(entries: &[Entry]) -> Result<(), String> {
    let path = list_file();
    fs::create_dir_all(backend::data_dir()).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(entries).map_err(|e| e.to_string())?;
    // A torn write would drop every server from the sidebar: write beside
    // the list, flush it, and rename it into place. Without the flush a
    // power cut can leave a file of the right length full of zeros.
    let partial = path.with_extension("json.tmp");
    let mut file = fs::File::create(&partial).map_err(|e| e.to_string())?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    fs::rename(&partial, &path).map_err(|e| e.to_string())
}

/// Adds a server folder to the list (once).
pub fn add(entry: Entry) -> Result<(), String> {
    let mut all = list();
    all.retain(|e| (&e.host, &e.dir) != (&entry.host, &entry.dir));
    all.push(entry);
    save_list(&all)
}

/// Gives the server in `dir` on this computer a new name in the list; its folder keeps its own.
pub fn rename(dir: &Path, name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(tr("A server needs a name.").into());
    }
    let mut all = list();
    let entry = all
        .iter_mut()
        .find(|e| e.host.is_none() && e.dir == dir)
        .ok_or_else(|| tr("This server is no longer in the launcher’s list."))?;
    entry.name = name.to_string();
    save_list(&all)
}

/// Takes a server on this computer off the list; its folder stays.
pub fn forget(dir: &Path) -> Result<(), String> {
    let mut all = list();
    all.retain(|e| e.host.is_some() || e.dir != dir);
    save_list(&all)
}

/// Takes a server on another computer off the list; nothing there changes.
pub fn forget_remote(host: &str, dir: &Path) -> Result<(), String> {
    let mut all = list();
    all.retain(|e| e.host.as_deref() != Some(host) || e.dir != dir);
    save_list(&all)
}

/// Removes a registered, stopped server, retaining its files in the system trash.
pub fn delete(dir: &Path) -> Result<(), String> {
    if !list().iter().any(|entry| entry.dir == dir) {
        return Err(tr("This server is no longer in the launcher’s list.").into());
    }
    if service_installed(dir) {
        return Err(tr("Turn off running in the background before deleting this server.").into());
    }
    trash_stopped_server(dir, crate::desktop_seams::to_trash)?;
    forget(dir).map_err(|e| {
        trf(
            "The server was moved to the trash, but its list could not be saved: {error}",
            &[("error", &e.to_string())],
        )
    })
}

pub(super) fn trash_stopped_server(
    dir: &Path,
    trash: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    if dir
        .symlink_metadata()
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
        || !dir.join("server.toml").is_file()
    {
        return Err(tr("This is not a server folder.").into());
    }
    let state = dir.join(".sunrise");
    fs::create_dir_all(&state).map_err(|e| e.to_string())?;
    // The supervisor holds this same lock throughout starting, running and stopping.
    // Hold it through the move so a stale UI/API status cannot delete a live server.
    let lock = fs::File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(state.join("lock"))
        .map_err(|e| e.to_string())?;
    lock.try_lock()
        .map_err(|_| tr("Stop the server before deleting it.").to_string())?;
    // Windows can't move a folder while a file in it is open, the lock included: let it go
    // first. Still safe: a supervisor starting in between opens the lock again, and then the
    // move fails instead of taking a live server.
    #[cfg(windows)]
    drop(lock);
    trash(dir).map_err(|e| {
        trf(
            "Could not move the server to the trash: {error}",
            &[("error", &e.to_string())],
        )
    })
}

/// A folder name for a server name: lower case letters, digits and dashes.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "server".into()
    } else {
        out
    }
}

/// A new folder for a server called `name`, under `servers/` in the data folder.
pub fn new_folder(name: &str) -> PathBuf {
    let root = backend::data_dir().join("servers");
    let base = slug(name);
    let mut dir = root.join(&base);
    let mut n = 2;
    while dir.exists() {
        dir = root.join(format!("{base}-{n}"));
        n += 1;
    }
    dir
}
