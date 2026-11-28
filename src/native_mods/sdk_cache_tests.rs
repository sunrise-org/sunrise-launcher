use super::*;
use sunrise_launcher_release_verify::{sha256_hex, sums_file, SigningKey};
use sunrise_launcher_sdk::{Build, Mixins, Surface, Toolchain};

pub(crate) const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const TRIPLE: &str = "x86_64-unknown-linux-gnu";
/// An xz tar the packager wrote, holding `deps/a.rmeta`.
const SLICE: &[u8] = include_bytes!("../../crates/sdk/src/sdk/fixture.tar.xz");

fn runtime() -> String {
    Asset::name(Kind::Runtime, TRIPLE, Side::Client)
}

fn api() -> String {
    Asset::name(Kind::Api, TRIPLE, Side::Client)
}

fn engine() -> String {
    Asset::name(Kind::Engine, TRIPLE, Side::Client)
}

/// An index for [`COMMIT`] listing `files`, made with this launcher's Rust.
pub(crate) fn index_of(files: &[(&str, &[u8])], rust: &str) -> Index {
    let assets = files
        .iter()
        .map(|(name, bytes)| {
            let (kind, codec, target, side) = Asset::parts_of(name).unwrap();
            let asset = Asset {
                sha256: sha256_hex(bytes),
                size: bytes.len() as u64,
                kind,
                codec,
                target,
                side,
            };
            (name.to_string(), asset)
        })
        .collect();
    Index {
        schema: sdk::SCHEMA,
        build: Build {
            commit: COMMIT.into(),
            game: crate::backend::PINNED_GAME.into(),
            date: "2026-11-13".into(),
            nightly: Some("nightly-2026-11-13".into()),
        },
        package_format: u64::from(sdk::PACKAGE_FORMAT),
        toolchain: Toolchain { rust: rust.into() },
        content_api: sunrise_launcher_smod::manifest::API.into(),
        surface: Surface {
            hash: "ab".repeat(16),
            previous: None,
            source: None,
            changes: Vec::new(),
            notes: Vec::new(),
        },
        mixins: Mixins {
            rev: "e59da9094f0c2fdda6c627fe9eff22dec6a906f0".into(),
            patch: "52eaeef7cac10d894066635dc46b12f76d877416317f06eaf385f176de5e1c35".into(),
        },
        assets,
        inputs: serde_json::from_str(include_str!("../../crates/sdk/src/sdk/inputs.json")).unwrap(),
    }
}

/// A release folder `tag` under `base` holding `files` and an index whose checksums `key` signs
/// for `signed_for`.
pub(crate) fn publish(
    base: &Path,
    tag: &str,
    signed_for: &str,
    key: &SigningKey,
    index: &[u8],
    files: &[(&str, &[u8])],
) {
    let folder = base.join(tag);
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join(INDEX), index).unwrap();
    let mut sums = vec![(INDEX.to_owned(), sha256_hex(index))];
    for (name, bytes) in files {
        fs::write(folder.join(name), bytes).unwrap();
        sums.push((name.to_string(), sha256_hex(bytes)));
    }
    let sums = sums_file(&sums);
    fs::write(folder.join(SUMS), &sums).unwrap();
    let signature = key.sign(signed_for, sums.as_bytes());
    fs::write(folder.join(SIGNATURE), &signature).unwrap();
    fs::write(folder.join(SIGNATURE_V2), &signature).unwrap();
}

struct Fixture {
    scratch: tempfile::TempDir,
    key: SigningKey,
}

impl Fixture {
    fn new() -> Self {
        Self {
            scratch: tempfile::tempdir().unwrap(),
            key: SigningKey::generate().unwrap().0,
        }
    }

    fn base(&self) -> Base {
        Base::Folder(self.scratch.path().join("base"))
    }

    fn kept(&self) -> PathBuf {
        self.scratch.path().join("sdk")
    }

