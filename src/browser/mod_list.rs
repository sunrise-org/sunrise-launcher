//! The game's mod folder and the on/off list beside `options.txt` (`sunrise_client::mod_list`):
//! which of Sunrise's mods are turned on, in `sunrise-mods.txt`.
use std::fs;
use std::path::Path;

/// The game's mod folder and on-list, beside `options.txt`.
pub const MODS_DIR: &str = "sunrise-mods";
const MODS_LIST: &str = "sunrise-mods.txt";

/// The saved on/off list, which can only be read from a regular file.
pub(crate) fn selection_text(home: &Path) -> String {
    let path = home.join(MODS_LIST);
    if path.is_file() {
        fs::read_to_string(path).unwrap_or_default()
    } else {
        String::new()
    }
}

/// Whether mod `id` is on in `sunrise-mods.txt`.
pub fn is_enabled(home: &Path, id: &str) -> bool {
    let key = format!("mod.{id}");
    selection_text(home)
        .lines()
        .filter_map(|l| l.split_once(':'))
        .any(|(k, v)| k.trim() == key && v.trim() == "true")
}

/// Whether the saved list, which can be read, says mod `id` is off: a list that can't be read
/// says nothing, and a change to it is left to fail where it is made.
pub fn is_off(home: &Path, id: &str) -> bool {
    home.join(MODS_LIST).is_file() && !is_enabled(home, id)
}

/// Turns mod `id` on (or off) in `sunrise-mods.txt`, keeping the other lines.
pub fn set_enabled(home: &Path, id: &str, on: bool) -> Result<(), String> {
    set_all_enabled(home, &[id], on)
}

/// Turns every mod in `ids` on (or off) in one locked rewrite of `sunrise-mods.txt`: either all
/// of them change or, when the write fails, none do.
pub fn set_all_enabled(home: &Path, ids: &[&str], on: bool) -> Result<(), String> {
    let _lock = super::lock_home(home).map_err(|e| e.to_string())?;
    if on {
        super::sunrise::txn::require_current_update(home, ids)?;
    }
    let path = home.join(MODS_LIST);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    let keys: Vec<String> = ids.iter().map(|id| format!("mod.{id}")).collect();
    let mut out: String = text
        .split_inclusive('\n')
        .filter(|l| {
            l.split_once(':')
                .is_none_or(|(k, _)| !keys.iter().any(|key| k.trim() == key))
        })
        .collect();
    if on && !keys.is_empty() {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        for key in keys {
            out.push_str(&key);
            out.push_str(":true\n");
        }
    }
    fs::write(&path, out).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_on_list_keeps_other_lines() {
        let home = std::env::temp_dir().join(format!("sunrise-java-mods-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(&home).unwrap();
        fs::write(
            home.join(MODS_LIST),
            "mod.coords_hud:true\nmod.ruby:false\n",
        )
        .unwrap();
        set_enabled(&home, "ruby", true).unwrap();
        set_enabled(&home, "gem", true).unwrap();
        assert_eq!(
            fs::read_to_string(home.join(MODS_LIST)).unwrap(),
            "mod.coords_hud:true\nmod.ruby:true\nmod.gem:true\n"
        );
        set_enabled(&home, "ruby", false).unwrap();
        set_enabled(&home, "gem", false).unwrap();
        assert_eq!(
            fs::read_to_string(home.join(MODS_LIST)).unwrap(),
            "mod.coords_hud:true\n"
        );
        fs::remove_dir_all(&home).unwrap();
    }
}
