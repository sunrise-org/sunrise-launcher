//! A mod that alone stops a build (it hooks code the build's Sunrise doesn't have, or declares
//! the same support module as another mod) is left out, with the mods that need it, and the
//! build runs again without them: the player still gets a game with the others, and is told
//! which mods were left out and why.
use super::Bundle;
use crate::i18n::trf;
use std::collections::BTreeSet;
use sunrise_launcher_smod::dependencies::Dependency;

/// Why a mod was left out of a build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Why {
    /// It doesn't apply to, or compile on, the Sunrise the build uses.
    OtherVersion,
    /// It declares the same support module, in the same file, as this mod.
    Clash(String),
    /// It needs this mod, which was left out.
    Needs(String),
    /// It doesn't fit the build, or works on another build only: what the player reads about it.
    Unfit(String),
    /// It was judged to fit the build, and its compile against it failed.
    DidNotBuild,
}

/// What a mod that doesn't fit was made for and what changed since, each on its own, for a
/// window that says it in its own words ([`Why::Unfit`] holds the same as one sentence).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Detail {
    /// The compiler's output for a mod that failed to compile ([`Why::DidNotBuild`]).
    pub log: Option<String>,
    /// The Nightly or release it was made for, by its version name, when that is known here.
    pub made_for: Option<String>,
    /// Each thing the game changed that it uses, or the one reason it is tied to its build.
    pub changes: Vec<String>,
}

/// A mod left out of a build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeftOut {
    pub id: String,
    pub version: String,
    pub why: Why,
    /// The parts of an [`Why::Unfit`] reason, when the check that found it gave them.
    pub detail: Option<Detail>,
}

/// A mod of the build that failed to compile, with what the player is told and the compiler's
/// own output, for the one list of failures a build ends with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildFailure {
    pub id: String,
    pub reason: String,
    pub log: String,
}

/// The mods of `left` that can't stay: one whose own compile failed (the build output names
/// it as the one that failed, with the compiler's log), one that doesn't fit the build, and one
/// that needs a mod of those. Mods left out for another reason (a clash, another version)
/// aren't on it.
pub fn build_failures(left: &[LeftOut]) -> Vec<BuildFailure> {
    left.iter()
        .filter_map(|l| {
            let (reason, log) = match &l.why {
                Why::DidNotBuild => (
                    trf(
                        "{mod} didn't compile. The problem is in the mod, not in your game.",
                        &[("mod", &l.id)],
                    ),
                    l.detail.as_ref().and_then(|d| d.log.clone()),
                ),
                Why::Unfit(line) => (line.clone(), None),
                Why::Needs(other) => (
                    trf(
                        "it needs {library}, which doesn't fit",
                        &[("library", other)],
                    ),
                    None,
                ),
                Why::OtherVersion | Why::Clash(_) => return None,
            };
            Some(BuildFailure {
                id: l.id.clone(),
                reason,
                log: log.unwrap_or_default(),
            })
        })
        .collect()
}

/// Where an instance keeps the mods left out of a game that needed no build: every mod with
/// native code was left out, so the game is the plain one (`<home>/native-left-out.json`).
const KEPT: &str = "native-left-out.json";

/// The native mods turned on, as a record keeps them: what a record was made for.
fn inputs(home: &std::path::Path) -> serde_json::Value {
    let mut mods = crate::native_status::enabled(home);
    mods.sort();
    serde_json::json!(mods)
}

/// Records that `base`, played as it is from `home`, leaves out `left`, for the native mods
/// turned on now (their ids, versions and file digests). The file is written whole: a temporary
/// file in the same folder, then a rename. Used to construct released-format fixtures.
#[cfg(test)]
pub(super) fn keep(home: &std::path::Path, base: &str, left: &[LeftOut]) -> Result<(), String> {
    let record = serde_json::json!({
        "base": base,
        "inputs": inputs(home),
        "left_out": left.iter().map(LeftOut::to_json).collect::<Vec<_>>(),
    });
    let failed = |e: &dyn std::fmt::Display| format!("{KEPT}: {e}");
    let mut temp = tempfile::Builder::new()
        .prefix(".native-left-out-")
        .tempfile_in(home)
        .map_err(|e| failed(&e))?;
    std::io::Write::write_all(&mut temp, record.to_string().as_bytes()).map_err(|e| failed(&e))?;
    temp.persist(home.join(KEPT))
        .map_err(|e| failed(&e.error))?;
    Ok(())
}

/// Whether the released record is for `base` and for the native mods turned on now: the plain game
/// is then what a build of them gives, and Play has nothing to build.
pub fn kept_current(home: &std::path::Path, base: &str) -> bool {
    let record: serde_json::Value = std::fs::read(home.join(KEPT))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    made_for(&record, base, &inputs(home))
}

/// Whether `record` was made for `base` and these native mods.
fn made_for(record: &serde_json::Value, base: &str, inputs: &serde_json::Value) -> bool {
    record["base"].as_str() == Some(base) && record["inputs"] == *inputs
}