    fn keys(&self) -> [PublicKey; 1] {
        [self.key.public()]
    }

    /// Publishes build [`COMMIT`]'s SDK with a runtime and an api slice and an index listing
    /// `listed` too, then reads its index.
    fn sdk(&self, listed: &[(&str, &[u8])]) -> Result<Sdk, Refusal> {
        let (runtime, api) = (runtime(), api());
        let mut files: Vec<(&str, &[u8])> = vec![(runtime.as_str(), SLICE), (api.as_str(), SLICE)];
        let index = index_of(
            &[files.clone(), listed.to_vec()].concat(),
            rust_toolchain::VERSION,
        )
        .to_json();
        files.retain(|(name, _)| !listed.iter().any(|(other, _)| other == name));
        let tag = tag(COMMIT);
        let base = self.scratch.path().join("base");
        publish(&base, &tag, &tag, &self.key, index.as_bytes(), &files);
        index_in(&self.kept(), &self.base(), COMMIT, &self.keys())
    }
}

#[test]
fn a_base_is_a_url_or_a_folder() {
    assert_eq!(Base::parse(None), Base::Url(DEFAULT_BASE.into()));
    assert_eq!(Base::parse(Some("".into())), Base::Url(DEFAULT_BASE.into()));
    assert_eq!(
        Base::parse(Some("https://mirror.example/sdk/".into())),
        Base::Url("https://mirror.example/sdk".into())
    );
    assert_eq!(
        Base::parse(Some("http://localhost:8000".into())),
        Base::Url("http://localhost:8000".into())
    );
    assert_eq!(
        Base::parse(Some("/media/usb/sunrise-sdk".into())),
        Base::Folder("/media/usb/sunrise-sdk".into())
    );
}

#[test]
fn a_signed_index_is_read_kept_and_used_offline() {
    let fixture = Fixture::new();
    let sdk = fixture.sdk(&[]).unwrap();
    assert_eq!(sdk.commit(), COMMIT);
    assert_eq!(sdk.shown(), "the Nov 13 update");
    assert!(fixture.kept().join(COMMIT).join(INDEX).is_file());
    // With nothing at the base any more, the kept index serves.
    fs::remove_dir_all(fixture.scratch.path().join("base")).unwrap();
    let again = index_in(&fixture.kept(), &fixture.base(), COMMIT, &fixture.keys()).unwrap();
    assert_eq!(again.index, sdk.index);
    // A build id that is not one is refused before any path is made of it.
    assert!(matches!(
        index_in(&fixture.kept(), &fixture.base(), "../x", &fixture.keys()),
        Err(Refusal::Refused { .. })
    ));
}

#[test]
fn a_kept_surface_with_same_size_corruption_is_replaced_or_refused() {
    const NAME: &str = "surface.json";
    let good: &[u8] = br#"{"hash":"a"}"#;
    let corrupt: &[u8] = br#"{"hash":"b"}"#;
    let fixture = Fixture::new();
    let base = fixture.base();
    let sdk = fixture.sdk(&[(NAME, good)]).unwrap();
    let mirror = base.local(&tag(COMMIT), NAME).unwrap();
    fs::write(&mirror, good).unwrap();
    let kept = surface_file(&sdk, &base).unwrap();
    assert_eq!(fs::read(&kept).unwrap(), good);

    fs::write(&kept, corrupt).unwrap();
    assert_eq!(surface_file(&sdk, &base).unwrap(), kept);
    assert_eq!(fs::read(&kept).unwrap(), good);

    fs::remove_file(&mirror).unwrap();
    let warm = surface_file(&sdk, &base).unwrap();
    assert_eq!(fs::read(warm).unwrap(), good);
    fs::write(&kept, corrupt).unwrap();
    assert_eq!(
        surface_file(&sdk, &base).unwrap_err(),
        Refusal::Refused {
            build: sdk.shown(),
            why: sdk::Refusal::Missing(vec![NAME.into()]).to_string(),
        }
    );
}

