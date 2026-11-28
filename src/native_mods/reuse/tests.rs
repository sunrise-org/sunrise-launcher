use super::*;
use crate::native_mods::tests::{dir, scratch_state, write_bundle};
use std::process::Command;

/// The Sunrise commit the sets build: not the mods' own pin, whose SDK index other tests keep.
const REVISION: &str = "abcdefabcdefabcdefabcdefabcdefabcdefabcd";
/// The mods' own Sunrise pin (`write_bundle`): what a developer base must build.
const PINNED: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const C: &str = "cccccccccccccccccccccccccccccccccccccccc";
const BASE: &str = "native-aaaaaaa-base00000000";
const ORIGIN: &str = "26.3-0.1.0";

/// Keeps a signed SDK index for Sunrise `commit`, as a planned build leaves it; `date` makes
/// another index for the same build.
fn keep_index(commit: &str, date: &str) {
    let mut index = sdk_cache::tests::index_of(&[], rust_toolchain::VERSION);
    index.build.commit = commit.into();
    index.build.date = date.into();
    let folder = sdk_cache::build_folder(commit);
    fs::create_dir_all(&folder).unwrap();
    fs::write(
        folder.join(sunrise_launcher_sdk::INDEX),
        index.to_json().as_bytes(),
    )
    .unwrap();
}

/// A Windows game executable for this host's machine, so a base's platform reads as it does for a
/// release; any file elsewhere.
fn game_executable() -> Vec<u8> {
    let mut image = vec![0; 70];
    image[..2].copy_from_slice(b"MZ");
    image[0x3c..0x40].copy_from_slice(&64_u32.to_le_bytes());
    image[64..68].copy_from_slice(b"PE\0\0");
    let machine = match std::env::consts::ARCH {
        "aarch64" => rust_toolchain::WINDOWS_ARM64_MACHINE,
        _ => rust_toolchain::WINDOWS_X64_MACHINE,
    };
    image[68..70].copy_from_slice(&machine.to_le_bytes());
    image
}

/// An instance with two native mods, and the base its builds start from.
struct Set {
    root: PathBuf,
    folder: PathBuf,
    base: String,
    /// The native build the base is, if it is one.
    built_on: Option<About>,
    /// The Sunrise commit its builds compile.
    revision: &'static str,
    /// The release its builds start from.
    origin: String,
    platform: BuildPlatform,
    /// The SDK base setting its plans are admitted under.
    sdk_base: Option<String>,
    /// The developer's own prebuilt game its plans compile against, if it has one.
    local_sdk: Option<PathBuf>,
}

impl Set {
    /// Built on the native build [`BASE`], whose record names Sunrise [`REVISION`].
    fn new(name: &str) -> Self {
        keep_index(REVISION, "2026-11-13");
        Self::on_base(
            name,
            BASE,
            Some(About {
                base: ORIGIN.into(),
                sunrise: REVISION.into(),
                target: Target::Client,
                left_out: Vec::new(),
            }),
        )
    }

    /// Built on developer base `base`, a checkout of the mods' own pin ([`PINNED`]): a plan of it
    /// runs no SDK check.
    fn developer(name: &str, base: &str) -> Self {
        Self::on_base(name, base, None)
    }

    fn on_base(name: &str, base: &str, built_on: Option<About>) -> Self {
        let root = dir(name);
        let folder = root.join("instance").join("native");
        write_bundle(&folder.join("alpha"), "alpha", "pub fn alpha() {}\n");
        // Each test's set is its own build.
        write_bundle(
            &folder.join("beta"),
            "beta",
            &format!("pub fn beta() {{}}\n// {name}\n"),
        );
        Self {
            root,
            folder,
            base: base.to_owned(),
            revision: built_on.as_ref().map_or(PINNED, |_| REVISION),
            origin: built_on
                .as_ref()
                .map_or(base.to_owned(), |about| about.base.clone()),
            built_on,
            platform: BuildPlatform::default(),
            sdk_base: None,
            local_sdk: None,
        }
    }

    fn developer_base(&self) -> bool {
        crate::developer_install::is_developer(&self.origin)
    }

    /// The same set, built for `platform`.
    fn on(mut self, platform: BuildPlatform) -> Self {
        self.platform = platform;
        self
    }

