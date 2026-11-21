use super::*;
use crate::export::export;
use std::io::Write;
use std::time::Duration;

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "sunrise-launcher-import-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn zip_file(path: &Path, files: &[(&str, &[u8])]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
    for (name, bytes) in files {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
}

fn put(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

/// Makes instances as folders of `root`, refusing a name already taken, as the launcher does.
fn maker(root: &Path) -> impl Fn(&str) -> Result<Instance, String> + '_ {
    move |name: &str| {
        let home = root.join(name.to_lowercase().replace(' ', "-"));
        if home.exists() {
            return Err(instances::name_taken(name));
        }
        fs::create_dir_all(&home).unwrap();
        Ok(Instance {
            id: name.to_lowercase().replace(' ', "-"),
            name: name.into(),
            kind: instances::Kind::Own,
            work: home.clone(),
            home,
            incarnation: None,
            icon: None,
            group: None,
            version: None,
            channel: Default::default(),
            native_profile: crate::native_mods::Profile::Optimized,
            played: Duration::ZERO,
            last_played: None,
            launch: Default::default(),
        })
    }
}

const NONE: &dyn Fn(&str, f32) = &|_, _| {};

/// Bytes `build_import::is_game` accepts as a game build: this platform's
/// executable magic followed by the mod API mark every Sunrise program
/// carries. Not a real program -- `what` only has to recognise one.
fn fake_program() -> Vec<u8> {
    let magic: &[u8] = if cfg!(windows) {
        b"MZ"
    } else if cfg!(target_os = "macos") {
        &[0xcf, 0xfa, 0xed, 0xfe]
    } else {
        b"ELF"
    };
    let mut bytes = magic.to_vec();
    bytes.extend_from_slice(&sunrise_launcher_smod::game_identity::MARK);
    bytes
}