#[test]
fn a_source_mod_inherits_the_revision_of_its_verified_sdk() {
    let fixture = Fixture::new();
    let base = fixture.scratch.path().join("base");
    let tag = tag(COMMIT);
    let mixins = "b".repeat(40);
    let mut index = index_of(&[], rust_toolchain::VERSION);
    index.mixins.rev.clone_from(&mixins);
    publish(
        &base,
        &tag,
        &tag,
        &fixture.key,
        index.to_json().as_bytes(),
        &[],
    );
    let sdk = index_in(&fixture.kept(), &fixture.base(), COMMIT, &fixture.keys()).unwrap();

    let project = fixture.scratch.path().join("project");
    super::super::tests::write_bundle(&project, "implicit", "");
    fs::write(
        project.join(super::super::NATIVE_MANIFEST),
        format!("id='implicit'\nname='Implicit'\nversion='0.3.0'\napi='0.2'\ncapabilities=['registries']\n[native]\ngame='26.3'\nsunrise='{COMMIT}'\n"),
    ).unwrap();
    let mods = super::super::bundles(&project).unwrap();
    assert_eq!(
        super::super::mixins::pins(&mods, Some(&sdk), &fixture.base(), None).unwrap(),
        [mixins]
    );
}

#[test]
fn an_index_is_refused_by_name_when_its_signature_or_contents_do_not_hold() {
    let fixture = Fixture::new();
    let base = fixture.scratch.path().join("base");
    let tag = tag(COMMIT);
    let read = || index_in(&fixture.kept(), &fixture.base(), COMMIT, &fixture.keys());
    // No index at all: no SDK for that build.
    assert_eq!(
        read().unwrap_err(),
        Refusal::NoSdk {
            build: "your Sunrise version".into()
        }
    );
    let good = index_of(&[], rust_toolchain::VERSION).to_json();
    // Signed for a game release's tag, not this SDK's: refused.
    publish(
        &base,
        &tag,
        "v2099.01.01",
        &fixture.key,
        good.as_bytes(),
        &[],
    );
    assert!(matches!(read(), Err(Refusal::Unsigned { .. })));
    // Signed, but another index than the signed one is served.
    publish(&base, &tag, &tag, &fixture.key, good.as_bytes(), &[]);
    fs::write(
        base.join(&tag).join(INDEX),
        good.replace("2026-11-13", "2026-11-14"),
    )
    .unwrap();
    assert!(matches!(read(), Err(Refusal::Unsigned { .. })));
    // By a key the launcher doesn't trust.
    let stranger = SigningKey::generate().unwrap().0;
    publish(&base, &tag, &tag, &stranger, good.as_bytes(), &[]);
    assert!(matches!(read(), Err(Refusal::Unsigned { .. })));
    // A newer layout, or a newer Rust, needs a newer launcher; an older Rust, a newer game.
    let mut newer = index_of(&[], rust_toolchain::VERSION);
    newer.package_format += 1;
    publish(
        &base,
        &tag,
        &tag,
        &fixture.key,
        newer.to_json().as_bytes(),
        &[],
    );
    assert!(matches!(read(), Err(Refusal::UpdateLauncher { .. })));
    let newer_rust = index_of(&[], "999.0.0").to_json();
    publish(&base, &tag, &tag, &fixture.key, newer_rust.as_bytes(), &[]);
    assert!(matches!(read(), Err(Refusal::UpdateLauncher { .. })));
    let older_rust = index_of(&[], "1.0.0").to_json();
    publish(&base, &tag, &tag, &fixture.key, older_rust.as_bytes(), &[]);
    assert_eq!(
        read().unwrap_err(),
        Refusal::OlderRust {
            build: "the Nov 13 update".into(),
            rust: "1.0.0".into()
        }
    );
    // Nothing refused was kept.
    assert!(!fixture.kept().join(COMMIT).join(INDEX).exists());
}