    /// The same set, compiled against a developer's prebuilt game folder whose package file
    /// names it as `package`.
    fn against(&mut self, package: &str) {
        let folder = self.root.join("prebuilt");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join(PREBUILT_IDENTITY), package).unwrap();
        self.local_sdk = Some(folder);
    }

    /// The instance's overrides file.
    fn overrides(&self) -> PathBuf {
        let mods = self
            .root
            .join("instance")
            .join(sunrise_launcher_smod::mod_list::DIR);
        fs::create_dir_all(&mods).unwrap();
        mods.join(dependencies::OVERRIDES)
    }

    /// Fresh private copies of the mods, as `pipeline` makes them.
    fn copies(&self) -> Vec<Bundle> {
        let copies = dir(&format!(
            "{}-copies",
            self.root.file_name().unwrap().to_string_lossy()
        ));
        scan_bundles(&self.folder)
            .unwrap()
            .into_iter()
            .map(|bundle| {
                let to = copies.join(&bundle.manifest.id);
                copy_bundle(&bundle, &to).unwrap();
                Bundle { dir: to, ..bundle }
            })
            .collect()
    }

    fn request<'a>(&'a self, options: &'a BuildOptions<'a>) -> Request<'a> {
        Request {
            base: &self.base,
            folder: &self.folder,
            built_on: self.built_on.as_ref(),
            platform: self.platform,
            profile: Profile::Optimized,
            target: Target::Client,
            options,
            local_sdk: self.local_sdk.as_deref(),
        }
    }

    fn found_with(&self, options: &BuildOptions<'_>) -> Option<String> {
        lookup(&self.request(options), &self.copies()).map(|hit| hit.version)
    }

    fn found(&self) -> Option<String> {
        self.found_with(&BuildOptions::default())
    }

    /// Plans the set as `pipeline` does, without the SDK check, installs what it would build
    /// unless a build there holds the plan (`installable`), and keeps the evidence `pipeline`
    /// keeps for it. Returns the build's version.
    fn install(&self, options: &BuildOptions<'_>) -> String {
        let found = scan_bundles(&self.folder).unwrap();
        let digests: HashMap<String, Option<String>> = found
            .iter()
            .map(|b| (b.manifest.id.clone(), source_digest(b)))
            .collect();
        let requested = self.copies();
        let side = side_of(Target::Client);
        let (left_out, mods, declared) = kept_mods(&requested, options.skip, side);
        let instance = instance_of(&self.folder, options);
        let overrides =
            dependencies::Overrides::read(&instance.join(sunrise_launcher_smod::mod_list::DIR))
                .unwrap();
        let (dispatch, links) =
            resolve_build_dependencies(instance, &mods, &declared, side, &overrides).unwrap();
        let ordered: Vec<Bundle> = links.order.iter().map(|&i| mods[i].clone()).collect();
        let keys = sunrise_launcher_release_verify::sdk_keys_for(backend::GAME_REPO);
        let sdk_base = sdk_cache::Base::from_env();
        let developer = self.developer_base();
        let sdk = (!developer).then(|| sdk_cache::index(&sdk_base, self.revision, &keys).unwrap());
        let (pins, mut read) = mixins::read_pins(&mods, sdk.as_ref(), None, |pin| {
            Ok(sdk_cache::index(&sdk_base, pin, &keys)?)
        })
        .unwrap();
        if let Some(sdk) = &sdk {
            read.sdk
                .insert(self.revision.into(), sdk.signed().to_owned());
        }
        for (bundle, pin) in mods.iter().zip(&pins) {
            mixins::materialize(bundle, &bundle.dir, pin).unwrap();
        }
        let mixins = mixins::for_key(&pins, |_| panic!("one pin")).unwrap();
        let version = version_for(
            self.revision,
            &mixins,
            Profile::Optimized,
            Target::Client,
            platform_key(developer, self.platform, &self.origin).as_deref(),
            &requested,
            &left_out,
        )
        .unwrap();
        let contents = Contents {
            revision: self.revision,
            origin: &self.origin,
            mixins: &mixins,
            profile: Profile::Optimized,
            kept: &mods,
            digests: &digests,
            left_out: &left_out,
        };
        let chosen = Chosen::select(self.local_sdk.as_deref(), sdk, || {
            Ok(sdk_cache::index(&sdk_base, self.revision, &keys)?)
        })
        .unwrap();
        let image = Image::new(&mods, &pins, &ordered, &dispatch, &left_out, &read, &chosen);
        let (version, evidence) = installable(&version, &contents, &image).unwrap();
        let admission = Admission::with_sdk_base(
            &self.base,
            &self.origin,
            options.exact_home.is_some(),
            self.sdk_base.as_deref().map(std::ffi::OsStr::new),
        );
        match evidence {
            Some(evidence) => admit(evidence, admission),
            None => {
                self.publish(&version, &mixins, &left_out, &mods, &ordered, &digests);
                seal_published(&version, &contents, &image, admission);
            }
        }
        version
    }

    /// Installs what `pipeline` would for the set as `version`, laid out as
    /// `installation::install` lays it out.
    fn publish(
        &self,
        version: &str,
        mixins: &str,
        left_out: &[LeftOut],
        mods: &[Bundle],
        ordered: &[Bundle],
        digests: &HashMap<String, Option<String>>,
    ) {
        let installed = backend::version_dir(version);
        let _ = fs::remove_dir_all(&installed);
        fs::create_dir_all(&installed).unwrap();
        fs::write(installed.join(backend::CLIENT), game_executable()).unwrap();
        let record = serde_json::json!({
            "base": self.origin,
            "sunrise": self.revision,
            "horizon_mixins": mixins,
            "rust": rust_toolchain::VERSION,
            "profile": Profile::Optimized.name(),
            "route": "prebuilt-api",
            "left_out": left_out.iter().map(LeftOut::to_json).collect::<Vec<_>>(),
            "target": "client",
            "mods": mods.iter().map(|m| serde_json::json!({
                "id": m.manifest.id,
                "version": m.manifest.version,
                "sha256": digests[&m.manifest.id],
            })).collect::<Vec<_>>(),
        });
        fs::write(installed.join(RECORD), record.to_string()).unwrap();
        for bundle in ordered {
            copy_bundle(
                bundle,
                &installed
                    .join(sunrise_launcher_smod::mod_list::NATIVE_DIR)
                    .join(&bundle.manifest.id),
            )
            .unwrap();
        }
        fs::write(installed.join(".installed"), "").unwrap();
    }

    fn evidence(&self, version: &str) -> PathBuf {
        backend::version_dir(version).join(EVIDENCE)
    }

    /// Rewrites `version`'s evidence through `edit`, as JSON.
    fn edit_evidence(&self, version: &str, edit: impl FnOnce(&mut serde_json::Value)) {
        let path = self.evidence(version);
        let mut json: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        edit(&mut json);
        fs::write(path, json.to_string()).unwrap();
    }

    /// Turns installed build `version` of the whole set into what a launcher before this one
    /// installed under the same version for a build without `left`: no evidence, a record of the
    /// set without it, and no copy of it.
    fn make_reduced(&self, version: &str, left: &LeftOut) {
        let installed = backend::version_dir(version);
        let _ = fs::remove_file(self.evidence(version));
        let mut record: serde_json::Value =
            serde_json::from_slice(&fs::read(installed.join(RECORD)).unwrap()).unwrap();
        record["left_out"] = serde_json::json!([left.to_json()]);
        record["mods"]
            .as_array_mut()
            .unwrap()
            .retain(|m| m["id"] != left.id.as_str());
        fs::write(installed.join(RECORD), record.to_string()).unwrap();
        fs::remove_dir_all(
            installed
                .join(sunrise_launcher_smod::mod_list::NATIVE_DIR)
                .join(&left.id),
        )
        .unwrap();
    }

    /// The left-out entry for `id`, as a build that failed to compile it keeps it.
    fn left(&self, id: &str) -> LeftOut {
        let bundle = scan_bundles(&self.folder)
            .unwrap()
            .into_iter()
            .find(|b| b.manifest.id == id)
            .unwrap();
        LeftOut::new(&bundle, isolate::Why::DidNotBuild)
    }
}

impl Drop for Set {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
        if self.built_on.is_some() {
            let _ = fs::remove_dir_all(sdk_cache::build_folder(self.revision));
        }
    }
}

