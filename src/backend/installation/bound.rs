//! The game image a deferred first Play resolved before any full download.
use super::{
    super::*, InstallIntent, InstallOutcome, LockMode, LockRelease, ReceiptFailure, Registry,
};
use crate::{
    managed_play::Artifact,
    native_mods::sdk_cache::{stage_game, Base, Refusal},
};
use std::sync::Arc;
use sunrise_launcher_release_verify::{
    is_build, Channel, PublicKey, NIGHTLY, SIGNATURE, SIGNATURE_V2, SUMS,
};
use sunrise_launcher_sdk::INDEX;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GameDownload {
    source: String,
    file: String,
    size: u64,
    requested: Option<GameArchitecture>,
    time: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    published_at: String,
}

/// Both digests come from the same verified checksum list. The `archive` field in the saved
/// proof binds either the player archive or the SDK index, according to the bound download.
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
        crate::i18n::trf(
            "Could not save the rolling receipt for {tag}: {error}",
            &[
                ("tag", &self.tag),
                ("error", &self.failure.error.to_string()),
            ],
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
    pub(crate) fn archive(&self) -> &str {
        &self.archive
    }

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
        self.download.architecture()
    }

    pub(crate) fn verify(&self, tag: &str) -> Result<Artifact, String> {
        self.download.validate(tag)?;
        self.download
            .installed_at(&version_dir(tag), tag, &self.proof, InstallIntent::Exact)
    }
}

impl GameDownload {
    fn architecture(&self) -> Option<InstalledGameArchitecture> {
        let requested = self.requested?;
        let binary = if self.file == INDEX {
            GameArchitecture::X64
        } else {
            architecture_for_asset(&self.file)?
        };
        Some(InstalledGameArchitecture { requested, binary })
    }

