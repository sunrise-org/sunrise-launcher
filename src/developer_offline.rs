//! Explicit account-free Play for persisted unsigned checkout lineage.
use crate::{
    developer_install,
    i18n::{tr, trf},
};
use std::process::Command;

pub const ENV: &str = "SUNRISE_LAUNCHER_DEVELOPER_OFFLINE";
pub const FLAG: &str = "--developer-single-player";
pub const NOTICE: &str =
    crate::i18n::tr_key("Offline developer session: single-player only; no account features.");

pub fn requested() -> bool {
    std::env::var_os(ENV).as_deref() == Some(std::ffi::OsStr::new("1"))
}

pub fn enabled(tag: &str) -> bool {
    if !requested() {
        return false;
    }
    let native = crate::native_mods::about(tag);
    let base = native
        .as_ref()
        .map_or_else(|| base(tag), |a| a.base.clone());
    eligible(
        &base,
        &crate::backend::version_dir(&base),
        native.as_ref().map(|a| a.sunrise.as_str()),
    )
}

/// A file's time and size, if it is there.
pub type Stamp = Option<(std::time::SystemTime, u64)>;

/// `path` with its time and size.
pub fn stamp(path: std::path::PathBuf) -> (std::path::PathBuf, Stamp) {
    let meta = std::fs::metadata(&path).ok();
    let stamp = meta.and_then(|m| Some((m.modified().ok()?, m.len())));
    (path, stamp)
}

/// The lineage records [`enabled`] reads for `tag`: a native build's `native-mods.json`, an
/// imported build's record. They name its base build ([`base`]).
pub fn records(tag: &str) -> [std::path::PathBuf; 2] {
    let own = crate::backend::version_dir(tag);
    [
        own.join(crate::native_mods::RECORD),
        own.join(crate::build_import::RECORD),
    ]
}

/// The build `tag` is made from, as its lineage records say ([`records`]), else itself: the
/// build whose records [`enabled`] checks.
pub fn base(tag: &str) -> String {
    crate::native_mods::about(tag)
        .map(|a| a.base)
        .or_else(|| crate::build_import::about(tag).map(|a| a.base))
        .unwrap_or_else(|| tag.to_owned())
}

/// The base build's records [`enabled`] reads: its install mark and developer record.
pub fn base_records(base: &str) -> [std::path::PathBuf; 2] {
    let dir = crate::backend::version_dir(base);
    [dir.join(".installed"), dir.join("developer.json")]
}

/// Every file [`enabled`] reads for `tag`, with its time and size: [`records`], then the
/// [`base_records`] of its [`base`]. While these stay the same, so does [`enabled`]'s answer for
/// `tag`, so a caller can keep it instead of asking at every frame.
pub fn inputs(tag: &str) -> Vec<(std::path::PathBuf, Stamp)> {
    records(tag)
        .into_iter()
        .chain(base_records(&base(tag)))
        .map(stamp)
        .collect()
}

fn eligible(base: &str, directory: &std::path::Path, native_revision: Option<&str>) -> bool {
    developer_install::is_developer(base)
        && directory.join(".installed").is_file()
        && developer_install::revision_in(directory)
            .is_ok_and(|revision| native_revision.is_none_or(|expected| expected == revision))
}

pub const REBUILD: &str = crate::i18n::tr_key("This game cannot enforce offline developer single-player. Rebuild and reinstall the developer game and its native mods.");
const START_FAILED: &str = crate::i18n::tr_key("The game failed to start while checking support for offline developer play. Repeat your sunrise-launcher developer-install command, rebuild this build’s native mods with native-build, then try Play again. If it still fails, report this message on the Sunrise Discord (linked in the launcher).\nDetails: {reason}");
pub fn require_capability(tag: &str) -> Result<(), String> {
    let program = crate::backend::client_program(tag);
    let output = Command::new(&program)
        .arg("--launch-api")
        .output()
        .map_err(|e| startup_failure(&format!("{}: {e}", program.display())))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let reason = stderr
            .lines()
            .map(str::trim)
            .find(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                trf(
                    "The game stopped without an error message ({status}).",
                    &[("status", &output.status.to_string())],
                )
            });
        return Err(startup_failure(&reason));
    }
    if supports_restriction(&output.stdout) {
        Ok(())
    } else {
        Err(tr(REBUILD).into())
    }
}

