//! The archive a deferred first Play resolved before any full download.
use super::{
    super::*, InstallIntent, InstallOutcome, LockMode, LockRelease, ReceiptFailure, Registry,
};
use crate::managed_play::Artifact;
use std::sync::Arc;
use sunrise_launcher_release_verify::{is_build, Channel, NIGHTLY, SIGNATURE, SIGNATURE_V2, SUMS};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GameDownload {
    source: String,
    file: String,
    size: u64,
    requested: Option<GameArchitecture>,
    time: String,
}

/// Both digests come from the same verified checksum list. The launcher can retry the transfer
/// without resolving the channel again or fetching different proof for the saved archive.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GameProof {
    archive: String,
    facts: Option<String>,
}

/// An installed, verified rolling build whose separate newest-Nightly record could not be saved.
#[derive(Debug)]
pub struct NightlyReceiptWarning {
    tag: String,
    download: GameDownload,
    proof: GameProof,
    failure: ReceiptFailure,
}

impl NightlyReceiptWarning {
    pub fn tag(&self) -> &str {
        &self.tag
    }

    pub fn message(&self) -> String {
        let build = crate::nightly::display_name(&self.tag)
            .unwrap_or_else(|| crate::i18n::tr("Nightly").to_owned());
        crate::i18n::trf(
            "{build} is installed, but Sunrise couldn't record it as the newest Nightly.",
            &[("build", &build)],
        )
    }

    pub fn detail(&self) -> String {
        format!(
            "Could not save the rolling receipt for {}: {}",
            self.tag, self.failure.error
        )
    }

    /// Retry just the record, using the signed proof from the install that produced this warning.
    /// A different cache image cannot be selected, and there is no download or reinstallation.
    pub fn retry(&self) -> Result<(), String> {
        let registry = Registry::with_release(&data_dir(), LockRelease::Close)
            .map_err(|e| crate::native_mods::cache_error(e.into()))?;
        let _publication = registry
            .publication(LockMode::Exclusive)
            .map_err(|e| crate::native_mods::cache_error(e.into()))?;
        self.download.installed_at(
            &version_dir(&self.tag),
            &self.tag,
            &self.proof,
            InstallIntent::Exact,
        )?;
        let versions = versions_dir();
        if crate::nightly::current_receipt(&versions)
            .as_deref()
            .is_some_and(|current| {
                current == self.tag || Some(current) != self.failure.previous.as_deref()
            })
        {
            return Ok(());
        }
        super::save_nightly_receipt(&versions, &self.tag)
            .map_or(Ok(()), |failure| Err(failure.error.to_string()))
    }
}

impl std::fmt::Display for NightlyReceiptWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}\n{}", self.message(), self.detail())
    }
}

impl GameProof {
    pub(crate) fn saved(value: serde_json::Value) -> Result<Self, String> {
        let proof: Self = serde_json::from_value(value).map_err(|e| e.to_string())?;
        let valid = super::valid_archive_digest;
        if !valid(&proof.archive) || proof.facts.as_deref().is_some_and(|s| !valid(s)) {
            return Err(crate::i18n::tr("The saved game download proof is invalid.").into());
        }
        Ok(proof)
    }
}

/// The selected Windows archive and its saved proof, distinct from the cache marker's
/// requested preference. An ARM64 selection can bind the same x64 archive as x64 directly.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BoundVariant {
    download: GameDownload,
    proof: GameProof,
}

impl BoundVariant {
    pub(crate) fn architecture(&self) -> Option<InstalledGameArchitecture> {
        Some(InstalledGameArchitecture {
            requested: self.download.requested?,
            binary: architecture_for_asset(&self.download.file)?,
        })
    }

    pub(crate) fn verify(&self, tag: &str) -> Result<Artifact, String> {
        self.download.validate(tag)?;
        self.download
            .installed_at(&version_dir(tag), tag, &self.proof, InstallIntent::Exact)
    }
}

impl GameDownload {
    pub(crate) fn variant(&self, proof: &GameProof) -> Option<BoundVariant> {
        architecture_for_asset(&self.file)?;
        Some(BoundVariant {
            download: self.clone(),
            proof: proof.clone(),
        })
    }