/// A set planned and installed once is found again from a copy of its mods, and used without
/// the SDK's check or any other step but saying it is up to date.
#[test]
fn an_installed_set_is_found_from_a_copy_of_its_mods_without_checking_them() {
    let _guard = scratch_state();
    let set = Set::new("reuse-hit");
    let version = set.install(&BuildOptions::default());
    assert!(
        set.evidence(&version).is_file(),
        "the plan keeps its evidence"
    );
    let checks = SDK_CHECKS.with(std::cell::Cell::get);
    let options = BuildOptions::default();
    let hit = lookup(&set.request(&options), &set.copies()).expect("the same set is found");
    assert_eq!(hit.version, version);
    let steps = std::cell::RefCell::new(Vec::new());
    let built = reused(
        hit,
        Profile::Optimized,
        Target::Client,
        &options,
        &|step| steps.borrow_mut().push(step),
        std::time::Instant::now(),
    )
    .unwrap();
    assert_eq!(
        (built.version.as_str(), built.plan),
        (version.as_str(), Plan::UpToDate)
    );
    assert!(built.left_out.is_empty());
    assert_eq!(
        steps.into_inner(),
        [Step::Plan(Plan::UpToDate, Profile::Optimized)]
    );
    assert_eq!(
        SDK_CHECKS.with(std::cell::Cell::get),
        checks,
        "the SDK tool checked nothing"
    );
    let _ = fs::remove_dir_all(backend::version_dir(&version));
}

/// Each input that changes the build without being in its version misses, and the same input
/// again finds the build again: a mod's file, a horizon-mixins pin, the SDK index read for the
/// build, the dispatch order the overrides allow, the platform and the target.
#[test]
fn an_input_that_changes_the_build_misses_it() {
    let _guard = scratch_state();
    let set = Set::new("reuse-image");
    let version = set.install(&BuildOptions::default());
    assert_eq!(set.found().as_deref(), Some(version.as_str()));

    let source = set.folder.join("alpha").join("src/lib.rs");
    fs::write(&source, "pub fn alpha() { changed() }\n").unwrap();
    assert!(set.found().is_none(), "a changed mod file");
    fs::write(&source, "pub fn alpha() {}\n").unwrap();
    assert_eq!(set.found().as_deref(), Some(version.as_str()));

    let manifest = set.folder.join("beta").join(NATIVE_MANIFEST);
    let pinned = fs::read_to_string(&manifest).unwrap();
    let repinned = pinned.replace(&format!("mixins='{B}'"), &format!("mixins='{C}'"));
    assert_ne!(repinned, pinned, "the fixture pins horizon-mixins");
    fs::write(&manifest, repinned).unwrap();
    assert!(set.found().is_none(), "another horizon-mixins pin");
    fs::write(&manifest, &pinned).unwrap();
    assert_eq!(set.found().as_deref(), Some(version.as_str()));

    keep_index(REVISION, "2026-11-14");
    assert!(
        set.found().is_none(),
        "another SDK index for the build, with the same pins"
    );
    fs::remove_dir_all(sdk_cache::build_folder(REVISION)).unwrap();
    assert!(
        set.found().is_none(),
        "no SDK index kept: nothing is fetched"
    );
    keep_index(REVISION, "2026-11-13");
    assert_eq!(set.found().as_deref(), Some(version.as_str()));

    let windows = Set::new("reuse-image-windows").on(BuildPlatform {
        target: Some(rust_toolchain::WINDOWS_X64_TARGET),
        machine: None,
    });
    assert!(windows.found().is_none(), "another platform key");
    let options = BuildOptions::default();
    let server = Request {
        target: Target::Server,
        ..set.request(&options)
    };
    assert!(lookup(&server, &set.copies()).is_none(), "a server");
    let _ = fs::remove_dir_all(backend::version_dir(&version));
}

/// The dispatch order is not in the version, but the build was planned with it: an override
/// that changes it misses the build, though every file is the same.
#[test]
fn another_dispatch_order_misses_the_build() {
    let _guard = scratch_state();
    let set = Set::new("reuse-dispatch");
    let manifest = set.folder.join("beta").join(NATIVE_MANIFEST);
    let text = fs::read_to_string(&manifest).unwrap();
    fs::write(
        &manifest,
        text.replace("[native]", "load_before=['alpha']\n[native]"),
    )
    .unwrap();
    let version = set.install(&BuildOptions::default());
    assert_eq!(set.found().as_deref(), Some(version.as_str()));
    fs::write(set.overrides(), "[beta]\nalpha='drop'\n").unwrap();
    assert!(
        set.found().is_none(),
        "the override changes the dispatch order"
    );
    fs::remove_file(set.overrides()).unwrap();
    assert_eq!(set.found().as_deref(), Some(version.as_str()));
    let _ = fs::remove_dir_all(backend::version_dir(&version));
}

/// What admitted the plan without changing the build is kept apart from what the build used:
/// another launcher, another base or an exact profile build misses, and once that caller has
/// planned the set too, both find it.
#[test]
fn a_rule_that_only_admits_the_plan_misses_until_it_admits_it() {
    let _guard = scratch_state();
    let set = Set::new("reuse-admission");
    let version = set.install(&BuildOptions::default());
    let launcher = |json: &mut serde_json::Value| json["admitted"][0]["launcher"] = "0.0.1".into();
    set.edit_evidence(&version, launcher);
    assert!(set.found().is_none(), "another launcher's checks");
    assert_eq!(set.install(&BuildOptions::default()), version);
    assert_eq!(set.found().as_deref(), Some(version.as_str()));

    let instance = set.root.join("instance");
    let exact = BuildOptions {
        exact_home: Some(&instance),
        ..BuildOptions::default()
    };
    assert!(set.found_with(&exact).is_none(), "an exact profile build");
    assert_eq!(set.install(&exact), version, "the same build");
    assert_eq!(set.found_with(&exact).as_deref(), Some(version.as_str()));
    assert_eq!(set.found().as_deref(), Some(version.as_str()));

    let options = BuildOptions::default();
    let other = Request {
        base: "native-aaaaaaa-other0000000",
        ..set.request(&options)
    };
    assert!(lookup(&other, &set.copies()).is_none(), "another base");
    let _ = fs::remove_dir_all(backend::version_dir(&version));
}