/// What changes when the released record does, cheap to read on every frame.
pub fn kept_stamp(home: &std::path::Path) -> String {
    let meta = std::fs::metadata(home.join(KEPT)).ok();
    format!(
        "{:?}{:?}",
        meta.as_ref().and_then(|m| m.modified().ok()),
        meta.map(|m| m.len())
    )
}

/// The mods an older launcher recorded as left out of `base` for the instance playing from `home`.
pub fn kept(home: &std::path::Path, base: &str) -> Vec<LeftOut> {
    let record: serde_json::Value = std::fs::read(home.join(KEPT))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    if record["base"].as_str() != Some(base) {
        return Vec::new();
    }
    record["left_out"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(LeftOut::from_json)
        .collect()
}

impl LeftOut {
    pub(super) fn new(bundle: &Bundle, why: Why) -> Self {
        Self {
            id: bundle.manifest.id.clone(),
            version: bundle.manifest.version.clone(),
            why,
            detail: None,
        }
    }

    pub(super) fn with_detail(self, detail: Detail) -> Self {
        Self {
            detail: Some(detail),
            ..self
        }
    }

    /// What the player reads.
    pub fn label(&self) -> String {
        match &self.why {
            Why::OtherVersion => trf(
                "{name} was left out: made for another Sunrise version",
                &[("name", &self.id)],
            ),
            Why::Clash(other) => trf(
                "{name} was left out: clashes with {other}",
                &[("name", &self.id), ("other", other)],
            ),
            Why::Needs(other) => trf(
                "{name} was left out: it needs {other}, which was left out",
                &[("name", &self.id), ("other", other)],
            ),
            Why::Unfit(line) => trf(
                "{name} was left out of this build: {why}. It comes back when its author makes a version for this game.",
                &[("name", &self.id), ("why", line)],
            ),
            Why::DidNotBuild => super::fit::did_not_build(&self.id),
        }
    }

    /// Historical `left_out` representation for fixtures.
    #[cfg(test)]
    pub(super) fn to_json(&self) -> serde_json::Value {
        let (why, with) = match &self.why {
            Why::OtherVersion => ("other-version", None),
            Why::Clash(other) => ("clash", Some(other)),
            Why::Needs(other) => ("needs", Some(other)),
            Why::Unfit(line) => ("unfit", Some(line)),
            Why::DidNotBuild => ("did-not-build", None),
        };
        let mut json =
            serde_json::json!({"id": self.id, "version": self.version, "why": why, "with": with});
        if let Some(detail) = &self.detail {
            json["made_for"] = serde_json::json!(detail.made_for);
            json["changes"] = serde_json::json!(detail.changes);
            json["log"] = serde_json::json!(detail.log);
        }
        json
    }

    pub(super) fn from_json(value: &serde_json::Value) -> Option<Self> {
        let with = || value["with"].as_str().map(str::to_owned);
        Some(Self {
            id: value["id"].as_str()?.to_owned(),
            version: value["version"].as_str()?.to_owned(),
            why: match value["why"].as_str()? {
                "clash" => Why::Clash(with()?),
                "needs" => Why::Needs(with()?),
                "unfit" => Why::Unfit(with()?),
                "did-not-build" => Why::DidNotBuild,
                _ => Why::OtherVersion,
            },
            detail: value["changes"].as_array().map(|changes| Detail {
                log: value["log"].as_str().map(str::to_owned),
                made_for: value["made_for"].as_str().map(str::to_owned),
                changes: changes
                    .iter()
                    .filter_map(|c| c.as_str().map(str::to_owned))
                    .collect(),
            }),
        })
    }
}

/// `left` and, after them, every mod of `mods` that needs one of them (not optionally),
/// transitively, each once.
pub(super) fn with_dependents(left: &[LeftOut], mods: &[(&Bundle, &[Dependency])]) -> Vec<LeftOut> {
    let mut out = left.to_vec();
    let mut gone: BTreeSet<String> = left.iter().map(|l| l.id.clone()).collect();
    loop {
        let next = mods.iter().find_map(|(bundle, deps)| {
            let needed = deps.iter().find(|d| !d.optional && gone.contains(&d.id))?;
            (!gone.contains(&bundle.manifest.id))
                .then(|| LeftOut::new(bundle, Why::Needs(needed.id.clone())))
        });
        let Some(next) = next else {
            return out;
        };
        gone.insert(next.id.clone());
        out.push(next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn bundle(root: &Path, id: &str, modules: &[(&str, &str)]) -> Bundle {
        let dir = root.join(id);
        super::super::tests::write_bundle(&dir, id, "fn hook() {}");
        let path = dir.join(super::super::NATIVE_MANIFEST);
        let mut text = std::fs::read_to_string(&path).unwrap();
        for (file, name) in modules {
            text += &format!(
                "[[native.module]]\nid = \"m\"\nfile = \"{file}\"\nname = \"{name}\"\nsource = \"src/m.rs\"\n"
            );
        }
        std::fs::write(&path, text).unwrap();
        std::fs::write(dir.join("src/m.rs"), "pub fn helper() {}").unwrap();
        super::super::bundle_in(&dir).unwrap().unwrap()
    }

    /// A helper that breaks a rule of the format is the mod's own source, not
    /// a Sunrise it doesn't fit: the build fails with the tool's diagnostic
    /// (which names the injection and the code) rather than leaving the mod out
    /// as "made for another Sunrise version" (#13971).

    #[test]
    fn the_mods_needing_a_left_out_mod_go_too_but_not_optional_ones() {
        let root = super::super::tests::dir("isolate-deps");
        let [lib, addon, extra, maybe, other] =
            ["lib", "addon", "extra", "maybe", "other"].map(|id| bundle(&root, id, &[]));
        let dep = |id: &str, optional| Dependency {
            id: id.into(),
            requirement: semver::VersionReq::STAR,
            optional,
            side: sunrise_launcher_smod::manifest::Side::Both,
            reason: None,
            url: None,
            order: sunrise_launcher_smod::manifest::DependencyOrder::After,
        };
        let (on_lib, on_addon, maybe_lib) = (
            [dep("lib", false)],
            [dep("addon", false)],
            [dep("lib", true)],
        );
        let mods: Vec<(&Bundle, &[Dependency])> = vec![
            (&extra, &on_addon),
            (&lib, &[]),
            (&addon, &on_lib),
            (&maybe, &maybe_lib),
            (&other, &[]),
        ];
        let left = with_dependents(&[LeftOut::new(&lib, Why::Unfit("why".into()))], &mods);
        let ids: Vec<&str> = left.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, ["lib", "addon", "extra"]);
        assert_eq!(
            left[2].label(),
            "extra was left out: it needs addon, which was left out"
        );
        // What a build's record keeps reads back the same.
        let did_not_build = LeftOut::new(&other, Why::DidNotBuild);
        let detailed = LeftOut::new(&lib, Why::Unfit("why".into())).with_detail(Detail {
            log: None,
            made_for: Some("nightly-2026-11-16".into()),
            changes: vec!["the type Block changed".into()],
        });
        for l in left.iter().chain([&did_not_build, &detailed]) {
            assert_eq!(LeftOut::from_json(&l.to_json()).as_ref(), Some(l));
        }
        assert_eq!(
            left[0].label(),
            "lib was left out of this build: why. It comes back when its author makes a version for this game."
        );
    }

    #[test]
    fn the_record_of_a_plain_game_holds_for_the_same_mods_only() {
        let home = super::super::tests::dir("isolate-kept");
        let left = [LeftOut {
            id: "a".into(),
            version: "1.0.0".into(),
            why: Why::OtherVersion,
            detail: None,
        }];
        keep(&home, "base-1", &left).unwrap();
        assert!(kept_current(&home, "base-1"));
        assert!(!kept_current(&home, "base-2"));
        assert_eq!(kept(&home, "base-1"), left);
        // Another mod turned on is another input: the record no longer holds.
        let record: serde_json::Value =
            serde_json::from_slice(&std::fs::read(home.join(KEPT)).unwrap()).unwrap();
        let other = serde_json::json!([["b", "1.0.0", "digest"]]);
        assert!(made_for(&record, "base-1", &inputs(&home)));
        assert!(!made_for(&record, "base-1", &other));
        // Written again, it replaces the file whole and leaves no temporary file.
        keep(&home, "base-3", &[]).unwrap();
        assert!(kept_current(&home, "base-3") && kept(&home, "base-1").is_empty());
        let names: Vec<_> = std::fs::read_dir(&home)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, [KEPT]);
    }

    #[test]
    fn a_build_lists_each_mod_that_failed_to_compile_with_its_own_log() {
        let home = super::super::tests::dir("isolate-failures");
        let left = |id: &str, why, log: Option<&str>| LeftOut {
            id: id.into(),
            version: "1.0.0".into(),
            why,
            detail: log.map(|log| Detail {
                log: Some(log.into()),
                ..Detail::default()
            }),
        };
        let all = [
            left("broken", Why::DidNotBuild, Some("error[E0425] in broken")),
            left("also", Why::DidNotBuild, Some("error[E0308] in also")),
            left("needy", Why::Needs("broken".into()), None),
            left("old", Why::Unfit("made for another build".into()), None),
        ];
        let failures = build_failures(&all);
        assert_eq!(
            failures
                .iter()
                .map(|f| (f.id.as_str(), f.log.as_str()))
                .collect::<Vec<_>>(),
            [
                ("broken", "error[E0425] in broken"),
                ("also", "error[E0308] in also"),
                ("needy", ""),
                ("old", ""),
            ],
            "a mod whose own compile failed, each with its own output"
        );
        assert_eq!(failures[3].reason, "made for another build");
        let clash = left("c", Why::Clash("d".into()), None);
        assert!(build_failures(&[clash, left("o", Why::OtherVersion, None)]).is_empty());
        // The record a build keeps carries the log, so the list survives a restart.
        keep(&home, "base-1", &all).unwrap();
        assert_eq!(build_failures(&kept(&home, "base-1")), failures);
    }
}