    pub(crate) fn select(release: &Release) -> Result<Self, String> {
        let settings = Settings::load();
        let assets = game_assets_for(
            std::env::consts::OS,
            native_windows_architecture(),
            std::env::consts::ARCH,
            settings.game_architecture,
        );
        let (asset, fallback) = assets.map_or((ASSET, GAME_FALLBACK), |(asset, fallback)| {
            (Some(asset), fallback)
        });
        let tag = &release.tag;
        let file = asset_installed(release, asset, fallback).ok_or_else(|| {
            crate::i18n::trf(
                "{tag} has no build for the selected game architecture.",
                &[("tag", tag)],
            )
        })?;
        let source = release.nightly.as_deref().unwrap_or(tag);
        if release.nightly.is_some()
            && crate::signed::title_version(source, &release.name).as_deref() != Some(tag.as_str())
        {
            return Err(crate::i18n::trf(
                "This Nightly's title ({title}) doesn't name its date and commit as Sunrise signs them, so it can't be checked: it was not installed.",
                &[("title", &format!("{:?}", release.name))],
            ));
        }
        let size = release.asset.as_ref().expect("the selected asset").1;
        let binding = Self {
            source: source.into(),
            file: file.into(),
            size,
            requested: selected_game_architecture(&settings),
            time: release.time.clone(),
        };
        binding.validate(tag)?;
        Ok(binding)
    }

    pub(crate) fn validate(&self, tag: &str) -> Result<(), String> {
        let channel = Channel::of(&self.source);
        let tag_ok = channel.map_or_else(
            || self.source == tag && is_game_tag(tag),
            |channel| {
                is_build(channel, tag)
                    && match channel {
                        Channel::Snapshot => self.source == tag,
                        Channel::Nightly => {
                            self.source == channel.tag()
                                || tag.get(..channel.tag().len() + 11) == Some(self.source.as_str())
                        }
                    }
            },
        );
        let asset_ok = game_assets_for(
            std::env::consts::OS,
            native_windows_architecture(),
            std::env::consts::ARCH,
            self.requested.unwrap_or_default(),
        )
        .is_some_and(|(primary, fallback)| self.file == primary || fallback == Some(&self.file));
        let platform_ok = if cfg!(windows) {
            matches!(
                self.requested,
                Some(GameArchitecture::X64 | GameArchitecture::Arm64)
            )
        } else {
            self.requested.is_none()
        };
        if !tag_ok || !asset_ok || !platform_ok {
            return Err(crate::i18n::trf(
                "No exact download matches the saved build {version}.",
                &[("version", tag)],
            ));
        }
        Ok(())
    }

    pub(crate) fn prove(&self, tag: &str) -> Result<GameProof, String> {
        use sunrise_launcher_release_verify as verify;
        let channel = Channel::of(&self.source);
        let sums = fetch_small(&crate::signed::asset_url(&self.source, SUMS))?;
        let signature = fetch_small(&crate::signed::asset_url(
            &self.source,
            if channel.is_some() {
                SIGNATURE_V2
            } else {
                SIGNATURE
            },
        ))?
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
        let keys = verify::trusted_keys_for(GAME_REPO);
        let digest = |file: &str| match channel {
            Some(channel) => verify::build_digest(
                channel,
                tag,
                file,
                sums.as_deref(),
                signature.as_deref(),
                &keys,
            ),
            None => verify::signed_digest(tag, file, sums.as_deref(), signature.as_deref(), &keys),
        };
        Ok(GameProof {
            archive: digest(&self.file)?,
            facts: digest(RELEASE_FILE).ok(),
        })
    }

    pub(crate) fn install(
        &self,
        tag: &str,
        proof: &GameProof,
        progress: impl Fn(f32),
    ) -> Result<(), String> {
        let outcome = self.publish(
            tag,
            proof,
            InstallIntent::Exact,
            progress,
            crate::signed::asset_url,
        )?;
        if let Some(warning) = outcome.receipt_warning() {
            eprintln!("{warning}");
        }
        if matches!(outcome, InstallOutcome::Installed(_)) {
            for (build, error) in version_storage::prune_old_versions(&[], tag) {
                eprintln!("Could not remove old game build {build}: {error}");
            }
        }
        Ok(())
    }

    pub(in crate::backend) fn repair(
        &self,
        tag: &str,
        proof: &GameProof,
        progress: impl Fn(f32),
    ) -> Result<InstallOutcome, String> {
        self.publish(
            tag,
            proof,
            InstallIntent::Repair,
            progress,
            crate::signed::asset_url,
        )
    }

