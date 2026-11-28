//! The featured servers a profile took off its Servers tab. They are kept beside the profile's
//! `servers.dat` in a small file of their own, one key a line (the address, or the name of a
//! server with none, in lower case), so the game's own list is never touched.
use super::Featured;
use std::path::Path;

const FILE: &str = "sunrise-hidden-featured.txt";

/// What a featured server is told by in the file.
pub(crate) fn key(server: &Featured) -> String {
    server.address.unwrap_or(server.name).to_ascii_lowercase()
}

/// The keys hidden in the game folder `home`; none when there is no file.
pub(crate) fn load(home: &Path) -> Vec<String> {
    std::fs::read_to_string(home.join(FILE))
        .map(|text| {
            text.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Writes `keys` as the hidden ones; with none, the file goes.
pub(crate) fn save(home: &Path, keys: &[String]) -> Result<(), String> {
    let path = home.join(FILE);
    if keys.is_empty() {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        };
    }
    std::fs::create_dir_all(home)
        .and_then(|()| std::fs::write(&path, keys.join("\n") + "\n"))
        .map_err(|e| e.to_string())
}
