//! The game's mod folder and the on/off list beside `options.txt` (`sunrise_client::mod_list`):
//! which of Sunrise's mods are turned on, in `sunrise-mods.txt`.
/// The game's mod folder and on-list, beside `options.txt`.
pub const MODS_DIR: &str = "sunrise-mods";

/// The saved on/off list, which can only be read from a regular file.
pub(crate) use sunrise_launcher_smod::mod_list::selection_text;
#[cfg(test)]
pub(crate) use sunrise_launcher_smod::mod_list::FILE as MODS_LIST;
pub use sunrise_launcher_smod::mod_list::{is_enabled, set_all_enabled, set_enabled};

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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