// Both spawn failures and child diagnostics cross this user-visible boundary.
fn startup_failure(reason: &str) -> String {
    let mut chars = reason.chars();
    let mut reason: String = chars.by_ref().take(500).collect();
    if chars.next().is_some() {
        reason.push('…');
    }
    trf(START_FAILED, &[("reason", &reason)])
}

fn supports_restriction(bytes: &[u8]) -> bool {
    #[derive(serde::Deserialize)]
    struct Api {
        options: Vec<OptionFlag>,
    }
    #[derive(serde::Deserialize)]
    struct OptionFlag {
        name: String,
        value: Option<String>,
    }
    serde_json::from_slice::<Api>(bytes).is_ok_and(|api| {
        api.options
            .iter()
            .any(|o| o.name == FLAG && o.value.is_none())
    })
}

pub fn mark(cmd: &mut Command) {
    cmd.arg(FLAG)
        .env_remove("SUNRISE_SESSION_FILE")
        .env_remove("SUNRISE_LAUNCHER_EXE");
}

/// Validate after instance arguments and wrappers have been applied, before acquiring real leases.
pub fn validate(cmd: &Command) -> Result<(), String> {
    let offline = cmd.get_args().any(|a| a == FLAG);
    if offline
        && cmd.get_envs().any(|(key, value)| {
            key == "SUNRISE_SESSION_FILE" && value.is_some_and(|v| !v.is_empty())
        })
    {
        return Err(tr(NOTICE).into());
    }
    if offline
        && cmd.get_args().any(|a| {
            let a = a.to_string_lossy();
            [
                "--server",
                "--quickPlayMultiplayer",
                "--session-file",
                "--session-stdin",
                "--dedicated",
                "--transfers",
            ]
            .iter()
            .any(|flag| a == *flag || a.starts_with(&format!("{flag}=")))
        })
    {
        return Err(tr(NOTICE).into());
    }
    Ok(())
}