#[test]
fn the_api_route_fetches_its_two_slices_only_and_reuses_them_on_their_stamps() {
    let fixture = Fixture::new();
    // The engine slice is listed but not published: the API route never asks for it.
    let engine = engine();
    let sdk = fixture
        .sdk(&[(engine.as_str(), &b"never read"[..])])
        .unwrap();
    let prebuilt = fixture.scratch.path().join("prebuilt");
    let heard = std::cell::RefCell::new(Vec::new());
    let package = slices_in(
        &prebuilt,
        &sdk,
        &fixture.base(),
        "prebuilt-api",
        TRIPLE,
        Side::Client,
        &|fetched| heard.borrow_mut().push(fetched),
    )
    .unwrap();
    assert_eq!(
        package,
        Package::at(&prebuilt.join(format!("{COMMIT}-{TRIPLE}")))
    );
    for slice in [&package.runtime, &package.api] {
        assert_eq!(fs::read(slice.join("deps/a.rmeta")).unwrap().len(), 8);
        assert!(stamp_path(slice).is_file());
    }
    assert!(!package.engine.exists());
    let total = 2 * SLICE.len() as u64;
    assert_eq!(heard.borrow().first(), Some(&Fetched { done: 0, total }));
    assert_eq!(heard.borrow().last(), Some(&Fetched { done: total, total }));
    // The engine route needs the slice the base doesn't have: refused by name.
    let refused = slices_in(
        &prebuilt,
        &sdk,
        &fixture.base(),
        "prebuilt",
        TRIPLE,
        Side::Client,
        &|_| {},
    )
    .unwrap_err();
    assert!(refused.to_string().contains(&engine), "{refused}");
    // Kept slices are used again on their stamps alone, with nothing at the base.
    fs::remove_dir_all(fixture.scratch.path().join("base")).unwrap();
    heard.borrow_mut().clear();
    let again = slices_in(
        &prebuilt,
        &sdk,
        &fixture.base(),
        "prebuilt-api",
        TRIPLE,
        Side::Client,
        &|fetched| heard.borrow_mut().push(fetched),
    )
    .unwrap();
    assert_eq!(again, package);
    assert_eq!(*heard.borrow(), [Fetched { done: 0, total: 0 }]);
    // A file whose size changed, or a file that was added, fails the stamp: fetched again
    // (and here refused, the base being gone).
    let changes: [fn(&Path); 2] = [
        |slice| fs::write(slice.join("deps/a.rmeta"), b"longer than eight").unwrap(),
        |slice| fs::write(slice.join("deps/extra.rmeta"), b"").unwrap(),
    ];
    for change in changes {
        let fresh = slices_in(
            &prebuilt,
            &sdk,
            &fixture.base(),
            "prebuilt-api",
            TRIPLE,
            Side::Client,
            &|_| {},
        );
        assert!(fresh.is_ok());
        change(&package.api);
        let stale = slices_in(
            &prebuilt,
            &sdk,
            &fixture.base(),
            "prebuilt-api",
            TRIPLE,
            Side::Client,
            &|_| {},
        );
        assert!(matches!(stale, Err(Refusal::Refused { .. })), "{stale:?}");
        // What failed its stamp is gone, never used half-checked.
        assert!(!package.api.exists());
        // Put it back for the next change.
        fs::create_dir_all(package.api.join("deps")).unwrap();
        fs::copy(
            package.runtime.join("deps/a.rmeta"),
            package.api.join("deps/a.rmeta"),
        )
        .unwrap();
        fs::copy(stamp_path(&package.runtime), stamp_path(&package.api)).unwrap();
        let stamp = fs::read_to_string(stamp_path(&package.api)).unwrap();
        fs::write(stamp_path(&package.api), stamp.replace(&runtime(), &api())).unwrap();
    }
}