/// A build without a mod is another version than the whole set's, so it never stands in for
/// it. A reduced build an older launcher installed under the whole set's version is never used
/// for the whole set either: the plan installs under the next version, and leaves the old build
/// as it is.
#[test]
fn a_reduced_build_never_satisfies_the_whole_set() {
    let _guard = scratch_state();
    let set = Set::new("reuse-reduced");
    let skip = [set.left("beta")];
    let reduced = BuildOptions {
        skip: &skip,
        ..BuildOptions::default()
    };
    let without = set.install(&reduced);
    assert_eq!(set.found_with(&reduced).as_deref(), Some(without.as_str()));
    assert!(
        set.found().is_none(),
        "the whole set isn't the reduced build"
    );
    let whole = set.install(&BuildOptions::default());
    assert_ne!(whole, without, "the two builds have their own versions");
    assert_eq!(set.found().as_deref(), Some(whole.as_str()));
    assert_eq!(set.found_with(&reduced).as_deref(), Some(without.as_str()));

    // An older launcher's reduced build, installed under the whole set's version.
    set.make_reduced(&whole, &skip[0]);
    let record = fs::read(backend::version_dir(&whole).join(RECORD)).unwrap();
    assert!(set.found().is_none(), "the old build isn't the whole set");
    let replaced = set.install(&BuildOptions::default());
    assert_ne!(
        replaced, whole,
        "the whole set is built under the next version"
    );
    assert_eq!(set.found().as_deref(), Some(replaced.as_str()));
    assert_eq!(
        fs::read(backend::version_dir(&whole).join(RECORD)).unwrap(),
        record,
        "the old build is left as it is"
    );
    assert!(!set.evidence(&whole).exists(), "and never sealed");
    assert_eq!(
        set.install(&BuildOptions::default()),
        replaced,
        "the same next version every time"
    );
    for version in [whole, without, replaced] {
        let _ = fs::remove_dir_all(backend::version_dir(&version));
    }
}

/// No evidence, or the record of the launcher before this one (`.native-inputs`), finds
/// nothing, and the build is never sealed afterwards: the plan installs under the next version
/// and leaves the old one as it is. Evidence that can't be read (torn, another layout, an
/// unknown field) is the same.
#[test]
fn missing_torn_or_older_evidence_plans_the_build() {
    let _guard = scratch_state();
    let set = Set::new("reuse-evidence");
    let version = set.install(&BuildOptions::default());
    let path = set.evidence(&version);
    fs::remove_file(&path).unwrap();
    let older = backend::version_dir(&version).join(".native-inputs");
    fs::write(&older, "0123\n").unwrap();
    assert!(set.found().is_none(), "no evidence");
    let next = set.install(&BuildOptions::default());
    assert_ne!(next, version, "built under the next version");
    assert!(!path.exists(), "the unproved build isn't sealed");
    assert_eq!(set.found().as_deref(), Some(next.as_str()));
    for version in [version, next] {
        let _ = fs::remove_dir_all(backend::version_dir(&version));
    }

    let torn = |text: &[u8]| text[..text.len() / 2].to_vec();
    let schema = |text: &[u8]| {
        let mut json: serde_json::Value = serde_json::from_slice(text).unwrap();
        json["schema"] = 0.into();
        json.to_string().into_bytes()
    };
    let extra = |text: &[u8]| {
        let mut json: serde_json::Value = serde_json::from_slice(text).unwrap();
        json["extra"] = true.into();
        json.to_string().into_bytes()
    };
    type Change = dyn Fn(&[u8]) -> Vec<u8>;
    let cases: [(&str, &Change); 3] = [
        ("torn", &torn),
        ("another layout", &schema),
        ("an unknown field", &extra),
    ];
    for (n, (case, change)) in cases.into_iter().enumerate() {
        let set = Set::new(&format!("reuse-evidence-{n}"));
        let version = set.install(&BuildOptions::default());
        let path = set.evidence(&version);
        let changed = change(&fs::read(&path).unwrap());
        fs::write(&path, &changed).unwrap();
        assert!(set.found().is_none(), "{case}");
        let next = set.install(&BuildOptions::default());
        assert_ne!(next, version, "{case}: built under the next version");
        assert_eq!(
            fs::read(&path).unwrap(),
            changed,
            "{case}: not sealed again"
        );
        assert_eq!(set.found().as_deref(), Some(next.as_str()), "{case}");
        for version in [version, next] {
            let _ = fs::remove_dir_all(backend::version_dir(&version));
        }
    }
}

