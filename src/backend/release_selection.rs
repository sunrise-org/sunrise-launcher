//! Resolve a published release's channel or exact installed version.
use super::{Release, RELEASE_FILE};
use chrono::NaiveDate;
use std::{cmp::Ordering, collections::BTreeMap};
use sunrise_launcher_release_verify::Channel;

/// The snapshot tagged `tag` (`snapshot-<date>-<commit>`, docs/release-flow.md "Snapshot"), looked
/// up by its tag alone, or nothing for any other tag. No list or update ever offers a snapshot, so a
/// developer names it (`sunrise-launcher install <tag>`); it installs as its tag, which its title
/// must name and its SHA256SUMS.v2.sig must be signed for (`backend::install`).
pub(super) fn snapshot(tag: &str) -> Result<Vec<Release>, String> {
    if Channel::of(tag) != Some(Channel::Snapshot) {
        return Ok(Vec::new());
    }
    if !sunrise_launcher_release_verify::is_build(Channel::Snapshot, tag) {
        return Err(format!(
            "{tag} is not a snapshot's snapshot-<date>-<commit>."
        ));
    }
    let release = crate::github::json_optional(&format!(
        "{}/repos/{}/releases/tags/{tag}",
        crate::github::API,
        super::GAME_REPO
    ))?
    .ok_or_else(|| format!("No Sunrise snapshot is tagged {tag}."))?;
    let settings = super::Settings::load();
    let assets = super::game_assets_for(
        std::env::consts::OS,
        super::native_windows_architecture(),
        std::env::consts::ARCH,
        settings.game_architecture,
    );
    let (asset, fallback) = assets
        .map_or((super::ASSET, super::GAME_FALLBACK), |(asset, fallback)| {
            (Some(asset), fallback)
        });
    Ok(
        snapshot_in(&serde_json::json!([release]), tag, asset, fallback)
            .into_iter()
            .collect(),
    )
}

/// The snapshot tagged `tag` in `list` (GitHub's answer for it) with `asset`, else `fallback`.
fn snapshot_in(
    list: &serde_json::Value,
    tag: &str,
    asset: Option<&str>,
    fallback: Option<&str>,
) -> Option<Release> {
    let keep = |tag: &str| Channel::of(tag) == Some(Channel::Snapshot);
    let mut found = super::parse_releases(list, asset, keep)
        .into_iter()
        .chain(super::parse_releases(list, fallback, keep))
        .filter(|r| r.tag == tag);
    let mut snapshot = found.next()?;
    if snapshot.asset.is_none() {
        snapshot.asset = found.find_map(|r| r.asset);
    }
    // Its GitHub tag is its version: its files are there, and it installs and verifies as that.
    snapshot.prerelease = true;
    snapshot.nightly = Some(snapshot.tag.clone());
    Some(snapshot)
}

pub(super) fn by_tag<'a>(releases: &'a [Release], tag: &str) -> Option<&'a Release> {
    releases
        .iter()
        .find(|release| release.tag == tag || release.nightly.as_deref() == Some(tag))
}

/// The signed build name covers the UTC build day, not a mutable GitHub publication time.
struct SignedIdentity {
    commit: String,
    day: NaiveDate,
}

impl SignedIdentity {
    fn read(release: &Release) -> Option<Self> {
        let source = release.nightly.as_deref()?;
        let bytes = super::fetch_small(&crate::signed::asset_url(source, RELEASE_FILE)).ok()??;
        let digest = crate::signed::build_digest(
            source,
            &release.tag,
            RELEASE_FILE,
            &sunrise_launcher_release_verify::trusted_keys_for(super::GAME_REPO),
            super::fetch_small,
        )
        .ok()?;
        if sunrise_launcher_release_verify::sha256_hex(&bytes) != digest {
            return None;
        }
        Self::decode(&release.tag, &bytes)
    }

    fn decode(tag: &str, verified: &[u8]) -> Option<Self> {
        #[derive(serde::Deserialize)]
        struct Facts {
            contract: u32,
            build: String,
            commit: String,
        }
        let facts: Facts = serde_json::from_slice(verified).ok()?;
        let channel = Channel::of(tag)?;
        if facts.contract != 1
            || facts.build != tag
            || !sunrise_launcher_release_verify::is_build(channel, tag)
            || !full_commit(&facts.commit)
        {
            return None;
        }
        let (_, suffix) = tag.split_once('-')?;
        let (day, abbreviated) = suffix.rsplit_once('-')?;
        if !facts.commit.starts_with(abbreviated) {
            return None;
        }
        Some(Self {
            commit: facts.commit,
            day: NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()?,
        })
    }