#[test]
fn using_a_kept_tool_offline_does_not_mark_or_evict_sdk_slices() {
    let fixture = Fixture::new();
    let name = Asset::name(Kind::Tool, TRIPLE, Side::Client);
    let sdk = fixture.sdk(&[(name.as_str(), SLICE)]).unwrap();
    let prebuilt = fixture.scratch.path().join("prebuilt");
    let into = prebuilt.join(format!("{COMMIT}-{TRIPLE}-tool"));
    let program = into.join(super::super::tool::PROGRAM);
    fs::create_dir_all(&into).unwrap();
    fs::write(&program, b"kept tool").unwrap();
    let stamp = Stamp {
        index: sdk.signed().to_owned(),
        asset: name,
        files: listing(&into).unwrap(),
    };
    let stamp_bytes = serde_json::to_vec(&stamp).unwrap();
    fs::write(stamp_path(&into), &stamp_bytes).unwrap();
    let other = prebuilt.join(format!("{COMMIT}-{TRIPLE}")).join("runtime");
    fs::create_dir_all(&other).unwrap();
    let canary = other.join("kept-resource");
    fs::write(&canary, b"other SDK slice").unwrap();
    fs::remove_dir_all(fixture.scratch.path().join("base")).unwrap();

    // Writing .last-used inside the tool folder would invalidate its exact file listing and
    // make the next offline lookup discard it while trying to fetch a replacement.
    for _ in 0..2 {
        let heard = std::cell::RefCell::new(Vec::new());
        let found = tool_in(&prebuilt, &sdk, &fixture.base(), TRIPLE, &|fetched| {
            heard.borrow_mut().push(fetched);
        })
        .unwrap();
        assert_eq!(found, program);
        assert_eq!(*heard.borrow(), [Fetched::default()]);
        assert_eq!(listing(&into), Some(stamp.files.clone()));
        assert_eq!(fs::read(&program).unwrap(), b"kept tool");
        assert_eq!(fs::read(stamp_path(&into)).unwrap(), stamp_bytes);
        assert_eq!(fs::read(&canary).unwrap(), b"other SDK slice");
    }
}

#[test]
fn a_stamp_from_another_index_is_not_trusted() {
    let fixture = Fixture::new();
    let sdk = fixture.sdk(&[]).unwrap();
    let prebuilt = fixture.scratch.path().join("prebuilt");
    let package = slices_in(
        &prebuilt,
        &sdk,
        &fixture.base(),
        "prebuilt-api",
        TRIPLE,
        Side::Client,
        &|_| {},
    )
    .unwrap();
    let other = Sdk {
        sha256: "0".repeat(64),
        ..sdk.clone()
    };
    assert!(stamp_holds(&sdk, &runtime(), &package.runtime));
    assert!(!stamp_holds(&other, &runtime(), &package.runtime));
    assert!(!stamp_holds(&sdk, &api(), &package.runtime));
}