    fn receipt_warning(
        &self,
        tag: &str,
        proof: &GameProof,
        failure: ReceiptFailure,
    ) -> Arc<NightlyReceiptWarning> {
        Arc::new(NightlyReceiptWarning {
            tag: tag.to_owned(),
            download: self.clone(),
            proof: proof.clone(),
            failure,
        })
    }

    fn publish(
        &self,
        tag: &str,
        proof: &GameProof,
        intent: InstallIntent,
        progress: impl Fn(f32),
        asset_url: impl Fn(&str, &str) -> String,
    ) -> Result<InstallOutcome, String> {
        // A saved binding can reuse an archive regardless of which preference previously
        // installed it. Repair is explicit: it must also prove the new request marker.
        if intent == InstallIntent::Exact && self.is_installed(tag, proof) {
            progress(1.0);
            return Ok(InstallOutcome::AlreadyInstalled(None));
        }
        let home = version_dir(tag);
        if intent == InstallIntent::Repair
            && self.source == NIGHTLY
            && super::completion_marker(&home).is_ok_and(|complete| complete)
        {
            let registry = Registry::with_release(&data_dir(), LockRelease::Close)
                .map_err(|e| crate::native_mods::cache_error(e.into()))?;
            let _publication = registry
                .publication(LockMode::Exclusive)
                .map_err(|e| crate::native_mods::cache_error(e.into()))?;
            // A proved cache hit needs neither the archive nor a new image. A stale
            // receipt alone isn't a failed install: only a noted failure makes the
            // operation a receipt-only retry rather than an ordinary Install or Update.
            if self.installed_at(&home, tag, proof, intent).is_ok() {
                // An explicit download or repair also protects a matching automatic install.
                intent.download().mark(&home)?;
                let versions = versions_dir();
                let outcome = if crate::nightly::has_current_receipt(&versions, tag) {
                    InstallOutcome::AlreadyInstalled(None)
                } else {
                    let retry = crate::nightly::failed_install_receipt(&versions, tag);
                    let warning = super::save_nightly_receipt(&versions, tag)
                        .map(|failure| self.receipt_warning(tag, proof, failure));
                    if retry {
                        InstallOutcome::ReceiptOnly(warning)
                    } else {
                        InstallOutcome::AlreadyInstalled(warning)
                    }
                };
                progress(1.0);
                return Ok(outcome);
            }
        }
        let release = Release {
            tag: tag.into(),
            name: String::new(),
            body: String::new(),
            date: String::new(),
            time: self.time.clone(),
            asset: Some((asset_url(&self.source, &self.file), self.size)),
            prerelease: false,
            nightly: Channel::of(&self.source).map(|_| self.source.clone()),
        };
        let facts = if let Some(sha) = proof.facts.as_deref() {
            let bytes = fetch_small(&asset_url(&self.source, RELEASE_FILE))?
                .ok_or_else(|| format!("{RELEASE_FILE}: signed build facts are missing"))?;
            Some((bytes, sha))
        } else {
            None
        };
        let receipt_failure = super::install_as(
            &release,
            tag,
            &proof.archive,
            self.requested,
            facts.as_ref().map(|(bytes, sha)| (bytes.as_slice(), *sha)),
            intent,
            progress,
        )?;
        self.installed_at(&home, tag, proof, intent)?;
        Ok(InstallOutcome::Installed(
            receipt_failure.map(|failure| self.receipt_warning(tag, proof, failure)),
        ))
    }

    pub(crate) fn is_installed(&self, tag: &str, proof: &GameProof) -> bool {
        self.installed_at(&version_dir(tag), tag, proof, InstallIntent::Exact)
            .is_ok()
    }