    pub(crate) fn variant(&self, proof: &GameProof) -> Option<BoundVariant> {
        self.architecture()?;
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
        let requested = selected_game_architecture(&settings);
        let file = asset_installed(release, asset, fallback)
            .or_else(|| {
                let (url, _) = release.asset.as_ref()?;
                (url.rsplit('/').next() == Some(INDEX)).then_some(INDEX)
            })
            .ok_or_else(|| {
                crate::i18n::trf(
                    "{tag} has no build for the selected game architecture.",
                    &[("tag", tag)],
                )
            })?;
        let source = release.nightly.as_deref().unwrap_or(tag);
        if release.nightly.is_some()
            && crate::signed::title_version(source, &release.name).as_deref() != Some(tag.as_str())
        {
            let message = if Channel::of(source) == Some(Channel::Dev) {
                crate::i18n::tr_key("This Dev build's title ({title}) doesn't name its date and commit as Sunrise signs them, so it can't be checked: it was not installed.")
            } else {
                crate::i18n::tr_key("This Nightly's title ({title}) doesn't name its date and commit as Sunrise signs them, so it can't be checked: it was not installed.")
            };
            return Err(crate::i18n::trf(
                message,
                &[("title", &format!("{:?}", release.name))],
            ));
        }
        let size = release.asset.as_ref().expect("the selected asset").1;
        let binding = Self {
            source: source.into(),
            file: file.into(),
            size,
            requested,
            time: release.time.clone(),
            published_at: release.published_at.clone(),
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
                        Channel::Dev => self.source == tag,
                        Channel::Nightly => {
                            self.source == channel.tag()
                                || tag.get(..channel.tag().len() + 11) == Some(self.source.as_str())
                        }
                    }
            },
        );
        let asset_ok = if self.file == INDEX {
            sdk_target(self.requested).is_some()
        } else {
            game_assets_for(
                std::env::consts::OS,
                native_windows_architecture(),
                std::env::consts::ARCH,
                self.requested.unwrap_or_default(),
            )
            .is_some_and(|(primary, fallback)| self.file == primary || fallback == Some(&self.file))
        };
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
        self.prove_cancellable(tag, None)
    }

    pub(crate) fn prove_cancellable(
        &self,
        tag: &str,
        stop: Option<&crate::native_mods::Cancellation>,
    ) -> Result<GameProof, String> {
        let keys = sunrise_launcher_release_verify::trusted_keys_for(GAME_REPO);
        if self.file == INDEX {
            let base = Base::from_env();
            self.prove_with(tag, &keys, |name| {
                if let Some(stop) = stop {
                    stop.check()?;
                }
                base.release_file(&self.source, tag, name)
                    .map_err(|error| error.to_string())
            })
        } else {
            self.prove_with(tag, &keys, |name| {
                fetch_small_cancellable(&crate::signed::asset_url(&self.source, name), stop)
            })
        }
    }

    fn prove_with(
        &self,
        tag: &str,
        keys: &[PublicKey],
        fetch: impl Fn(&str) -> Result<Option<Vec<u8>>, String>,
    ) -> Result<GameProof, String> {
        prove_release_files(
            &self.source,
            tag,
            &self.file,
            self.file == INDEX,
            keys,
            fetch,
        )
    }

    pub(crate) fn install(
        &self,
        tag: &str,
        proof: &GameProof,
        progress: impl Fn(f32),
    ) -> Result<(), String> {
        self.install_cancellable(tag, proof, None, progress)
    }

    /// [`Self::install`], which stops at the next safe point once `stop` is asked: nothing
    /// half unpacked is published, and what was downloaded stays resumable.
    pub(crate) fn install_cancellable(
        &self,
        tag: &str,
        proof: &GameProof,
        stop: Option<&crate::native_mods::Cancellation>,
        progress: impl Fn(f32),
    ) -> Result<(), String> {
        let outcome = self.publish_cancellable(
            tag,
            proof,
            InstallIntent::Exact,
            stop,
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

    pub(in crate::backend) fn repair_cancellable(
        &self,
        tag: &str,
        proof: &GameProof,
        stop: Option<&crate::native_mods::Cancellation>,
        progress: impl Fn(f32),
    ) -> Result<InstallOutcome, String> {
        self.publish_cancellable(
            tag,
            proof,
            InstallIntent::Repair,
            stop,
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

    #[cfg(test)]
    fn publish(
        &self,
        tag: &str,
        proof: &GameProof,
        intent: InstallIntent,
        progress: impl Fn(f32),
        asset_url: impl Fn(&str, &str) -> String,
    ) -> Result<InstallOutcome, String> {
        self.publish_cancellable(tag, proof, intent, None, progress, asset_url)
    }

    fn publish_cancellable(
        &self,
        tag: &str,
        proof: &GameProof,
        intent: InstallIntent,
        stop: Option<&crate::native_mods::Cancellation>,
        progress: impl Fn(f32),
        asset_url: impl Fn(&str, &str) -> String,
    ) -> Result<InstallOutcome, String> {
        self.publish_with(
            tag,
            proof,
            intent,
            super::Transfer { stop, progress },
            asset_url,
            (
                &Base::from_env(),
                &sunrise_launcher_release_verify::trusted_keys_for(GAME_REPO),
            ),
        )
    }

    fn publish_with(
        &self,
        tag: &str,
        proof: &GameProof,
        intent: InstallIntent,
        transfer: super::Transfer<'_, impl Fn(f32)>,
        asset_url: impl Fn(&str, &str) -> String,
        (base, keys): (&Base, &[PublicKey]),
    ) -> Result<InstallOutcome, String> {
        let super::Transfer { stop, progress } = transfer;
        if let Some(stop) = stop {
            stop.check()?;
        }
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
            published_at: self.published_at.clone(),
            asset: (self.file != INDEX).then(|| (asset_url(&self.source, &self.file), self.size)),
            prerelease: false,
            nightly: Channel::of(&self.source).map(|_| self.source.clone()),
        };
        let sdk_file = |name| {
            base.release_file(&self.source, tag, name)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("{name}: {}", std::io::ErrorKind::NotFound))
        };
        let receipt_failure = if self.file == INDEX {
            let triple = sdk_target(self.requested).expect("the target of a validated download");
            let facts_sha = proof.facts.as_deref().ok_or_else(|| {
                crate::i18n::tr("The saved game download proof is invalid.").to_owned()
            })?;
            super::install_image(
                &release,
                tag,
                intent,
                stop,
                &progress,
                |target| self.installed_at(target, tag, proof, intent).is_ok(),
                |stage| {
                    if let Some(stop) = stop {
                        stop.check()?;
                    }
                    let game_index = sdk_file(INDEX)?;
                    sunrise_launcher_release_verify::check(
                        INDEX,
                        &proof.archive,
                        &sunrise_launcher_release_verify::sha256_hex(&game_index),
                    )?;
                    let facts = sdk_file(RELEASE_FILE)?;
                    let (top, slices) = stage_game(
                        stage,
                        tag,
                        triple,
                        (&game_index, &proof.archive),
                        (base, keys),
                        (stop, &progress),
                    )
                    .map_err(|error| match error {
                        Refusal::Host => crate::i18n::trf(
                            "{tag} has no build for the selected game architecture.",
                            &[("tag", tag)],
                        ),
                        other => other.to_string(),
                    })?;
                    progress(0.97);
                    super::finish_image(&top, self.architecture(), Some((&facts, facts_sha)))?;
                    Artifact::record_sdk(&top, tag, slices)?;
                    Ok(top)
                },
            )?
        } else {
            let facts = if let Some(sha) = proof.facts.as_deref() {
                let bytes = fetch_small_cancellable(&asset_url(&self.source, RELEASE_FILE), stop)?
                    .ok_or_else(|| {
                        crate::i18n::trf(
                            "{file}: signed build facts are missing",
                            &[("file", RELEASE_FILE)],
                        )
                    })?;
                Some((bytes, sha))
            } else {
                None
            };
            super::install_as_cancellable(
                &release,
                tag,
                &proof.archive,
                self.requested,
                facts.as_ref().map(|(bytes, sha)| (bytes.as_slice(), *sha)),
                intent,
                super::Transfer { stop, progress },
            )?
        };
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
        let image = if self.file == INDEX {
            if proof.facts.is_none() {
                return Err(super::existing_build(tag));
            }
            Artifact::from_sdk(home, tag, &proof.archive)?
        } else {
            Artifact::from_archive(home, tag, &proof.archive)?
        };
        let facts_match = proof.facts.as_deref().is_none_or(|sha| {
            regular(RELEASE_FILE)
                && fs::read(home.join(RELEASE_FILE))
                    .is_ok_and(|bytes| sunrise_launcher_release_verify::sha256_hex(&bytes) == sha)
        });
        // A cache-only download may have reached the same signed archive as a native
        // request falling back to x64. Its preference is not part of binary identity.
        let binary_matches = self.architecture().is_none_or(|architecture| {
            if !regular(GAME_BUILD_ARCHITECTURE) {
                return false;
            }
            fs::read(home.join(GAME_BUILD_ARCHITECTURE))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<InstalledGameArchitecture>(&bytes).ok())
                .is_some_and(|actual| {
                    actual.binary == architecture.binary
                        && (intent == InstallIntent::Exact
                            || actual.requested == architecture.requested)
                })
        });
        if !facts_match || !binary_matches {
            return Err(super::existing_build(tag));
        }
        Ok(image)
    }
}