#[test]
fn a_game_release_names_its_build_through_its_signed_index() {
    let fixture = Fixture::new();
    let base = fixture.scratch.path().join("base");
    let kept = fixture.scratch.path().join("versions");
    let index = index_of(&[], rust_toolchain::VERSION).to_json();
    let version = "nightly-2026-11-13-0123456";
    // A Nightly: its dated release, signed for its own version.
    publish(
        &base,
        "nightly-2026-11-13",
        version,
        &fixture.key,
        index.as_bytes(),
        &[],
    );
    let read = |version: &str| build_of_in(&kept, version, &fixture.base(), &fixture.keys());
    assert_eq!(read(version).unwrap(), COMMIT);
    assert_eq!(fs::read_to_string(kept.join(version)).unwrap(), COMMIT);
    // Kept: known offline.
    fs::remove_dir_all(&base).unwrap();
    assert_eq!(read(version).unwrap(), COMMIT);
    // The Nightly being served, under `nightly`: read there, signed for its own version.
    let current = "nightly-2026-11-14-0123456";
    publish(
        &base,
        sunrise_launcher_release_verify::NIGHTLY,
        current,
        &fixture.key,
        index.as_bytes(),
        &[],
    );
    assert_eq!(read(current).unwrap(), COMMIT);
    // `nightly` serving another version doesn't pass for this one; its dated tag is read next.
    let older = "nightly-2026-11-12-0123456";
    assert!(matches!(read(older), Err(Refusal::NoSdk { .. })));
    publish(
        &base,
        "nightly-2026-11-12",
        older,
        &fixture.key,
        index.as_bytes(),
        &[],
    );
    assert_eq!(read(older).unwrap(), COMMIT);
    // A Nightly whose index names another commit than its version: refused.
    let other = "nightly-2026-11-13-abcdef0";
    publish(
        &base,
        "nightly-2026-11-13",
        other,
        &fixture.key,
        index.as_bytes(),
        &[],
    );
    assert!(matches!(read(other), Err(Refusal::Refused { .. })));
    // A dated tag republished with another build of that day, signed for that build: the build
    // asked for is gone, not tampered with.
    let replaced = "nightly-2026-11-17-abcdef0";
    publish(
        &base,
        "nightly-2026-11-17",
        "nightly-2026-11-17-0123456",
        &fixture.key,
        index.as_bytes(),
        &[],
    );
    assert!(matches!(read(replaced), Err(Refusal::Superseded { .. })));
    // That build signed by a key Sunrise doesn't trust, or a partial upload missing its
    // signature or its sums, is a safety refusal.
    let republish = |key: &SigningKey| {
        publish(
            &base,
            "nightly-2026-11-17",
            "nightly-2026-11-17-0123456",
            key,
            index.as_bytes(),
            &[],
        )
    };
    republish(&SigningKey::generate().unwrap().0);
    assert!(matches!(read(replaced), Err(Refusal::Unsigned { .. })));
    for missing in [SIGNATURE_V2, SUMS] {
        republish(&fixture.key);
        fs::remove_file(base.join("nightly-2026-11-17").join(missing)).unwrap();
        assert!(matches!(read(replaced), Err(Refusal::Unsigned { .. })));
    }
    republish(&fixture.key);
    // An index that still names that build but isn't the one its signed sums list is no
    // replacement: it's a safety refusal.
    fs::write(
        base.join("nightly-2026-11-17").join(INDEX),
        format!("{index} "),
    )
    .unwrap();
    assert!(matches!(read(replaced), Err(Refusal::Unsigned { .. })));
    // Only the signature decides: an index naming another build of that day, signed for
    // anything else, is still a safety refusal.
    let unproven = "nightly-2026-11-18-abcdef0";
    publish(
        &base,
        "nightly-2026-11-18",
        "nightly-2026-11-19-0123456",
        &fixture.key,
        index.as_bytes(),
        &[],
    );
    assert!(matches!(read(unproven), Err(Refusal::Unsigned { .. })));
    // A Dev build: read at its own tag, signed for its exact version.
    let dev = "dev-2026-11-13-0123456";
    publish(&base, dev, dev, &fixture.key, index.as_bytes(), &[]);
    assert_eq!(read(dev).unwrap(), COMMIT);
    // A Dev build is never read as a Nightly: a Dev index and signature served under the
    // Nightly's tags, for a Nightly version of the same date and commit, are refused.
    let as_nightly = "nightly-2026-11-15-0123456";
    for tag in [
        sunrise_launcher_release_verify::NIGHTLY,
        "nightly-2026-11-15",
    ] {
        publish(
            &base,
            tag,
            &dev.replace("13", "15"),
            &fixture.key,
            index.as_bytes(),
            &[],
        );
    }
    assert!(matches!(read(as_nightly), Err(Refusal::Unsigned { .. })));
    // A version of neither channel's full form, its own tag holding a validly signed index:
    // no build id (a channel's tag never reads as a release).
    for version in [
        "dev-2026-11-16",
        "nightly-2026-11-16",
        "dev",
        "dev-2026-11-16-XYZ1234",
    ] {
        publish(&base, version, version, &fixture.key, index.as_bytes(), &[]);
        assert!(read(version).is_err(), "{version}");
        assert!(!kept.join(version).exists(), "{version}");
    }
    // A release, signed for its tag; one signed for another tag is refused.
    publish(
        &base,
        "v2099.01.01",
        "v2099.01.01",
        &fixture.key,
        index.as_bytes(),
        &[],
    );
    assert_eq!(read("v2099.01.01").unwrap(), COMMIT);
    publish(
        &base,
        "v2099.01.02",
        "v2099.01.01",
        &fixture.key,
        index.as_bytes(),
        &[],
    );
    assert!(matches!(read("v2099.01.02"), Err(Refusal::Unsigned { .. })));
    // A release with no index has no SDK; a version that is not a plain name is never a path.
    fs::create_dir_all(base.join("v2099.01.03")).unwrap();
    assert!(matches!(read("v2099.01.03"), Err(Refusal::NoSdk { .. })));
    assert!(matches!(
        read("../v2099.01.01"),
        Err(Refusal::Refused { .. })
    ));
}

