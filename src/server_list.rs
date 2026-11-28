//! The player's server list, `servers.dat` in an instance's game folder: the game's Multiplayer
//! screen reads and writes the same file. Its format is vanilla's (`ServerList.load`/`save`,
//! `ServerData.read`/`write`): uncompressed NBT, `servers` a list of compounds with `name`, `ip`,
//! an optional `icon` (a PNG in base64), `acceptTextures`, `acceptedCodeOfConduct` and `hidden`.
//! The launcher adds, edits, moves and removes entries: every other field of every entry is written back
//! as it was read.
use crate::i18n::{tr, trf};
use base64::alphabet::STANDARD as ALPHABET;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig};
use base64::engine::DecodePaddingMode;
use base64::Engine;
use std::fs;
use std::net::Ipv6Addr;
use std::path::Path;
use sunrise_launcher_nbt::{self as nbt, Compound, JavaString, Limits, NamedTag, Tag};

const FILE: &str = "servers.dat";
/// Base64 as Java's decoder reads it: the trailing `=` may be there or not.
const ICON_BASE64: GeneralPurpose = GeneralPurpose::new(
    &ALPHABET,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);
#[derive(Clone, Debug, PartialEq)]
pub struct Server {
    pub name: String,
    pub ip: String,
    /// The server's picture, as the game last saw it (PNG).
    pub icon: Option<Vec<u8>>,
}

/// The entries as they are in the file, the hidden ones (servers joined with Direct
/// Connection) too.
pub struct List {
    entries: Vec<Compound>,
}

fn key(k: &str) -> JavaString {
    JavaString::from(k)
}

fn text(entry: &Compound, k: &str) -> String {
    match entry.get(&key(k)) {
        Some(Tag::String(s)) => s.to_string_lossy(),
        _ => String::new(),
    }
}

fn hidden(entry: &Compound) -> bool {
    matches!(entry.get(&key("hidden")), Some(Tag::Byte(b)) if *b != 0)
}

fn entry(name: &str, ip: &str) -> Compound {
    [
        (key("name"), Tag::String(name.into())),
        (key("ip"), Tag::String(ip.into())),
        (key("hidden"), Tag::Byte(0)),
    ]
    .into_iter()
    .collect()
}

/// A name and address as the game's Add Server screen takes them: trimmed, the address a valid
/// one, and a name of "Minecraft Server" when none was given.
fn checked<'a>(name: &'a str, ip: &'a str) -> Result<(&'a str, &'a str), String> {
    let (name, ip) = (name.trim(), ip.trim());
    if ip.is_empty() {
        return Err(tr("A server needs an address.").into());
    }
    if !valid_address(ip) {
        return Err(tr("That is not a server address: use a name or IP address, then an optional port from 0 to 65535.").into());
    }
    let name = if name.is_empty() {
        "Minecraft Server"
    } else {
        name
    };
    Ok((name, ip))
}

fn missing(index: usize) -> String {
    trf(
        "There is no server {number}.",
        &[("number", &(index + 1).to_string())],
    )
}

impl List {
    /// The list in `home`; empty when there is none or it can't be read, as the game does.
    pub fn load(home: &Path) -> List {
        Self::read(home).unwrap_or(List {
            entries: Vec::new(),
        })
    }

