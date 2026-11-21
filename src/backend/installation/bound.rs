//! The archive a deferred first Play resolved before any full download.
use super::super::*;
use crate::managed_play::Artifact;
use sunrise_launcher_release_verify::{is_build, Channel, SIGNATURE, SIGNATURE_V2, SUMS};

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
            .installed_at(&version_dir(tag), tag, &self.proof)
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
        let release = Release {
            tag: tag.into(),
            name: String::new(),
            body: String::new(),
            date: String::new(),
            time: self.time.clone(),
            asset: Some((
                crate::signed::asset_url(&self.source, &self.file),
                self.size,
            )),
            prerelease: false,
            nightly: Channel::of(&self.source).map(|_| self.source.clone()),
        };
        if self.is_installed(tag, proof) {
            progress(1.0);
            return Ok(());
        }
        let facts = if let Some(sha) = proof.facts.as_deref() {
            let bytes = fetch_small(&crate::signed::asset_url(&self.source, RELEASE_FILE))?
                .ok_or_else(|| format!("{RELEASE_FILE}: signed build facts are missing"))?;
            Some((bytes, sha))
        } else {
            None
        };
        super::install_as(
            &release,
            tag,
            &proof.archive,
            self.requested,
            facts.as_ref().map(|(bytes, sha)| (bytes.as_slice(), *sha)),
            progress,
        )?;
        if !self.is_installed(tag, proof) {
            return Err(super::existing_build(tag));
        }
        Ok(())
    }

    pub(crate) fn is_installed(&self, tag: &str, proof: &GameProof) -> bool {
        self.installed_at(&version_dir(tag), tag, proof).is_ok()
    }

    fn installed_at(&self, home: &Path, tag: &str, proof: &GameProof) -> Result<Artifact, String> {
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
                .is_some_and(|actual| actual.binary == binary)
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
            download.installed_at(home, tag, &proof).is_err(),
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
            assert!(download.installed_at(home, tag, &proof).is_ok());
            assert!(direct.installed_at(home, tag, &proof).is_ok());
            fs::remove_file(&archive).unwrap();
            marker(request, Arm64);
            Artifact::record_archive(home, tag, &proof.archive).unwrap();
            assert!(
                download.installed_at(home, tag, &proof).is_err(),
                "wrong binary"
            );
            assert!(
                direct.installed_at(home, tag, &proof).is_err(),
                "wrong binary"
            );
            fs::remove_file(&archive).unwrap();
        }
    }
}