#[test]
fn rust_versions_compare_by_number() {
    use std::cmp::Ordering::*;
    assert_eq!(compare_versions("1.95.0", "1.95.0"), Some(Equal));
    assert_eq!(compare_versions("1.100.0", "1.95.0"), Some(Greater));
    assert_eq!(compare_versions("1.9.0", "1.95.0"), Some(Less));
    assert_eq!(compare_versions("1.95.0-beta", "1.95.0"), None);
}

#[test]
fn sizes_and_builds_read_plainly() {
    assert_eq!(megabytes(71_000_000), "71 MB");
    assert_eq!(megabytes(1), "1 MB");
    // A build with no name here is never shown as its commit.
    assert_eq!(shown(COMMIT, None), "your Sunrise version");
    // A pin from a mod's manifest is cut only where a character ends.
    assert_eq!(shown("ééééé", None), "ééééé");
}

#[test]
fn a_refusal_offers_the_way_on_its_text_promises() {
    let build = || "nightly-2026-11-13".to_owned();
    let way = |refusal: Refusal| way_on(&format!("{refusal}"));
    assert_eq!(
        way(Refusal::NoSdk { build: build() }),
        Some(WayOn::OtherBuild)
    );
    assert_eq!(
        way(Refusal::Superseded { build: build() }),
        Some(WayOn::OtherBuild)
    );
    assert_eq!(
        way(Refusal::OlderRust {
            build: build(),
            rust: "1.80.0".into()
        }),
        Some(WayOn::OtherBuild)
    );
    assert_eq!(
        way(Refusal::Unsigned {
            build: build(),
            why: "checksum mismatch".into()
        }),
        Some(WayOn::Report)
    );
    for bytes in [None, Some(212_000_000)] {
        assert_eq!(
            way(Refusal::Offline {
                build: build(),
                bytes
            }),
            Some(WayOn::Retry)
        );
    }
    assert_eq!(way(Refusal::Host), None);
}

#[test]
fn a_developer_tool_is_honoured_by_a_debug_launcher_or_for_a_developer_base_only() {
    let _guard = super::super::tests::scratch_state();
    let program =
        std::env::temp_dir().join(format!("sunrise-developer-tool-{}", std::process::id()));
    fs::write(&program, "").unwrap();
    std::env::set_var(DEVELOPER_TOOL, &program);
    let any_mod = developer_tool(false);
    let developer_base = developer_tool(true);
    std::env::remove_var(DEVELOPER_TOOL);
    fs::remove_file(&program).unwrap();
    assert_eq!(any_mod.is_some(), cfg!(debug_assertions));
    assert_eq!(developer_base, Some(program));
}