/// The installed build must still be the one sealed and planned: a changed game, a record that
/// names other mods or another pin, a missing mod copy, or no completion marker misses. A plan
/// never seals a game changed after it was sealed: it builds under the next version, and the
/// changed game is never found again.
#[test]
fn an_installed_build_that_changed_misses() {
    let _guard = scratch_state();
    let set = Set::new("reuse-seal");
    let version = set.install(&BuildOptions::default());
    let installed = backend::version_dir(&version);
    let game = installed.join(backend::CLIENT);
    fs::write(&game, b"another game").unwrap();
    assert!(set.found().is_none(), "another executable");
    fs::write(&game, game_executable()).unwrap();
    assert_eq!(set.found().as_deref(), Some(version.as_str()));

    let library = installed.join(format!(
        "{}extra{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    fs::write(&library, b"library").unwrap();
    assert!(set.found().is_none(), "a library beside the game");
    fs::remove_file(&library).unwrap();

    let record = installed.join(RECORD);
    let planned = fs::read_to_string(&record).unwrap();
    fs::write(&record, planned.replace(B, C)).unwrap();
    assert!(
        set.found().is_none(),
        "a record with another horizon-mixins key"
    );
    fs::write(&record, &planned).unwrap();
    assert_eq!(set.found().as_deref(), Some(version.as_str()));

    let copy = installed
        .join(sunrise_launcher_smod::mod_list::NATIVE_DIR)
        .join("beta");
    let moved = installed.join("beta-copy");
    fs::rename(&copy, &moved).unwrap();
    assert!(set.found().is_none(), "a missing mod copy");
    fs::rename(&moved, &copy).unwrap();
    assert_eq!(set.found().as_deref(), Some(version.as_str()));

    // The game changes after it was sealed, and the set is planned again.
    let evidence = fs::read(set.evidence(&version)).unwrap();
    fs::write(&game, b"another game").unwrap();
    let next = set.install(&BuildOptions::default());
    assert_ne!(next, version, "built under the next version");
    assert_eq!(
        fs::read(set.evidence(&version)).unwrap(),
        evidence,
        "the changed game isn't sealed"
    );
    assert_eq!(set.found().as_deref(), Some(next.as_str()));

    fs::remove_file(installed.join(".installed")).unwrap();
    fs::remove_file(backend::version_dir(&next).join(".installed")).unwrap();
    assert!(set.found().is_none(), "a build that isn't installed");
    for version in [version, next] {
        let _ = fs::remove_dir_all(backend::version_dir(&version));
    }
}

/// Each caller that planned a build is kept once, newest last, up to [`ADMISSIONS_KEPT`].
#[test]
fn the_evidence_keeps_the_newest_admissions_once() {
    let _guard = scratch_state();
    let set = Set::new("reuse-admissions");
    let version = set.install(&BuildOptions::default());
    let first = serde_json::json!(Admission::with_sdk_base(BASE, ORIGIN, false, None));
    set.edit_evidence(&version, |json| {
        let others: Vec<serde_json::Value> = (0..10)
            .map(|n| {
                let mut other = first.clone();
                other["base"] = format!("native-aaaaaaa-other{n:07}").into();
                other
            })
            .collect();
        json["admitted"] = others.into();
    });
    set.install(&BuildOptions::default());
    set.install(&BuildOptions::default());
    let evidence: serde_json::Value =
        serde_json::from_slice(&fs::read(set.evidence(&version)).unwrap()).unwrap();
    let admitted = evidence["admitted"].as_array().unwrap();
    assert_eq!(admitted.len(), ADMISSIONS_KEPT);
    assert_eq!(admitted.last(), Some(&first));
    assert_eq!(admitted.iter().filter(|a| **a == first).count(), 1);
    assert_eq!(admitted[0]["base"], "native-aaaaaaa-other0000003");
    let _ = fs::remove_dir_all(backend::version_dir(&version));
}

/// The SDK base setting can hold a mirror's credentials: the evidence keeps a fingerprint of
/// it, which tells another setting apart, and never the setting.
#[test]
fn the_evidence_never_holds_the_sdk_base_setting() {
    let _guard = scratch_state();
    let mut set = Set::new("reuse-sdk-base");
    let setting = "https://reader:s3cret-token@mirror.example/sdk";
    set.sdk_base = Some(setting.into());
    let version = set.install(&BuildOptions::default());
    let text = fs::read_to_string(set.evidence(&version)).unwrap();
    for part in [setting, "s3cret-token", "mirror.example", "reader"] {
        assert!(!text.contains(part), "{part} in {text}");
    }
    let admission = |setting: Option<&str>| {
        Admission::with_sdk_base(BASE, ORIGIN, false, setting.map(std::ffi::OsStr::new))
    };
    assert_eq!(admission(Some(setting)), admission(Some(setting)));
    assert_ne!(
        admission(Some(setting)),
        admission(Some("https://reader:other@mirror.example/sdk"))
    );
    assert_ne!(admission(Some(setting)), admission(None));
    let _ = fs::remove_dir_all(backend::version_dir(&version));
}

/// Runs `test` again in a child process with its own launcher and game folders.
fn in_child(test: &str, child: &str) -> bool {
    if std::env::var_os(child).is_some() {
        return true;
    }
    let home = tempfile::tempdir().unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test, "--include-ignored", "--nocapture"])
        .env(child, "1")
        .env("SUNRISE_LAUNCHER_HOME", home.path().join("launcher"))
        .env("SUNRISE_HOME", home.path().join("home"))
        .env(
            "SUNRISE_LAUNCHER_LOCATION_FILE",
            home.path().join("location"),
        )
        .env("SUNRISE_NO_KEYCHAIN", "1")
        .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
        .env("XDG_DATA_HOME", home.path().join("data"))
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env_remove(sdk_cache::BASE_VAR)
        .env_remove(sdk_cache::DEVELOPER_SDK)
        .env_remove(sdk_cache::DEVELOPER_TOOL)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("1 passed; 0 failed"),
        "child did not run {test}: {stdout}"
    );
    false
}

/// Installs, prepared, release or developer base `tag`, and the instance `sunrise`.
fn install_base(tag: &str, files: &[(&str, String)]) -> Instance {
    let dir = backend::version_dir(tag);
    fs::create_dir_all(backend::resources(tag)).unwrap();
    fs::write(backend::resources(tag).join("sunrise-resources.json"), "{}").unwrap();
    for (name, text) in files {
        fs::write(dir.join(name), text).unwrap();
    }
    fs::write(dir.join(backend::CLIENT), game_executable()).unwrap();
    fs::write(dir.join(".installed"), "").unwrap();
    fs::write(
        backend::data_dir().join("instances.json"),
        r#"{"instances":{"sunrise":{}}}"#,
    )
    .unwrap();
    instances::find("sunrise").unwrap()
}

/// `set` built through the default entry for `instance` on `base`: the result, how many times
/// the SDK tool was asked to check mods, and the steps said.
fn build_default(
    set: &Set,
    instance: &Instance,
) -> (Result<ClientBuild, Failed>, usize, Vec<Step>) {
    let steps = std::cell::RefCell::new(Vec::new());
    let checks = SDK_CHECKS.with(std::cell::Cell::get);
    let built = build(
        &set.base,
        &set.folder,
        instance,
        None,
        false,
        &Cancellation::default(),
        |step| steps.borrow_mut().push(step),
        || None,
    );
    (
        built,
        SDK_CHECKS.with(std::cell::Cell::get) - checks,
        steps.into_inner(),
    )
}