    fn compare(&self, other: &Self) -> Option<Ordering> {
        if self.commit == other.commit {
            return Some(Ordering::Equal);
        }
        let order = self.day.cmp(&other.day);
        // The signed release contract has no sequence within a UTC day.
        (order != Ordering::Equal).then_some(order)
    }
}

fn full_commit(commit: &str) -> bool {
    commit.len() == 40
        && commit
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(crate) struct WorldBuildOrder {
    target: Option<SignedIdentity>,
    releases: Vec<Release>,
    comparisons: BTreeMap<String, Option<Ordering>>,
}

impl WorldBuildOrder {
    pub(crate) fn for_target(tag: &str) -> Self {
        let mut releases = super::release_list()
            .map(|list| world_order_releases(&list))
            .unwrap_or_default();
        if Channel::of(tag) == Some(Channel::Snapshot) && !releases.iter().any(|r| r.tag == tag) {
            releases.extend(snapshot(tag).unwrap_or_default());
        }
        let target = releases
            .iter()
            .find(|r| r.tag == tag)
            .and_then(SignedIdentity::read);
        Self {
            target,
            releases,
            comparisons: BTreeMap::new(),
        }
    }

    /// Abbreviated tags are only lookup hints, never the saved world's identity.
    pub(crate) fn compare_saved(&mut self, commit: &str) -> Option<Ordering> {
        self.compare_saved_with(commit, SignedIdentity::read)
    }

    fn compare_saved_with(
        &mut self,
        commit: &str,
        read_signed: impl FnMut(&Release) -> Option<SignedIdentity>,
    ) -> Option<Ordering> {
        let target = self.target.as_ref()?;
        if commit == target.commit {
            return Some(Ordering::Equal);
        }
        if !full_commit(commit) {
            return None;
        }
        *self.comparisons.entry(commit.into()).or_insert_with(|| {
            let mut signed = self
                .releases
                .iter()
                .filter(|r| {
                    r.nightly.is_some()
                        && r.tag
                            .rsplit_once('-')
                            .is_some_and(|(_, hint)| commit.starts_with(hint))
                })
                .filter_map(read_signed)
                .filter(|identity| identity.commit == commit);
            let saved = signed.next()?;
            if signed.any(|identity| identity.day != saved.day) {
                return None;
            }
            saved.compare(target)
        })
    }
}

fn world_order_releases(list: &serde_json::Value) -> Vec<Release> {
    let mut releases = super::game_releases(list);
    // Snapshots are excluded from the picker and update feed. Their signed metadata
    // still proves which game last saved a world, including against a Nightly target.
    releases.extend(
        super::parse_releases(list, None, |tag| {
            sunrise_launcher_release_verify::is_build(Channel::Snapshot, tag)
        })
        .into_iter()
        .map(|mut snapshot| {
            snapshot.nightly = Some(snapshot.tag.clone());
            snapshot
        }),
    );
    releases
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::game_releases_with;

    #[test]
    fn a_saved_snapshot_gets_signed_order_against_a_nightly_target() {
        let saved = "b".repeat(40);
        let tag = "snapshot-2026-11-19-bbbbbbb";
        let raw = serde_json::json!([{
            "tag_name": tag,
            "name": "Snapshot",
            "published_at": "2026-11-19T00:00:00Z",
            "prerelease": true,
            "assets": []
        }]);
        assert!(super::super::game_releases(&raw).is_empty());
        let mut order = WorldBuildOrder {
            target: Some(SignedIdentity {
                commit: "a".repeat(40),
                day: NaiveDate::from_ymd_opt(2026, 11, 18).unwrap(),
            }),
            releases: world_order_releases(&raw),
            comparisons: BTreeMap::new(),
        };
        assert_eq!(
            order.compare_saved_with(&saved, |release| {
                assert_eq!(release.nightly.as_deref(), Some(tag));
                let signed = serde_json::json!({ "contract": 1, "build": tag, "commit": saved });
                SignedIdentity::decode(&release.tag, signed.to_string().as_bytes())
            }),
            Some(Ordering::Greater)
        );
    }

    #[test]
    fn world_order_uses_full_signed_identity_and_has_no_order_within_one_day() {
        let decode = |build: &str, commit: &str| {
            SignedIdentity::decode(
                build,
                &serde_json::to_vec(&serde_json::json!({
                    "contract": 1, "build": build, "commit": commit,
                }))
                .unwrap(),
            )
        };
        let a = &"a".repeat(40);
        let b = &"b".repeat(40);
        let first = decode("nightly-2026-11-18-aaaaaaa", a).unwrap();
        let same_day = decode("snapshot-2026-11-18-bbbbbbb", b).unwrap();
        let newer = decode("nightly-2026-11-19-bbbbbbb", b).unwrap();
        assert_eq!(first.compare(&same_day), None);
        assert_eq!(first.compare(&newer), Some(Ordering::Less));
        assert_eq!(newer.compare(&same_day), Some(Ordering::Equal));
        assert!(decode("nightly-2026-11-18-aaaaaaa", "aaaaaaa").is_none());
        assert!(decode("nightly-2026-11-18-aaaaaaa", b).is_none());
    }

    fn published(channel: &str) -> Vec<Release> {
        game_releases_with(
            &serde_json::json!([{
                "tag_name": channel,
                "name": "Nightly 2026-11-10 · 9f13812",
                "published_at": "2026-11-10T17:07:46Z",
                "prerelease": true,
                "assets": [{"name": "sunrise-linux-x86_64.tar.gz", "size": 123}]
            }]),
            Some("sunrise-linux-x86_64.tar.gz"),
            None,
        )
    }

    #[test]
    fn channel_and_exact_version_select_the_same_published_build() {
        let channel = "nightly";
        let releases = published(channel);
        for requested in [channel, "nightly-2026-11-10-9f13812"] {
            let release = by_tag(&releases, requested).unwrap();
            assert_eq!(release.tag, "nightly-2026-11-10-9f13812");
            assert_eq!(release.nightly.as_deref(), Some(channel));
            assert_eq!(
                    release.asset,
                    Some((format!("https://github.com/sunrise-org/sunrise/releases/download/{channel}/sunrise-linux-x86_64.tar.gz"), 123))
                );
        }
    }

    #[test]
    fn a_snapshot_is_found_by_its_tag_and_installs_as_it() {
        const SNAPSHOT: &str = "snapshot-2026-11-13-bbbbbbb";
        let (arm, x64) = ("sunrise-windows-arm64.zip", "sunrise-windows-x64.zip");
        let list = serde_json::json!([{
            "tag_name": SNAPSHOT,
            "name": "Snapshot 2026-11-13 · bbbbbbb",
            "published_at": "2026-11-13T10:00:00Z",
            "prerelease": true,
            "assets": [{"name": x64, "size": 1}]
        }]);
        let found = snapshot_in(&list, SNAPSHOT, Some(arm), Some(x64)).unwrap();
        assert_eq!(found.tag, SNAPSHOT);
        assert_eq!(found.nightly.as_deref(), Some(SNAPSHOT));
        assert!(found.prerelease);
        assert!(found
            .asset
            .unwrap()
            .0
            .ends_with(&format!("/{SNAPSHOT}/{x64}")));
        assert_eq!(
            crate::signed::title_version(SNAPSHOT, &found.name).as_deref(),
            Some(SNAPSHOT),
            "what install() checks before verifying"
        );
        assert!(snapshot_in(&list, "snapshot-2026-11-12-aaaaaaa", Some(x64), None).is_none());
        let nightly = serde_json::json!([{"tag_name": "nightly", "name": "Nightly 2026-11-13 · bbbbbbb",
                                           "assets": [{"name": x64, "size": 1}]}]);
        assert!(snapshot_in(&nightly, "nightly", Some(x64), None).is_none());
        // Any other tag is not looked up; a malformed snapshot tag is refused before any request.
        assert_eq!(snapshot("nightly").unwrap().len(), 0);
        assert_eq!(snapshot("v0.2.81").unwrap().len(), 0);
        assert!(snapshot("snapshot-../../x").is_err());
    }

    #[test]
    fn an_unknown_tag_never_falls_back_to_an_available_build() {
        assert!(published("nightly-2026-11-10").is_empty());
        let releases = published("nightly");
        assert!(by_tag(&releases, "nightly-2026-11-09").is_none());
        assert!(by_tag(&releases, "nightly-2026-11-10-deadbee").is_none());
        assert!(by_tag(&[], "nightly").is_none());
    }
}