pub fn selected() -> bool {
    let instance = crate::instances::selected();
    instance.playing_version().is_ok_and(|tag| enabled(&tag))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_inputs_are_the_files_eligibility_reads_the_base_builds_included() {
        // An imported build of a developer base: its own record, then the base's install mark
        // and developer record, as `enabled` reads them.
        let tag = "imported-inputs-test";
        let own = crate::backend::version_dir(tag);
        std::fs::create_dir_all(&own).unwrap();
        std::fs::write(
            own.join(crate::build_import::RECORD),
            r#"{"base": "developer-inputs-base", "file": "x.jar"}"#,
        )
        .unwrap();
        let base = crate::backend::version_dir("developer-inputs-base");
        let paths: Vec<_> = inputs(tag).into_iter().map(|(path, _)| path).collect();
        assert_eq!(
            paths,
            [
                own.join(crate::native_mods::RECORD),
                own.join(crate::build_import::RECORD),
                base.join(".installed"),
                base.join("developer.json"),
            ]
        );
        // The base reinstalled under the same tag: the inputs change.
        let before = inputs(tag);
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join(".installed"), "").unwrap();
        assert_ne!(inputs(tag), before);
        std::fs::remove_dir_all(&own).unwrap();
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn startup_failure_bounds_unicode_details_without_changing_short_diagnostics() {
        let short = "dyld[4321]: Library not loaded: @rpath/libsunrise_mod_host.dylib";
        assert!(startup_failure(short).ends_with(short));
        let long = "界".repeat(501);
        let message = startup_failure(&long);
        let details = message.split_once("Details: ").unwrap().1;
        assert_eq!(details.chars().count(), 501);
        assert!(details.ends_with('…'));
        assert!(!startup_failure(&"界".repeat(500)).ends_with('…'));
    }

    #[test]
    fn account_gate_is_preserved_without_opt_in_and_for_signed_lineage() {
        if let Ok(tag) = std::env::var("OFFLINE_GATE_TEST_TAG") {
            assert!(!enabled(&tag));
            let error = crate::backend::client_as(&tag, None).unwrap_err();
            assert!(
                error.contains(crate::i18n::tr(crate::backend::SIGN_IN_HINT)),
                "{error}"
            );
            return;
        }
        for (tag, opt_in) in [
            ("developer-local", None),
            ("v2026.11.10", Some("1")),
            ("nightly-abcdef", Some("1")),
            ("developer-local", Some("true")),
        ] {
            let root = tempfile::tempdir().unwrap();
            let version = root.path().join("versions").join(tag);
            std::fs::create_dir_all(&version).unwrap();
            std::fs::write(version.join(".installed"), "").unwrap();
            std::fs::write(
                version.join("developer.json"),
                serde_json::json!({"revision":"a".repeat(40)}).to_string(),
            )
            .unwrap();
            let mut child = Command::new(std::env::current_exe().unwrap());
            child.args(["--exact", "developer_offline::tests::account_gate_is_preserved_without_opt_in_and_for_signed_lineage", "--nocapture"])
                .env("OFFLINE_GATE_TEST_TAG", tag)
                .env("SUNRISE_LAUNCHER_HOME", root.path())
                .env("SUNRISE_HOME", root.path().join("game"))
                .env("XDG_CONFIG_HOME", root.path().join("config"))
                .env("XDG_DATA_HOME", root.path().join("data"));
            if let Some(value) = opt_in {
                child.env(ENV, value);
            } else {
                child.env_remove(ENV);
            }
            let output = child.output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
    }

    #[test]
    fn legacy_and_malformed_capabilities_refuse() {
        for reply in [
            b"not json".as_slice(),
            br#"{}"#,
            br#"{"options":[]}"#,
            br#"{"options":[{"name":"--developer-single-player","value":"VALUE"}]}"#,
        ] {
            assert!(!supports_restriction(reply));
        }
        assert!(supports_restriction(
            br#"{"options":[{"name":"--developer-single-player","value":null}]}"#
        ));
    }
    #[test]
    fn persisted_provenance_not_just_a_prefix() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path();
        assert!(!eligible("developer-local", dir, None));
        std::fs::write(dir.join(".installed"), "").unwrap();
        for record in ["{}", "not json", r#"{"revision":"wrong"}"#] {
            std::fs::write(dir.join("developer.json"), record).unwrap();
            assert!(!eligible("developer-local", dir, None));
        }
        std::fs::write(
            dir.join("developer.json"),
            serde_json::json!({"revision":"a".repeat(40)}).to_string(),
        )
        .unwrap();
        assert!(eligible("developer-local", dir, None));
        assert!(eligible("developer-local", dir, Some(&"a".repeat(40))));
        assert!(!eligible("developer-local", dir, Some(&"b".repeat(40))));
        for base in ["v2026.11.10", "nightly-abcdef", "native-signed"] {
            assert!(!eligible(base, dir, None));
        }
    }
    #[test]
    fn composition_cannot_reintroduce_account_state_or_clear_the_restriction() {
        let mut command = Command::new("game");
        command.env("SUNRISE_SESSION_FILE", "inherited-session.json");
        mark(&mut command);
        for key in ["SUNRISE_SESSION_FILE", "SUNRISE_LAUNCHER_EXE"] {
            assert_eq!(
                command.get_envs().find(|(name, _)| *name == key).unwrap().1,
                None
            );
        }
        command.env(ENV, "0");
        assert!(command.get_args().any(|argument| argument == FLAG));
        assert!(validate(&command).is_ok());
        let mut command = Command::new("game");
        mark(&mut command);
        let launch = crate::launch::Launch {
            env: [(
                "SUNRISE_SESSION_FILE".to_string(),
                "reintroduced".to_string(),
            )]
            .into(),
            ..Default::default()
        };
        let wrapped = crate::launch::wrap(command, &launch).unwrap();
        assert!(validate(&wrapped).is_err());
    }

    #[test]
    fn final_network_and_session_intents_are_refused() {
        for flag in [
            "--server",
            "--quickPlayMultiplayer",
            "--session-file",
            "--session-stdin",
            "--dedicated",
            "--transfers",
            "--server=localhost",
        ] {
            let mut command = Command::new("game");
            mark(&mut command);
            command.arg(flag);
            assert!(validate(&command).is_err(), "{flag}");
        }
        let mut command = Command::new("game");
        mark(&mut command);
        command.args(["--world", "DeveloperProof", "--creative"]);
        assert!(validate(&command).is_ok());
        let mut normal = Command::new("game");
        normal.args(["--server", "localhost"]);
        assert!(validate(&normal).is_ok());
    }
}