    /// The list in `home`, empty when there is none; none when the file is there but can't be read.
    fn read(home: &Path) -> Option<List> {
        let bytes = match fs::read(home.join(FILE)) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(_) => return None,
        };
        let entries = match bytes.is_empty() {
            true => Vec::new(),
            false => match nbt::read_named(&bytes, Limits::default())
                .ok()?
                .0
                .tag
                .get("servers")
            {
                Some(Tag::List(list)) => list
                    .iter()
                    .filter_map(|t| match t {
                        Tag::Compound(c) => Some(c.clone()),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            },
        };
        Some(List { entries })
    }

    /// The servers the Multiplayer screen shows, in its order.
    pub fn servers(&self) -> Vec<Server> {
        self.entries
            .iter()
            .filter(|e| !hidden(e))
            .map(|e| Server {
                name: text(e, "name"),
                ip: text(e, "ip"),
                icon: Some(text(e, "icon"))
                    .filter(|s| !s.is_empty())
                    .and_then(|s| ICON_BASE64.decode(s).ok()),
            })
            .collect()
    }

    /// Adds a server at the end of the shown ones, as the game's Add Server does.
    pub fn add(&mut self, name: &str, ip: &str) -> Result<(), String> {
        let (name, ip) = checked(name, ip)?;
        let at = self
            .entries
            .iter()
            .rposition(|e| !hidden(e))
            .map_or(0, |i| i + 1);
        self.entries.insert(at, entry(name, ip));
        Ok(())
    }

    /// Where the `index`th shown server sits among all the entries.
    fn slot(&self, index: usize) -> Option<usize> {
        (0..self.entries.len())
            .filter(|&i| !hidden(&self.entries[i]))
            .nth(index)
    }

    /// Gives the `index`th shown server a new name and address, as the game's Edit Server does:
    /// every other field stays, except the picture of a server that moved to another address.
    pub fn edit(&mut self, index: usize, name: &str, ip: &str) -> Result<(), String> {
        let (name, ip) = checked(name, ip)?;
        let at = self.slot(index).ok_or_else(|| missing(index))?;
        let entry = &mut self.entries[at];
        if text(entry, "ip") != ip {
            entry.remove(&key("icon"));
        }
        entry.insert(key("name"), Tag::String(name.into()));
        entry.insert(key("ip"), Tag::String(ip.into()));
        Ok(())
    }

    /// Swaps the `a`th and the `b`th shown servers: how one moves up or down the list.
    pub fn swap(&mut self, a: usize, b: usize) -> Result<(), String> {
        let first = self.slot(a).ok_or_else(|| missing(a))?;
        let second = self.slot(b).ok_or_else(|| missing(b))?;
        self.entries.swap(first, second);
        Ok(())
    }

    /// Removes the `index`th shown server.
    pub fn remove(&mut self, index: usize) -> Result<Server, String> {
        let server = self.servers().into_iter().nth(index).ok_or_else(|| {
            trf(
                "There is no server {number}.",
                &[("number", &(index + 1).to_string())],
            )
        })?;
        let at = (0..self.entries.len())
            .filter(|&i| !hidden(&self.entries[i]))
            .nth(index)
            .unwrap();
        self.entries.remove(at);
        Ok(server)
    }

    /// Writes the list as the game does: a new file first, the old one kept as `servers.dat_old`.
    pub fn save(&self, home: &Path) -> Result<(), String> {
        // The game writes the shown servers first, then the hidden ones.
        let (shown, hidden_entries): (Vec<&Compound>, Vec<&Compound>) =
            self.entries.iter().partition(|e| !hidden(e));
        let root = NamedTag {
            name: "".into(),
            tag: Tag::compound([(
                "servers",
                Tag::List(
                    shown
                        .into_iter()
                        .chain(hidden_entries)
                        .cloned()
                        .map(Tag::Compound)
                        .collect(),
                ),
            )]),
        };
        let bytes = nbt::write_named(&root, Limits::default()).map_err(|e| e.to_string())?;
        let (new, current) = (home.join("servers.dat_new"), home.join(FILE));
        let written = fs::create_dir_all(home)
            .and_then(|()| fs::write(&new, bytes))
            .and_then(|()| match current.exists() {
                true => fs::copy(&current, home.join("servers.dat_old")).map(|_| ()),
                false => Ok(()),
            })
            .and_then(|()| fs::rename(&new, &current));
        written.map_err(|e| {
            trf(
                "Could not save the server list: {error}",
                &[("error", &e.to_string())],
            )
        })
    }
}

/// Whether `address` is a host with an optional `:port`, as the game's Add Server screen takes it:
/// a name or IPv4 address, or an IPv6 address (in `[...]` when a port follows), and a port of
/// digits from 0 to 65535 (none is fine).
fn valid_address(address: &str) -> bool {
    let (host, port, bracketed) = match address.strip_prefix('[') {
        Some(rest) => match rest.split_once(']') {
            Some((host, "")) => (host, "", true),
            Some((host, after)) => match after.strip_prefix(':') {
                Some(port) => (host, port, true),
                None => return false,
            },
            None => return false,
        },
        None => match address.split_once(':') {
            Some((host, port)) if !port.contains(':') => (host, port, false),
            _ => (address, "", false),
        },
    };
    let host_ok = match bracketed || host.contains(':') {
        true => host.parse::<Ipv6Addr>().is_ok(),
        false => !host.is_empty() && !host.contains(|c: char| c.is_whitespace() || c.is_control()),
    };
    let port_ok = port.is_empty()
        || (port.bytes().all(|b| b.is_ascii_digit()) && port.parse::<u16>().is_ok());
    host_ok && port_ok
}

/// The shown server a command names: its position from 1, its name (any case) or its address.
pub fn find(list: &List, query: &str) -> Result<(usize, Server), String> {
    let servers = list.servers();
    let q = query.trim();
    q.parse::<usize>()
        .ok()
        .filter(|n| (1..=servers.len()).contains(n))
        .map(|n| n - 1)
        .or_else(|| servers.iter().position(|s| s.name.eq_ignore_ascii_case(q)))
        .or_else(|| servers.iter().position(|s| s.ip.eq_ignore_ascii_case(q)))
        .map(|i| (i, servers[i].clone()))
        .ok_or_else(|| trf("No server {query} in the list", &[("query", q)]))
}

#[cfg(test)]
mod tests;