/// Through the default build entry (the Mods page's and the CLI's, with no profile chosen), an
/// installed set is used before the mods are checked to choose the profile: the SDK tool checks
/// nothing. Without evidence, both that check and the build's own run it.
#[test]
fn the_default_build_uses_an_installed_set_without_the_sdk_check() {
    if !in_child(
        "native_mods::reuse::tests::the_default_build_uses_an_installed_set_without_the_sdk_check",
        "SUNRISE_REUSE_DEFAULT_BUILD_CHILD",
    ) {
        return;
    }
    // The release the native base was made from, and the native base on it.
    install_base(ORIGIN, &[]);
    let record =
        format!(r#"{{"base":"{ORIGIN}","sunrise":"{REVISION}","target":"client","left_out":[]}}"#);
    let instance = install_base(BASE, &[(RECORD, record)]);
    let set = Set::new("reuse-default-build").on(platform_for_game(BASE).unwrap());

    let version = set.install(&BuildOptions::default());
    let (built, checks, steps) = build_default(&set, &instance);
    assert!(
        matches!(&built, Ok(ClientBuild::Native(found)) if *found == version),
        "{built:?}"
    );
    assert_eq!(checks, 0, "the SDK tool checked nothing");
    assert_eq!(steps, [Step::Plan(Plan::UpToDate, Profile::Optimized)]);
    assert_eq!(
        instances::find("sunrise").unwrap().version.as_deref(),
        Some(version.as_str()),
        "the instance plays the build"
    );

    // Without evidence, the profile check and the build both ask the SDK tool; this SDK has none.
    fs::remove_file(set.evidence(&version)).unwrap();
    let (built, checks, _) = build_default(&set, &instance);
    assert!(built.is_err(), "the fixture's SDK has no tool");
    assert_eq!(checks, 2, "the profile's check and the build's");
}

/// An override changes the image without changing the version. Even after the fast path
/// misses, the default build must use the next slot rather than re-admit the old image.
#[test]
fn the_default_build_uses_the_slot_with_the_planned_image() {
    if !in_child(
        "native_mods::reuse::tests::the_default_build_uses_the_slot_with_the_planned_image",
        "SUNRISE_REUSE_PLANNED_IMAGE_CHILD",
    ) {
        return;
    }
    const DEVELOPER: &str = "developer-reuse";
    let developer = format!(r#"{{"revision":"{PINNED}"}}"#);
    let instance = install_base(DEVELOPER, &[("developer.json", developer)]);
    keep_index(PINNED, "2026-11-13");
    let set =
        Set::developer("reuse-planned-image", DEVELOPER).on(platform_for_game(DEVELOPER).unwrap());
    let manifest = set.folder.join("beta").join(NATIVE_MANIFEST);
    let text = fs::read_to_string(&manifest).unwrap();
    fs::write(
        &manifest,
        text.replace("[native]", "load_before=['alpha']\n[native]"),
    )
    .unwrap();
    let version = set.install(&BuildOptions::default());
    assert_eq!(set.found().as_deref(), Some(version.as_str()));
    let evidence = fs::read(set.evidence(&version)).unwrap();
    fs::write(set.overrides(), "[beta]\nalpha='drop'\n").unwrap();
    assert!(set.found().is_none(), "the dispatch order changed");

    // Seed the new image in the next slot. Keep the original seal valid for the real build.
    let game = backend::version_dir(&version).join(backend::CLIENT);
    fs::remove_file(&game).unwrap();
    let next = set.install(&BuildOptions::default());
    assert_eq!(versions(&version).nth(1).as_deref(), Some(next.as_str()));
    fs::write(&game, game_executable()).unwrap();
    // The next slot matches, but this caller still has to plan before it can use it.
    set.edit_evidence(&next, |json| {
        json["admitted"][0]["launcher"] = "0.0.1".into()
    });
    assert!(set.found().is_none(), "the next slot needs admission");

    let (built, _, steps) = build_default(&set, &instance);
    assert!(
        matches!(&built, Ok(ClientBuild::Native(found)) if *found == next),
        "{built:?}"
    );
    assert_eq!(steps, [Step::Plan(Plan::UpToDate, Profile::Optimized)]);
    assert_eq!(
        instances::find("sunrise").unwrap().version.as_deref(),
        Some(next.as_str())
    );
    assert_eq!(
        fs::read(set.evidence(&version)).unwrap(),
        evidence,
        "the original image's evidence is unchanged"
    );
    assert_eq!(
        set.found().as_deref(),
        Some(next.as_str()),
        "planning admitted the matching image"
    );
}

#[test]
fn switching_mixins_source_mode_selects_a_different_image() {
    if !in_child(
        "native_mods::reuse::tests::switching_mixins_source_mode_selects_a_different_image",
        "SUNRISE_REUSE_MIXINS_MODE_CHILD",
    ) {
        return;
    }
    let set = Set::new("reuse-mode-switch");
    let options = BuildOptions::default();
    let release = set.install(&options);
    assert_eq!(set.found().as_deref(), Some(release.as_str()));

    mixins_from_source(true);
    assert!(
        set.found().is_none(),
        "an ordinary image cannot prove a source build"
    );
    let source = set.install(&options);
    assert_ne!(release, source);
    assert_eq!(set.found().as_deref(), Some(source.as_str()));

    mixins_from_source(false);
    assert_eq!(set.found().as_deref(), Some(release.as_str()));
    fs::remove_dir_all(backend::version_dir(release.as_str())).unwrap();
    assert!(
        set.found().is_none(),
        "a source image cannot prove an ordinary build"
    );
}

#[test]
fn a_missing_developer_manifest_does_not_expose_its_path() {
    if !in_child(
        "native_mods::reuse::tests::a_missing_developer_manifest_does_not_expose_its_path",
        "SUNRISE_REUSE_MANIFEST_ERROR_CHILD",
    ) {
        return;
    }
    for (shape, private_component) in [
        ("home/private-unix-account/checkout", "private-unix-account"),
        (
            r"Users\private-windows-account\checkout",
            "private-windows-account",
        ),
    ] {
        let scratch = tempfile::tempdir().unwrap();
        let folder = scratch.path().join(shape);
        let failed = logged(&Cancellation::default(), &|| {}, |_| {
            Chosen::select(Some(&folder), None, || unreachable!())
        })
        .unwrap_err();
        let log = fs::read_to_string(super::super::build_log::log_path()).unwrap();
        for text in [failed.to_string(), log] {
            assert!(text.contains(PREBUILT_IDENTITY), "{text}");
            assert!(
                text.contains(&std::io::ErrorKind::NotFound.to_string()),
                "{text}"
            );
            assert!(!text.contains(private_component), "{text}");
        }
    }
}

#[test]
fn occupied_unproved_slots_refuse_with_a_fixed_build_reason() {
    if !in_child(
        "native_mods::reuse::tests::occupied_unproved_slots_refuse_with_a_fixed_build_reason",
        "SUNRISE_REUSE_EXHAUSTED_CHILD",
    ) {
        return;
    }
    let set = Set::new("reuse-exhausted");
    let options = BuildOptions::default();
    let plan = derive(&set.request(&options), &set.copies()).unwrap();
    let digests = scan_bundles(&set.folder)
        .unwrap()
        .iter()
        .map(|bundle| (bundle.manifest.id.clone(), source_digest(bundle)))
        .collect();
    let contents = Contents {
        revision: &plan.revision,
        origin: &plan.origin,
        mixins: &plan.mixins,
        profile: Profile::Optimized,
        kept: &plan.mods,
        digests: &digests,
        left_out: &plan.left_out,
    };
    let paths: Vec<_> = versions(&plan.version)
        .map(|version| Target::Client.directory(&version))
        .collect();
    assert_eq!(paths.len(), 8);
    for folder in &paths {
        fs::create_dir_all(folder).unwrap();
    }
    crate::launcher_log::init(&set.root);
    let (before, _) = crate::launcher_log::since(0);
    assert!(installable(&plan.version, &contents, &plan.image).is_err());
    let (_, lines) = crate::launcher_log::since(before);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].ends_with("Native build refused: no vacant or proved installed image slot"));
    for folder in paths {
        assert!(fs::read_dir(folder).unwrap().next().is_none());
    }
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "daily; build the launcher binary first"]
fn verified_native_publication_seals_once_and_reuses_without_the_tool() {
    use std::os::unix::fs::PermissionsExt;
    if !in_child(
        "native_mods::reuse::tests::verified_native_publication_seals_once_and_reuses_without_the_tool",
        "SUNRISE_REUSE_FIRST_SEAL_CHILD",
    ) {
        return;
    }
    const DEVELOPER: &str = "developer-first-seal";
    let developer = format!(r#"{{"revision":"{PINNED}"}}"#);
    let instance = install_base(DEVELOPER, &[("developer.json", developer)]);
    let mut set = Set::developer("reuse-first-seal", DEVELOPER);
    set.against("fixture package");
    let sdk = set.local_sdk.as_deref().unwrap();
    let runtime = sdk.join("runtime/bin");
    fs::create_dir_all(&runtime).unwrap();
    fs::write(runtime.join(backend::CLIENT), game_executable()).unwrap();
    for id in ["alpha", "beta"] {
        fs::write(set.folder.join(id).join("Cargo.lock"), b"version = 4\n").unwrap();
    }
    // Use local fixture inputs throughout: neither a Rust download nor a system linker probe
    // should be needed to exercise launcher publication of the SDK's verified outputs.
    let host = rust_toolchain::host().unwrap();
    let rust = rust_toolchain::root().join(format!("{}-{}", rust_toolchain::VERSION, host.target));
    fs::create_dir_all(&rust).unwrap();
    fs::write(rust.join(".installed"), b"").unwrap();
    for name in ["cc", "pkg-config"] {
        let program = set.root.join(name);
        fs::write(&program, b"#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(
        [set.root.clone()]
            .into_iter()
            .chain(std::env::split_paths(&path)),
    )
    .unwrap();
    let tool = set.root.join("developer-tool");
    fs::write(&tool, r#"#!/usr/bin/env python3
import hashlib
import json
from pathlib import Path
import sys

here = Path(__file__).parent
with (here / 'tool-calls').open('a') as log:
    print(sys.argv[1], file=log)
plan = json.loads(input())
if sys.argv[1] == 'native-layout':
    sdk = Path(plan['prebuilt'])
    print(json.dumps({'package': {name: str(sdk / name) for name in ('runtime', 'api', 'engine')}}))
    sys.exit(0)
assert sys.argv[1] == 'native-build'
work = Path(plan['work'])
work.mkdir(parents=True, exist_ok=True)
records = []
libraries = []
for mod in ('alpha', 'beta'):
    name = f'lib{mod}.so'
    lib = work / name
    lib.write_bytes(mod.encode())
    libraries.append(str(lib))
    records.append({'file': name, 'sha256': hashlib.sha256(mod.encode()).hexdigest(),
                    'kind': 'mod', 'mod': mod,
                    'identity': f'{(here / "revision").read_text()} {mod} 0.1.0 {(here / "rust-version").read_text()} unwind'})
print(json.dumps({'event': 'built', 'route': 'prebuilt-api',
                  'binary': str(Path(plan['package']['runtime']) / 'bin' / (here / 'client-name').read_text()),
                  'libraries': libraries, 'records': records, 'mods': ['alpha', 'beta'], 'internals': {}}))
"#).unwrap();
    fs::set_permissions(&tool, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(set.root.join("revision"), PINNED).unwrap();
    fs::write(set.root.join("rust-version"), rust_toolchain::VERSION).unwrap();
    fs::write(set.root.join("client-name"), backend::CLIENT).unwrap();
    std::env::set_var("PATH", path);
    std::env::set_var(sdk_cache::DEVELOPER_SDK, sdk);
    std::env::set_var(sdk_cache::DEVELOPER_TOOL, &tool);

    let options = BuildOptions::default();
    let derived = derive(&set.request(&options), &set.copies()).unwrap();
    let version = derived.version;
    let dest = Target::Client.directory(&version);
    let failed = build(
        DEVELOPER,
        &set.folder,
        &instance,
        Some(Profile::Optimized),
        false,
        &Cancellation::default(),
        |step| {
            if step == Step::Install {
                fs::create_dir(&dest).unwrap();
                fs::write(dest.join("occupied"), b"keep").unwrap();
            }
        },
        || None,
    );
    assert!(
        failed.is_err(),
        "an occupied publication destination was accepted"
    );
    assert_eq!(fs::read(dest.join("occupied")).unwrap(), b"keep");
    assert!(!set.evidence(&version).exists());
    assert!(set.found().is_none());
    fs::remove_dir_all(&dest).unwrap();

    let (built, _, steps) = build_default(&set, &instance);
    assert!(
        matches!(&built, Ok(ClientBuild::Native(found)) if found == &version),
        "{built:?}"
    );
    assert!(steps.contains(&Step::Install));
    assert!(
        Evidence::read(&dest).unwrap().is_some(),
        "publication did not write its first seal"
    );
    let calls = set.root.join("tool-calls");
    let before = fs::read_to_string(&calls).unwrap();
    assert_eq!(
        before
            .lines()
            .filter(|call| *call == "native-build")
            .count(),
        2
    );
    let (built, checks, steps) = build_default(&set, &instance);
    assert!(
        matches!(&built, Ok(ClientBuild::Native(found)) if found == &version),
        "{built:?}"
    );
    assert_eq!(checks, 0);
    assert_eq!(steps, [Step::Plan(Plan::UpToDate, Profile::Optimized)]);
    assert_eq!(fs::read_to_string(calls).unwrap(), before);
}

/// Re-enabling a mod whose set's sealed exact build is still kept on disk clears `pending`
/// through Play's own `reuse::lookup`; a never-built set stays pending; removing the kept
/// build in Storage invalidates `fingerprint` and brings `pending` back.
#[test]
fn pending_label_uses_reuse_lookup_across_reenable_never_built_and_storage_clear() {
    if !in_child(
        "native_mods::reuse::tests::pending_label_uses_reuse_lookup_across_reenable_never_built_and_storage_clear",
        "SUNRISE_REUSE_PENDING_LABEL_CHILD",
    ) {
        return;
    }
    const DEVELOPER: &str = "developer-pending-reuse";
    let developer = format!(r#"{{"revision":"{PINNED}"}}"#);
    install_base(DEVELOPER, &[("developer.json", developer)]);
    keep_index(PINNED, "2026-11-13");
    let home = backend::game_home();
    let mods_dir = home.join(INSTALLED);
    fs::create_dir_all(&mods_dir).unwrap();
    let pack = |id: &str, body: &str| {
        let manifest = format!(
            "id='{id}'\nname='{id}'\nversion='0.1.0'\napi='{}'\ncapabilities=['registries']\n[native]\ngame='26.3'\nsunrise='{PINNED}'\nmixins='{B}'\n",
            sunrise_launcher_smod::manifest::API,
        );
        let bytes = sunrise_launcher_smod::manifest::Package::write_files(
            &manifest,
            &[
                (
                    "Cargo.toml",
                    b"[package]\nname='example'\nversion='9.0.0'\n[dependencies]\nsunrise-client='0.1'\n"
                        .as_slice(),
                ),
                ("src/lib.rs", body.as_bytes()),
            ],
        )
        .unwrap();
        fs::write(mods_dir.join(format!("{id}.smod")), bytes).unwrap();
    };
    pack("alpha", "pub fn alpha() {}\n");
    pack("beta", "pub fn beta() {}\n");
    pack("gamma", "pub fn gamma() {}\n");

    let platform = platform_for_game(DEVELOPER).unwrap();
    let seal_enabled = |name: &str| {
        let root = dir(name);
        let folder = stage_installed(&home, root.join("staged"), None)
            .unwrap()
            .unwrap();
        let set = Set {
            root,
            folder,
            base: DEVELOPER.into(),
            built_on: None,
            revision: PINNED,
            origin: DEVELOPER.into(),
            platform,
            sdk_base: None,
            local_sdk: None,
        };
        set.install(&BuildOptions {
            exact_home: Some(&home),
            ..BuildOptions::default()
        })
    };

    // 1. Build and seal {alpha, beta}.
    crate::browser::mod_list::set_enabled(&home, "alpha", true).unwrap();
    crate::browser::mod_list::set_enabled(&home, "beta", true).unwrap();
    let both = seal_enabled("pending-both");

    // 2. Turn alpha off, build and seal {beta}, and make {beta} the active build.
    crate::browser::mod_list::set_enabled(&home, "alpha", false).unwrap();
    let beta_only = seal_enabled("pending-beta");
    assert_ne!(both, beta_only);
    fs::write(
        backend::data_dir().join("settings.json"),
        format!(r#"{{"selected":"{DEVELOPER}"}}"#),
    )
    .unwrap();
    fs::write(
        backend::data_dir().join("instances.json"),
        format!(r#"{{"instances":{{"sunrise":{{"version":"{beta_only}"}}}}}}"#),
    )
    .unwrap();
    let profile_id = [1u8; 16];
    let zero = [0u8; 32];
    let active: crate::managed_play::Active = serde_json::from_value(serde_json::json!({
        "version": 1,
        "profile": profile_id,
        "generation": 1,
        "base": { "tag": DEVELOPER, "executable": zero, "runtime": zero },
        "executable": { "tag": beta_only, "executable": zero, "runtime": zero },
        "resources": { "primary": zero, "accelerator": null },
        "inputs": zero,
        "receipts": [],
    }))
    .unwrap();

    // Re-enable alpha: active build is still {beta}, but {alpha, beta} is kept and proved.
    crate::browser::mod_list::set_enabled(&home, "alpha", true).unwrap();
    assert_eq!(
        crate::native_status::unbuilt(
            &crate::native_status::enabled(&home),
            crate::native_status::built(active.executable()).as_deref(),
        ),
        ["alpha"]
    );
    assert!(
        crate::native_status::pending(&home, Some(&active)).is_empty(),
        "re-enabled mod with kept sealed build must not promise a compile"
    );
    assert!(
        crate::native_status::pending_for(&home, Some(&active), Profile::Optimized, None)
            .is_empty()
    );
    let fingerprint_before = crate::native_status::fingerprint(&home, Some(active.executable()));

    // A never-built set ({alpha, beta, gamma}) stays pending as today.
    crate::browser::mod_list::set_enabled(&home, "gamma", true).unwrap();
    let mut never_built = crate::native_status::pending(&home, Some(&active));
    never_built.sort();
    assert_eq!(never_built, ["alpha", "gamma"]);
    crate::browser::mod_list::set_enabled(&home, "gamma", false).unwrap();
    assert!(crate::native_status::pending(&home, Some(&active)).is_empty());

    // Removing the kept {alpha, beta} build in Storage invalidates the fingerprint and brings
    // the pending label and Build-and-play state back.
    let report = crate::storage::Storage::open()
        .unwrap()
        .report(Some(crate::storage::Category::SpareGameBuilds), false)
        .unwrap();
    let row = report
        .rows
        .iter()
        .find(|row| row.category == crate::storage::Category::SpareGameBuilds)
        .unwrap();
    assert!(row.kept.is_empty(), "spare build should clear cleanly");
    assert!(!backend::version_dir(&both).exists());
    assert!(backend::version_dir(&beta_only).exists());
    assert_ne!(
        crate::native_status::fingerprint(&home, Some(active.executable())),
        fingerprint_before,
        "removing a kept build in Storage must invalidate the Mods/Home fingerprint"
    );
    assert_eq!(
        crate::native_status::pending(&home, Some(&active)),
        ["alpha"]
    );
}

mod unsealed;
