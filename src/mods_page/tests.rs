use super::*;
use std::{fs, io::Write as _};
use sunrise_launcher::{browser::mod_list, native_status};

#[test]
fn build_consent_binds_an_unopened_mods_page_to_the_selected_profile() {
    const CHILD: &str = "SUNRISE_BUILD_CONSENT_PROFILE_TEST";
    if std::env::var_os(CHILD).is_none() {
        let root = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "mods_page::tests::build_consent_binds_an_unopened_mods_page_to_the_selected_profile",
            ])
            .env(CHILD, "1")
            .env("SUNRISE_LAUNCHER_HOME", root.path().join("launcher"))
            .env("SUNRISE_HOME", root.path().join("default"))
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }

    let default = backend::game_home();
    let selected = default.with_file_name("selected");
    for (home, id) in [(&default, "default_mod"), (&selected, "selected_mod")] {
        let dir = home.join(native_mods::INSTALLED);
        fs::create_dir_all(&dir).unwrap();
        let mut zip =
            zip::ZipWriter::new(fs::File::create(dir.join(format!("{id}.smod"))).unwrap());
        let pin = "f".repeat(40);
        let manifest = format!("id = '{id}'\nname = '{id}'\nversion = '1.0.0'\napi = '0.2'\ncapabilities = ['registries']\n[native]\ngame = '26.3'\nsunrise = '{pin}'\nmixins = '{pin}'\n");
        for (name, text) in [
            ("sunrise-mod.toml", manifest.as_str()),
            (
                "Cargo.toml",
                "[package]\nname = 'fixture'\nversion = '1.0.0'\n",
            ),
            ("src/lib.rs", "pub fn fixture() {}\n"),
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        mod_list::set_enabled(home, id, true).unwrap();
    }
    let untouched = fs::read(default.join("sunrise-mods.txt")).unwrap();
    let mut settings = backend::Settings::default();
    for allow in [false, true] {
        mod_list::set_enabled(&selected, "selected_mod", true).unwrap();
        let mut home = ModsHome::default();
        assert_eq!(home.current(), default, "Mods has never opened");
        let read = home.begin_build_consent(selected.clone());
        let mut pending = Some(PendingConsent::ReadingBuild(read.clone()));
        assert!(native_consent::allow(&mut pending, true, &mut settings)
            .unwrap()
            .is_none());
        assert!(settings.native_mods_allowed_at.is_none());
        pending
            .as_mut()
            .unwrap()
            .finish_read(&read, native_status::build_inputs(&read, None));
        assert_eq!(
            pending.as_ref().unwrap().rows()[0].label,
            "selected_mod (selected_mod.smod, not checked against the registry)"
        );
        assert_eq!(home.current(), selected);
        if allow {
            let action = native_consent::allow(&mut pending, true, &mut settings).unwrap();
            assert!(matches!(action, Some(PendingConsent::Build(inputs))
                if matches!(inputs.as_slice(), [one] if one.id == "selected_mod")));
            assert!(settings.native_mods_allowed_at.is_some());
        } else {
            drop(pending.take());
            assert!(settings.native_mods_allowed_at.is_none());
        }
        assert!(pending.is_none());
        assert!(mod_list::is_enabled(&selected, "selected_mod"));
        assert_eq!(
            fs::read(default.join("sunrise-mods.txt")).unwrap(),
            untouched
        );
    }
}