/// Verify the signed `SHA256SUMS` digest for `file` (and `sunrise-release.json`, required for
/// SDK-only installs and optional for static archives). Shared by install and SDK offer checks.
pub(crate) fn prove_release_files(
    source: &str,
    tag: &str,
    file: &str,
    require_facts: bool,
    keys: &[PublicKey],
    fetch: impl Fn(&str) -> Result<Option<Vec<u8>>, String>,
) -> Result<GameProof, String> {
    use sunrise_launcher_release_verify as verify;
    let channel = Channel::of(source);
    let sums = fetch(SUMS)?;
    let signature = fetch(if channel.is_some() {
        SIGNATURE_V2
    } else {
        SIGNATURE
    })?
    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
    let digest = |name: &str| match channel {
        Some(channel) => verify::build_digest(
            channel,
            tag,
            name,
            sums.as_deref(),
            signature.as_deref(),
            keys,
        ),
        None => verify::signed_digest(tag, name, sums.as_deref(), signature.as_deref(), keys),
    };
    let archive = digest(file)?;
    let facts = if require_facts {
        Some(digest(RELEASE_FILE)?)
    } else {
        digest(RELEASE_FILE).ok()
    };
    Ok(GameProof { archive, facts })
}

pub(in crate::backend) fn sdk_target(requested: Option<GameArchitecture>) -> Option<&'static str> {
    sdk_target_for(
        std::env::consts::OS,
        native_windows_architecture(),
        std::env::consts::ARCH,
        requested,
    )
}

/// Resolve the SDK client-slice target using the same Windows compatibility rules as player
/// archives: x64 runs natively on x64 and under emulation on Windows ARM64, both when x64 is
/// explicitly selected and as the ARM64-to-x64 fallback.
fn sdk_target_for(
    os: &str,
    windows_arch: Option<&str>,
    arch: &str,
    requested: Option<GameArchitecture>,
) -> Option<&'static str> {
    match (os, arch) {
        ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu"),
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("windows", _) => {
            let requested = requested.unwrap_or_default();
            let (primary, fallback) = game_assets_for("windows", windows_arch, arch, requested)?;
            let allows_x64 = architecture_for_asset(primary) == Some(GameArchitecture::X64)
                || fallback.and_then(architecture_for_asset) == Some(GameArchitecture::X64);
            allows_x64.then_some(crate::rust_toolchain::WINDOWS_X64_TARGET)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests;