    fn installed_at(
        &self,
        home: &Path,
        tag: &str,
        proof: &GameProof,
        intent: InstallIntent,
    ) -> Result<Artifact, String> {
        let regular = |name: &str| {
            fs::symlink_metadata(home.join(name))
                .is_ok_and(|meta| meta.is_file() && !crate::native_cache::linked(&meta))
        };
        let image = Artifact::from_archive(home, tag, &proof.archive)?;
        let facts_match = proof.facts.as_deref().is_none_or(|sha| {
            regular(RELEASE_FILE)
                && fs::read(home.join(RELEASE_FILE))
                    .is_ok_and(|bytes| sunrise_launcher_release_verify::sha256_hex(&bytes) == sha)
        });
        // A cache-only download may have reached the same signed archive as a native
        // request falling back to x64. Its preference is not part of binary identity.
        let binary_matches = architecture_for_asset(&self.file).is_none_or(|binary| {
            if !regular(GAME_BUILD_ARCHITECTURE) {
                return false;
            }
            fs::read(home.join(GAME_BUILD_ARCHITECTURE))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<InstalledGameArchitecture>(&bytes).ok())
                .is_some_and(|actual| {
                    actual.binary == binary
                        && (intent == InstallIntent::Exact
                            || Some(actual.requested) == self.requested)
                })
        });
        if !facts_match || !binary_matches {
            return Err(super::existing_build(tag));
        }
        Ok(image)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TAG: &str = "26.3-0.1.7";