#[test]
fn says_what_a_dropped_file_is() {
    let dir = scratch("what");
    let mrpack = |name: &str, deps: &str| {
        let path = dir.join(name);
        let index = format!(r#"{{"formatVersion":1,"name":"P","files":[],"dependencies":{deps}}}"#);
        zip_file(&path, &[("modrinth.index.json", index.as_bytes())]);
        path
    };
    let cases: Vec<(PathBuf, Result<Dropped, ()>)> = vec![
        // Modrinth modpacks and Java modpacks are not imported.
        (mrpack("shared.mrpack", r#"{"minecraft":"26.3"}"#), Err(())),
        (
            mrpack(
                "java.mrpack",
                r#"{"minecraft":"1.21.1","fabric-loader":"0.16"}"#,
            ),
            Err(()),
        ),
        {
            let p = dir.join("cf.zip");
            zip_file(
                &p,
                &[("manifest.json", b"{}"), ("overrides/config/a.toml", b"")],
            );
            (p, Err(()))
        },
        {
            let p = dir.join("prism.zip");
            zip_file(
                &p,
                &[
                    ("Example/mmc-pack.json", b"{}"),
                    ("Example/.minecraft/mods/example.jar", b""),
                ],
            );
            (p, Err(()))
        },
        {
            let p = dir.join("rp.zip");
            zip_file(&p, &[("pack.mcmeta", b"{}"), ("assets/a.png", b"")]);
            (p, Ok(Dropped::Pack(packs::Kind::Resource)))
        },
        {
            let p = dir.join("dp.zip");
            zip_file(
                &p,
                &[
                    ("pack.mcmeta", b"{}"),
                    ("data/a/function/f.mcfunction", b""),
                ],
            );
            (p, Ok(Dropped::DataPack))
        },
        {
            let p = dir.join("sp.zip");
            zip_file(&p, &[("shaders/final.fsh", b"")]);
            (p, Ok(Dropped::Pack(packs::Kind::Shader)))
        },
        {
            let p = dir.join("glow.sshader");
            zip_file(&p, &[("sunrise-shaders.toml", b"")]);
            (p, Ok(Dropped::Pack(packs::Kind::Shader)))
        },
        {
            let p = dir.join("folder.zip");
            zip_file(
                &p,
                &[("options.txt", b"lang:fr_ch\n"), ("saves/W/level.dat", b"")],
            );
            (p, Ok(Dropped::Instance))
        },
        // A game build: the Windows program, taken on its extension without
        // being read, so `build_import` is the one to say why a file is not a
        // game for this computer.
        {
            let p = dir.join("sunrise-client.exe");
            put(&p, b"not read");
            (p, Ok(Dropped::Build))
        },
        // A browser's second copy keeps the extension, so it is a build too.
        {
            let p = dir.join("sunrise-client (1).exe");
            put(&p, b"not read");
            (p, Ok(Dropped::Build))
        },
        // A build for Linux or macOS has no extension: read, and recognised
        // by its magic and the mod API mark.
        {
            let p = dir.join("sunrise-client");
            put(&p, &fake_program());
            (p, Ok(Dropped::Build))
        },
        // An Actions artifact is a zip around the program: nothing a pack or
        // an instance holds matches, so the build check is the one that does.
        {
            let p = dir.join("artifact.zip");
            // Named as the game is on this platform: from_zip looks for that
            // entry, which is sunrise-client.exe on Windows.
            zip_file(&p, &[(crate::backend::CLIENT, fake_program().as_slice())]);
            (p, Ok(Dropped::Build))
        },
        // Named as the game but holding nothing like it: not a build.
        {
            let p = dir.join("sunrise-client-notes");
            put(&p, b"just notes");
            (p, Err(()))
        },
        // An extensionless file that is not the game is still nothing.
        {
            let p = dir.join("README");
            put(&p, b"hello");
            (p, Err(()))
        },
        {
            let p = dir.join("random.zip");
            zip_file(&p, &[("readme.txt", b"")]);
            (p, Err(()))
        },
        {
            let p = dir.join("picture.png");
            put(&p, b"png");
            (p, Err(()))
        },
    ];
    for (file, want) in cases {
        assert_eq!(what(&file).map_err(|_| ()), want, "{}", file.display());
    }
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_dropped_zip_reports_the_central_directory_limit() {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.start_file("manifest.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"{}").unwrap();
    let mut bytes = zip.finish().unwrap().into_inner();
    // A ZIP32 end record with no comment is 22 bytes. Its directory size starts at byte 12.
    let size = bytes.len() - 22 + 12;
    bytes[size..size + 4].copy_from_slice(&(128_u32 << 20).to_le_bytes());

    let scratch = tempfile::tempdir().unwrap();
    for name in ["oversized.zip", "oversized.sshader"] {
        let path = scratch.path().join(name);
        fs::write(&path, &bytes).unwrap();
        let error = what(&path).unwrap_err();
        assert!(error.contains("ZIP central directory size"), "{error}");
    }
}

#[test]
fn an_export_comes_back_as_an_instance_and_nothing_escapes_its_folder() {
    let dir = scratch("roundtrip");
    let home = dir.join("from");
    put(&home.join("options.txt"), b"lang:fr_ch\n");
    put(&home.join("resourcepacks/Glass.zip"), b"glass");
    put(
        &home.join("sunrise-mods/coords/sunrise-mod.toml"),
        b"id = \"coords\"\n",
    );
    put(&home.join("saves/Base/region/r.0.0.mca"), b"region");
    put(&home.join("saves/Base/level.dat"), b"not nbt");
    let made = dir.join("instances");
    {
        let out = dir.join("Shared.zip");
        export(&home, "Shared", &["Base".into()], &out).unwrap();
        let i = import_into(&out, NONE, &maker(&made)).unwrap();
        for f in [
            "options.txt",
            "resourcepacks/Glass.zip",
            "sunrise-mods/coords/sunrise-mod.toml",
            "saves/Base/region/r.0.0.mca",
        ] {
            assert_eq!(
                fs::read(i.home.join(f)).unwrap(),
                fs::read(home.join(f)).unwrap(),
                "{f}"
            );
        }
    }
    assert!(made.join("shared").is_dir());

    // A zip that reaches out of its folder, or carries what an instance doesn't keep.
    let evil = dir.join("evil.zip");
    zip_file(
        &evil,
        &[
            ("options.txt", b"ok"),
            ("../outside.txt", b"no"),
            ("saves/../../up.txt", b"no"),
            ("logs/latest.log", b"no"),
            ("session.json", b"no"),
        ],
    );
    let i = import_into(&evil, NONE, &maker(&made)).unwrap();
    assert!(i.home.join("options.txt").is_file());
    assert!(!dir.join("outside.txt").exists() && !made.join("up.txt").exists());
    assert!(!i.home.join("logs").exists() && !i.home.join("session.json").exists());

    // A Modrinth modpack is not imported, and nothing is made for it.
    let far = dir.join("far.mrpack");
    zip_file(&far, &[("modrinth.index.json", b"{}")]);
    assert!(import_into(&far, NONE, &maker(&made)).is_err());
    assert!(!made.join("far").exists());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_zip_cannot_plant_the_browsers_list() {
    let dir = scratch("planted-list");
    let made = dir.join("instances");
    let pack = dir.join("planted.zip");
    let list = br#"{"entries":[{"source":"modrinth","project":"p","slug":"p","title":"p",
        "kind":"resource-pack","version":"1","number":"1","file":"resourcepacks/a.zip",
        "checksum":"SHA-512:00","page":""}]}"#;
    zip_file(
        &pack,
        &[
            ("options.txt", b"ok"),
            ("mod-browser.json", list),
            ("Mod-Browser.json", list),
        ],
    );
    let i = import_into(&pack, NONE, &maker(&made)).unwrap();
    let names: Vec<_> = fs::read_dir(&i.home)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["options.txt"]);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_zip_bomb_stops_at_the_limit_and_leaves_nothing() {
    let dir = scratch("bomb");
    let made = dir.join("instances");
    // 64 KiB of zeros compresses to almost nothing; the limit here is 16 KiB.
    let bomb = dir.join("bomb.zip");
    zip_file(
        &bomb,
        &[
            ("options.txt", b"ok"),
            ("saves/Big/region/r.0.0.mca", &[0u8; 64 << 10]),
        ],
    );
    assert!(fs::metadata(&bomb).unwrap().len() < 4 << 10);
    let refused = import_any(&bomb, NONE, &maker(&made), 16 << 10).unwrap_err();
    assert!(refused.contains("more than 16 KiB"), "{refused}");
    assert!(!made.join("bomb").exists(), "the half-made profile goes");
    // Under the limit, all of it comes in; the limit counts every file together.
    let two = dir.join("two.zip");
    zip_file(
        &two,
        &[
            ("options.txt", &[1u8; 8 << 10]),
            ("servers.dat", &[2u8; 8 << 10]),
        ],
    );
    let i = import_any(&two, NONE, &maker(&made), 16 << 10).unwrap();
    assert_eq!(fs::read(i.home.join("servers.dat")).unwrap().len(), 8 << 10);
    let three = dir.join("three.zip");
    zip_file(
        &three,
        &[
            ("options.txt", &[1u8; 8 << 10]),
            ("servers.dat", &[2u8; 8 << 10 | 1]),
        ],
    );
    assert!(import_any(&three, NONE, &maker(&made), 16 << 10).is_err());
    fs::remove_dir_all(&dir).unwrap();
}
