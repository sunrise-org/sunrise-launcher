//! Source-only native packages. Reading a package never executes its code.
use super::Facts;
use std::{collections::BTreeMap, io::Read, path::Path};

/// The prebuilt package's layout number: the game repository's `sdk/package-format` file.
pub const PACKAGE_FORMAT: u32 = 6;

/// Every prebuilt archive's name starts with this. The `v<N>` is the package layout the launcher
/// reads ([`PACKAGE_FORMAT`]): a release made before it has no archive by this name.
pub fn prebuilt_prefix() -> String {
    format!("sunrise-prebuilt-v{PACKAGE_FORMAT}-")
}

/// Whether a valid package is native, irrespective of its install folder.
pub fn holds(path: &Path) -> bool {
    crate::Package::read(path).is_ok_and(|package| package.is_native())
}

/// A flat native mod and its checked, archive-relative files.
pub struct Bundle {
    pub manifest: crate::Manifest,
    pub files: BTreeMap<String, Vec<u8>>,
}

impl Bundle {
    pub fn read(path: &Path) -> Result<Self, String> {
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
        let package = crate::Package::read_zip(&mut zip)?;
        if !package.is_native() {
            return Err("not a native package".into());
        }
        // An install is the first open: the runtime files' hashes are checked here, before the
        // download is accepted.
        package.read_runtime(path)?;
        let mut files = BTreeMap::new();
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
            if entry.is_dir() {
                continue;
            }
            let name = entry.name().map_err(|e| e.to_string())?.into_owned();
            let size = entry.size();
            let mut bytes = Vec::new();
            (&mut entry)
                .take(size.saturating_add(1))
                .read_to_end(&mut bytes)
                .map_err(|e| format!("{name}: {e}"))?;
            if bytes.len() as u64 != size {
                return Err(format!("{name}: invalid expanded size"));
            }
            // The package reader never opens a source file; the build that does refuses a
            // prebuilt binary here.
            if crate::manifest::source_file(&name)
                && let Some(kind) = crate::manifest::binary_kind(&bytes[..bytes.len().min(1024)])
            {
                return Err(crate::manifest::ContentFileError::Binary { name, kind }.to_string());
            }
            files.insert(name, bytes);
        }
        Ok(Self {
            manifest: package.manifest,
            files,
        })
    }

    /// Registry facts from the one package manifest, never Cargo metadata.
    pub fn facts(&self) -> Facts {
        let native = self.manifest.native.as_ref().expect("native bundle");
        Facts {
            id: Some(self.manifest.id.clone()),
            version: Some(self.manifest.version.clone()),
            // Packages built with the mod tool carry the resolved pin. A source-only archive
            // can still omit it; record only what the manifest names when no SDK is at hand.
            api: match &native.mixins {
                Some(mixins) => format!("native-1/{}/{mixins}", native.sunrise),
                None => format!("native-1/{}", native.sunrise),
            },
            game: Some(native.game.clone()),
            dependencies: self.manifest.dependencies.clone(),
            ..Facts::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn manifest() -> String {
        format!(
            "id = 'veinminer'\nname = 'Veinminer'\nversion = '0.3.0'\napi = '0.2'\ncapabilities = ['registries']\n[native]\ngame = '26.3'\nsunrise = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        )
    }

    fn read(name: &str, manifest: &str, extras: &[(&str, &[u8])]) -> Result<Bundle, String> {
        let path =
            std::env::temp_dir().join(format!("native-bundle-{name}-{}.smod", std::process::id()));
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        for (name, bytes) in [
            (crate::manifest::MANIFEST, manifest.as_bytes()),
            (
                "Cargo.toml",
                b"[package]\nname='other'\nversion='9'\n".as_slice(),
            ),
            ("src/lib.rs", b"fn hook() {}".as_slice()),
        ]
        .into_iter()
        .chain(extras.iter().copied())
        {
            zip.start_file(name, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
        let result = Bundle::read(&path);
        std::fs::remove_file(path).unwrap();
        result
    }

    #[test]
    fn a_source_archive_can_defer_its_mixins_pin_to_the_sdk() {
        let text = manifest();
        let build = "a".repeat(40);
        let pin = "b".repeat(40);
        let explicit = read("explicit-pin", &text, &[]).unwrap();
        assert_eq!(explicit.facts().api, format!("native-1/{build}/{pin}"));

        let omitted = text.replace(&format!("mixins = '{pin}'\n"), "");
        let implicit = read("omitted-pin", &omitted, &[]).unwrap();
        assert_eq!(implicit.facts().api, format!("native-1/{build}"));
        assert_eq!(implicit.manifest.native.unwrap().mixins, None);

        let bad = omitted + "mixins = 'main'\n";
        assert!(
            read("bad-pin", &bad, &[])
                .err()
                .unwrap()
                .contains("exact revisions")
        );
    }

    #[test]
    fn an_install_refuses_runtime_bytes_that_do_not_match_the_manifest() {
        let manifest = format!(
            "{}[runtime]\n\"nav.bin\"='{}'\n",
            manifest(),
            crate::manifest::sha512_of(b"tables")
        );
        let bundle = read("runtime-ok", &manifest, &[("runtime/nav.bin", b"tables")]).unwrap();
        assert_eq!(bundle.files["runtime/nav.bin"], b"tables");
        // The same name and size, different bytes.
        let error = read(
            "runtime-edited",
            &manifest,
            &[("runtime/nav.bin", b"tablez")],
        )
        .err()
        .unwrap();
        assert!(error.contains("runtime/nav.bin"), "{error}");
    }

    #[test]
    fn facts_use_manifest_identity_and_exact_build_pins() {
        let bundle = read("facts", &manifest(), &[]).unwrap();
        let facts = bundle.facts();
        assert_eq!(facts.id.as_deref(), Some("veinminer"));
        assert_eq!(facts.version.as_deref(), Some("0.3.0"));
        assert_eq!(
            facts.api,
            format!("native-1/{}/{}", "a".repeat(40), "b".repeat(40))
        );
        assert_eq!(facts.game.as_deref(), Some("26.3"));
    }

    #[test]
    fn package_carries_resources_and_code_together() {
        let bytes = [0, 255, 42];
        let bundle = read(
            "resources",
            &manifest(),
            &[
                ("data/veinminer/table.bin", &bytes),
                ("assets/veinminer/lang/en_us.json", b"{}"),
            ],
        )
        .unwrap();
        assert_eq!(bundle.files["data/veinminer/table.bin"], bytes);
        assert_eq!(bundle.files["src/lib.rs"], b"fn hook() {}");
        assert!(bundle.files.contains_key(crate::manifest::MANIFEST));
    }

    #[test]
    fn any_other_file_is_carried_as_source() {
        for (i, name) in [
            "sunrise-native.json",
            "native/rust/src/lib.rs",
            "vendor/zlib/inflate.c",
        ]
        .into_iter()
        .enumerate()
        {
            let bundle = read(&format!("carried-{i}"), &manifest(), &[(name, b"int x;")]).unwrap();
            assert_eq!(bundle.files[name], b"int x;");
        }
    }

    #[test]
    fn a_prebuilt_binary_among_the_source_is_refused_by_its_bytes() {
        for (i, bytes) in [&b"\x7fELF\x02\x01"[..], b"!<arch>\nlib.o/"]
            .into_iter()
            .enumerate()
        {
            let error = read(
                &format!("binary-{i}"),
                &manifest(),
                &[("vendor/lib.c", bytes)],
            )
            .err()
            .unwrap();
            assert!(error.starts_with("vendor/lib.c: "), "{error}");
            assert!(error.contains("prebuilt binaries"), "{error}");
        }
    }

    #[test]
    fn refuses_unsafe_entries_and_unknown_fields() {
        for (i, name) in ["../escape.rs", "src/../escape.rs"].iter().enumerate() {
            assert!(
                read(&format!("invalid-{i}"), &manifest(), &[(name, b"bad")]).is_err(),
                "{name}"
            );
        }
        assert!(read("unknown", &(manifest() + "unexpected = true\n"), &[]).is_err());
        assert!(read("pin", &manifest().replace(&"a".repeat(40), "main"), &[]).is_err());
        assert!(
            read(
                "magic",
                &manifest(),
                &[("assets/veinminer/file.bin", b"MZ bad")]
            )
            .is_err()
        );
    }

    #[test]
    fn classification_uses_the_validated_package_not_install_folder() {
        let root = std::env::temp_dir().join(format!("native-kind-{}", std::process::id()));
        for (folder, native) in [("content", true), ("native", false)] {
            let dir = root.join(folder);
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("test.smod");
            let text = manifest();
            let (text, files) = if native {
                (
                    text.as_str(),
                    vec![
                        (
                            "Cargo.toml",
                            b"[package]\nname='example'\nversion='1'".as_slice(),
                        ),
                        ("src/lib.rs", b"fn hook() {}".as_slice()),
                    ],
                )
            } else {
                (text.split("[native]").next().unwrap(), vec![])
            };
            std::fs::write(&path, crate::Package::write_files(text, &files).unwrap()).unwrap();
            assert_eq!(holds(&path), native);
            if native {
                let directory = dir.join("source");
                zip::ZipArchive::new(std::fs::File::open(&path).unwrap())
                    .unwrap()
                    .extract(&directory)
                    .unwrap();
                assert!(holds(&directory));
                std::fs::remove_file(directory.join(crate::manifest::MANIFEST)).unwrap();
                assert!(
                    !holds(&directory),
                    "Cargo alone is not a valid native package"
                );
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