    fn isolated_case(test: &str, run: impl FnOnce()) {
        const ACTOR: &str = "SUNRISE_TEST_ARCHIVE_REPAIR";
        if std::env::var_os(ACTOR).is_some() {
            run();
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let module = module_path!().split_once("::").unwrap().1;
        assert!(std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &format!("{module}::{test}")])
            .env(ACTOR, "1")
            .env("SUNRISE_HOME", root.path().join("game"))
            .env("SUNRISE_LAUNCHER_HOME", root.path().join("launcher"))
            .status()
            .unwrap()
            .success());
    }

    fn serve_windows_archive(
        requested: GameArchitecture,
        asset: &str,
        client: &[u8],
    ) -> (GameDownload, GameProof, String, std::thread::JoinHandle<()>) {
        use std::io::{Read, Write};
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        zip.start_file(CLIENT, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(client).unwrap();
        let bytes = zip.finish().unwrap().into_inner();
        let proof = GameProof {
            archive: sunrise_launcher_release_verify::sha256_hex(&bytes),
            facts: None,
        };
        let download = GameDownload {
            source: TAG.into(),
            file: asset.into(),
            requested: Some(requested),
            size: bytes.len() as u64,
            time: String::new(),
        };
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/{asset}", listener.local_addr().unwrap());
        let serve = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = stream.read(&mut [0; 4096]).unwrap();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                bytes.len()
            )
            .unwrap();
            stream.write_all(&bytes).unwrap();
        });
        (download, proof, url, serve)
    }

    #[test]
    fn explicit_repair_records_arm64_request_when_the_signed_fallback_is_unchanged() {
        isolated_case(
            "explicit_repair_records_arm64_request_when_the_signed_fallback_is_unchanged",
            || {
                use GameArchitecture::{Arm64, X64};
                let client = b"the signed x64 image";
                let (download, proof, url, server) =
                    serve_windows_archive(Arm64, "sunrise-windows-x64.zip", client);
                let home = version_dir(TAG);
                fs::create_dir_all(&home).unwrap();
                fs::write(home.join(COMPLETE), b"").unwrap();
                fs::write(home.join(CLIENT), client).unwrap();
                fs::write(
                    home.join(GAME_BUILD_ARCHITECTURE),
                    br#"{"requested":"x64","binary":"x64"}"#,
                )
                .unwrap();
                Artifact::record_archive(&home, TAG, &proof.archive).unwrap();
                assert!(
                    download.is_installed(TAG, &proof),
                    "exact Play can already reuse these signed bytes"
                );
                assert!(check_game_architecture(TAG, None, Arm64).is_some());

                download
                    .publish(
                        TAG,
                        &proof,
                        InstallIntent::Repair,
                        |_| {},
                        |_, _| url.clone(),
                    )
                    .unwrap();

                assert!(download
                    .installed_at(&home, TAG, &proof, InstallIntent::Repair)
                    .is_ok());
                let repaired = installed_game_architecture(TAG).unwrap();
                assert_eq!((repaired.requested, repaired.binary), (Arm64, X64));
                assert!(check_game_architecture(TAG, None, Arm64).is_none());
                server.join().unwrap();
            },
        );
    }

    #[test]
    fn a_committed_nightly_survives_receipt_failure_and_retries_only_the_receipt() {
        isolated_case(
            "a_committed_nightly_survives_receipt_failure_and_retries_only_the_receipt",
            || {
                const ROLLING: &str = "nightly-2026-11-20-aaaaaaa";
                let (mut download, proof, url, server) = serve_windows_archive(
                    GameArchitecture::X64,
                    "sunrise-windows-x64.zip",
                    b"new signed Nightly image",
                );
                download.source = NIGHTLY.into();
                let versions = versions_dir();
                let target = version_dir(ROLLING);
                let receipt = versions.join(".nightly-current");
                fs::create_dir_all(&target).unwrap();
                fs::write(target.join(COMPLETE), "").unwrap();
                fs::write(target.join(CLIENT), b"previous image").unwrap();
                // Force just the atomic receipt replacement to fail on each platform.
                fs::create_dir(&receipt).unwrap();
                fs::write(receipt.join("obstruction"), "leave this alone").unwrap();
                let result = download
                    .publish(
                        ROLLING,
                        &proof,
                        InstallIntent::Repair,
                        |_| {},
                        |_, _| url.clone(),
                    )
                    .unwrap();
                server.join().unwrap();
                let InstallOutcome::Installed(Some(warning)) = result else {
                    panic!("expected a published build with a receipt warning: {result:?}");
                };
                assert_eq!(
                    warning.message(),
                    "Nightly · Nov 20 is installed, but Sunrise couldn't record it as the newest Nightly."
                );
                assert_eq!(
                    warning.to_string(),
                    format!(
                        "{}\nCould not save the rolling receipt for {ROLLING}: {}",
                        warning.message(),
                        warning.failure.error
                    ),
                );
                // The Storage action retries only the record; another failure still leaves it installed.
                assert!(warning.retry().is_err());
                assert_eq!(
                    fs::read(receipt.join("obstruction")).unwrap(),
                    b"leave this alone"
                );
                assert_eq!(installed(), [ROLLING]);
                assert_eq!(
                    fs::read(target.join(CLIENT)).unwrap(),
                    b"new signed Nightly image"
                );
                let identity = crate::profile_txn::Identity::of(&target).unwrap();
                let image = download
                    .installed_at(&target, ROLLING, &proof, InstallIntent::Repair)
                    .unwrap();
                assert!(!versions.join(format!(".repair-{ROLLING}")).exists());

                fs::remove_dir_all(&receipt).unwrap();
                fs::write(&receipt, "nightly-2026-11-20-bbbbbbb").unwrap();
                let retry = download
                    .publish(
                        ROLLING,
                        &proof,
                        InstallIntent::Repair,
                        |_| {},
                        |_, _| panic!("receipt retry must not request any assets"),
                    )
                    .unwrap();
                assert!(
                    matches!(retry, InstallOutcome::ReceiptOnly(None)),
                    "{retry:?}"
                );
                assert_eq!(fs::read_to_string(&receipt).unwrap(), ROLLING);
                assert!(!crate::nightly::failed_install_receipt(&versions, ROLLING));
                // Exercise the Storage action with no release lookup.
                fs::remove_file(&receipt).unwrap();
                warning.retry().unwrap();
                assert!(crate::nightly::has_current_receipt(&versions, ROLLING));
                assert_eq!(crate::profile_txn::Identity::of(&target).unwrap(), identity);
                assert_eq!(
                    download
                        .installed_at(&target, ROLLING, &proof, InstallIntent::Repair)
                        .unwrap(),
                    image
                );

                // A later successful record wins over the stale warning.
                let later = "nightly-2026-11-20-bbbbbbb";
                fs::write(&receipt, later).unwrap();
                warning.retry().unwrap();
                assert_eq!(fs::read_to_string(&receipt).unwrap(), later);

                fs::remove_file(&receipt).unwrap();
                fs::write(target.join(CLIENT), b"a different image").unwrap();
                assert!(warning.retry().is_err());
                assert!(!crate::nightly::has_current_receipt(&versions, ROLLING));
            },
        );
    }

    #[test]
    fn install_nightly_selects_the_current_build_cached_by_the_picker() {
        isolated_case(
            "install_nightly_selects_the_current_build_cached_by_the_picker",
            || {
                const ROLLING: &str = "nightly-2026-11-20-aaaaaaa";
                let (mut download, proof, url, server) = serve_windows_archive(
                    GameArchitecture::X64,
                    "sunrise-windows-x64.zip",
                    b"verified Nightly image",
                );
                download.source = NIGHTLY.into();
                let stable = version_dir(TAG);
                fs::create_dir_all(&stable).unwrap();
                fs::write(stable.join(COMPLETE), []).unwrap();
                let old = version_dir("26.3-0.1.6");
                fs::create_dir_all(&old).unwrap();
                fs::write(old.join(COMPLETE), []).unwrap();
                version_storage::Download::Automatic.mark(&old).unwrap();
                let mut settings = Settings::load();
                settings.selected = Some(TAG.into());
                settings.keep_versions = Some(1);
                settings.save().unwrap();

                // The picker publishes through the same worker without changing selection or pruning.
                let cached = download
                    .publish(
                        ROLLING,
                        &proof,
                        InstallIntent::Repair,
                        |_| {},
                        |_, _| url.clone(),
                    )
                    .unwrap();
                server.join().unwrap();
                assert!(matches!(cached, InstallOutcome::Installed(None)));
                assert_eq!(Settings::load().selected.as_deref(), Some(TAG));
                assert!(old.join(COMPLETE).is_file());
                let versions = versions_dir();
                assert!(crate::nightly::has_current_receipt(&versions, ROLLING));
                let target = version_dir(ROLLING);
                let identity = crate::profile_txn::Identity::of(&target).unwrap();
                let release = Release {
                    tag: ROLLING.into(),
                    name: "Nightly 2026-11-20 · aaaaaaa".into(),
                    body: String::new(),
                    date: "2026-11-20".into(),
                    time: String::new(),
                    asset: Some((url, download.size)),
                    prerelease: true,
                    nightly: Some(NIGHTLY.into()),
                };

                // Exercise both an already-current receipt and a stale receipt without a
                // failed install. Neither should suppress the CLI's normal selection.
                for prior_receipt in [ROLLING, "nightly-2026-11-20-bbbbbbb"] {
                    crate::nightly::record_current(&versions, prior_receipt).unwrap();
                    settings.save().unwrap();
                    let selected = install_offered_tag(
                        std::slice::from_ref(&release),
                        NIGHTLY,
                        false,
                        |offered| {
                            assert_eq!(offered.tag, ROLLING);
                            download.publish(
                                &offered.tag,
                                &proof,
                                InstallIntent::Repair,
                                |_| {},
                                |_, _| panic!("a cache hit must not request any assets"),
                            )
                        },
                    )
                    .unwrap();
                    assert_eq!(selected, ROLLING);
                    assert_eq!(Settings::load().selected.as_deref(), Some(ROLLING));
                    assert!(crate::nightly::has_current_receipt(&versions, ROLLING));
                    assert_eq!(crate::profile_txn::Identity::of(&target).unwrap(), identity);
                }
                assert!(
                    !old.exists(),
                    "a regular install prunes the old automatic build"
                );
            },
        );
    }

    #[test]
    fn ordinary_install_cannot_claim_an_absent_pending_first_play_build() {
        use crate::{
            managed_play::ReferencePublication,
            profile_txn::{self, Admission, Bootstrap},
        };
        use GameArchitecture::{Arm64, X64};

        isolated_case(
            "ordinary_install_cannot_claim_an_absent_pending_first_play_build",
            || {
                let home = game_home();
                fs::create_dir_all(&home).unwrap();
                let (frozen, proof, url, server) =
                    serve_windows_archive(X64, "sunrise-windows-x64.zip", b"frozen x64 image");
                let target = version_dir(TAG);
                let saved = std::cell::OnceCell::new();
                let (other, other_proof, other_url, other_server) =
                    serve_windows_archive(Arm64, "sunrise-windows-arm64.zip", b"other arm64 image");
                let ordinary = other.publish(
                    TAG,
                    &other_proof,
                    InstallIntent::Repair,
                    |_| {
                        if saved.get().is_some() {
                            return;
                        }
                        assert!(!target.exists());
                        // Leave an interrupted First Play binding while the ordinary archive
                        // is still private. The cache target has never been published.
                        let mut admission = Admission::open(&home, None).unwrap();
                        admission
                            .bind_bootstrap(
                                || {
                                    Ok(Bootstrap::new(
                                        TAG.into(),
                                        TAG.into(),
                                        [1; 16],
                                        [2; 32],
                                        serde_json::json!({"kind":"download", "archive":frozen}),
                                    ))
                                },
                                |_| ReferencePublication::begin(&home),
                            )
                            .unwrap();
                        let _fence = ReferencePublication::begin(&home).unwrap();
                        let binding = admission
                            .prove_bootstrap(|_| {
                                serde_json::to_value(&proof).map_err(|e| e.to_string())
                            })
                            .unwrap();
                        saved.set(binding).unwrap();
                    },
                    |_, _| other_url.clone(),
                );
                assert_eq!(
                    ordinary.unwrap_err(),
                    crate::native_mods::cache_error(crate::native_cache::Error::ReferencedProfile)
                );
                other_server.join().unwrap();
                assert!(!target.exists());
                let references = profile_txn::references(&home).unwrap();
                assert!(references.targets.contains(TAG));
                assert!(references.pointer.is_none());

                let mut admission =
                    Admission::open_fenced(&home, None, |_| ReferencePublication::begin(&home))
                        .unwrap();
                let resumed = admission
                    .bind_bootstrap::<ReferencePublication>(
                        || panic!("re-resolved the interrupted binding"),
                        |_| panic!("replaced the interrupted binding"),
                    )
                    .unwrap();
                assert_eq!(resumed, saved.into_inner().unwrap());
                let download: GameDownload =
                    serde_json::from_value(resumed.selection["archive"].clone()).unwrap();
                let proof = GameProof::saved(resumed.proof.unwrap()).unwrap();
                download
                    .publish(
                        TAG,
                        &proof,
                        InstallIntent::Exact,
                        |_| {},
                        |_, _| url.clone(),
                    )
                    .unwrap();
                server.join().unwrap();
                assert!(download.is_installed(TAG, &proof));
                assert_eq!(fs::read(target.join(CLIENT)).unwrap(), b"frozen x64 image");
            },
        );
    }

    #[test]
    fn both_requests_reuse_the_same_signed_fallback_but_not_a_different_binary() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let download = GameDownload {
            source: "nightly".into(),
            file: "sunrise-windows-x64.zip".into(),
            size: 0,
            requested: Some(GameArchitecture::Arm64),
            time: String::new(),
        };
        let proof = GameProof {
            archive: "a".repeat(64),
            facts: None,
        };
        let variant = download.variant(&proof).unwrap();
        assert!(installed_architecture_matches(
            GameArchitecture::Arm64,
            variant.architecture().unwrap()
        ));
        let archive = home.join(Artifact::ARCHIVE_RECEIPT);
        let tag = "nightly-2026-11-20-aabbccd";
        fs::write(home.join(COMPLETE), b"").unwrap();
        fs::write(home.join(CLIENT), b"client").unwrap();
        let marker = |requested, binary| {
            std::fs::write(
                home.join(GAME_BUILD_ARCHITECTURE),
                serde_json::to_vec(&InstalledGameArchitecture { requested, binary }).unwrap(),
            )
            .unwrap();
        };
        use GameArchitecture::{Arm64, X64};
        marker(Arm64, X64);
        Artifact::record_archive(home, tag, &"b".repeat(64)).unwrap();
        assert!(
            download
                .installed_at(home, tag, &proof, InstallIntent::Exact)
                .is_err(),
            "wrong archive"
        );
        fs::remove_file(&archive).unwrap();
        let direct = GameDownload {
            requested: Some(X64),
            ..download.clone()
        };
        for request in [Arm64, X64] {
            marker(request, X64);
            Artifact::record_archive(home, tag, &proof.archive).unwrap();
            assert!(download
                .installed_at(home, tag, &proof, InstallIntent::Exact)
                .is_ok());
            assert!(direct
                .installed_at(home, tag, &proof, InstallIntent::Exact)
                .is_ok());
            fs::remove_file(&archive).unwrap();
            marker(request, Arm64);
            Artifact::record_archive(home, tag, &proof.archive).unwrap();
            assert!(
                download
                    .installed_at(home, tag, &proof, InstallIntent::Exact)
                    .is_err(),
                "wrong binary"
            );
            assert!(
                direct
                    .installed_at(home, tag, &proof, InstallIntent::Exact)
                    .is_err(),
                "wrong binary"
            );
            fs::remove_file(&archive).unwrap();
        }
    }
}
