//! Nightly and Dev source-installed project state (retained for installed source mods until cutover).
use super::discover::install_fact;
use super::ui::*;
use super::*;
use std::time::{Duration, SystemTime};
pub(super) use sunrise_launcher::browser::sunrise::sources::Source;
use sunrise_launcher::browser::sunrise::sources::{self, Catalogue};

mod collection;
mod jobs;
mod project;

pub(super) use jobs::Event;
pub(super) use project::{short_sha, source_error};

const FRESH: Duration = Duration::from_secs(600);
pub(super) const SOURCE_ID_CONFLICT: &str =
    crate::i18n::tr_key("Another installed mod uses this ID.");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SourcePresence<'a> {
    Missing,
    Installed {
        origin: &'a sunrise_launcher::github_mods::Origin,
        file: &'a str,
    },
    Conflict,
}

impl InstalledRows {
    fn source_project<'a>(
        &self,
        id: &str,
        file: &str,
        catalogue: Option<&'a Catalogue>,
    ) -> Option<&'a Source> {
        let source = find_source(catalogue?, id)?;
        match self.source_presence(source) {
            SourcePresence::Installed { file: owned, .. } if owned == file => Some(source),
            _ => None,
        }
    }

    fn source_presence(&self, source: &Source) -> SourcePresence<'_> {
        let id = source.id();
        if self
            .entries
            .iter()
            .any(|entry| sunrise::mod_id(entry) == Some(id))
        {
            return SourcePresence::Conflict;
        }
        let mut local = self.local.iter().filter(|row| row.id == id);
        let Some(row) = local.next() else {
            return SourcePresence::Missing;
        };
        if local.next().is_some() {
            return SourcePresence::Conflict;
        }
        let Some(origin) = &row.github else {
            return SourcePresence::Conflict;
        };
        if source_matches_origin(source, origin) {
            SourcePresence::Installed {
                origin,
                file: &row.file,
            }
        } else {
            SourcePresence::Conflict
        }
    }
}

fn find_source<'a>(catalogue: &'a Catalogue, id: &str) -> Option<&'a Source> {
    let i = catalogue
        .sources
        .binary_search_by(|source| source.id().cmp(id))
        .ok()?;
    Some(&catalogue.sources[i])
}

fn listing_for_origin<'a>(
    catalogue: &'a Catalogue,
    origin: &sunrise_launcher::github_mods::Origin,
) -> Option<&'a Source> {
    let source = find_source(catalogue, &origin.id)?;
    source_matches_origin(source, origin).then_some(source)
}

fn source_matches_origin(source: &Source, origin: &sunrise_launcher::github_mods::Origin) -> bool {
    source.id() == origin.id
        && origin.owner.eq_ignore_ascii_case(source.owner())
        && origin.source.as_ref().is_some_and(|receipt| {
            receipt.repo_id == source.repo_id() && receipt.manifest == source.manifest()
        })
}

#[derive(Default)]
pub(super) struct State {
    catalogue: Option<Catalogue>,
    reading: bool,
    pub(super) open: Option<jobs::Open>,
    detail_request: u64,
    pub(super) work: jobs::Work,
    pub(super) collection: Option<collection::CollectionOpen>,
}

impl Launcher {
    pub(super) fn discover_target_name(&self) -> String {
        self.selected_instance()
            .and_then(|instance| self.profile_target(instance))
            .map_or_else(
                || tr("No game build selected").into(),
                |tag| crate::sheets::build_name(&tag),
            )
    }

    pub(crate) fn read_nightly_catalogue(&mut self, force: bool) {
        let n = &mut self.mods_state.nightly;
        if n.reading
            || (!force
                && n.catalogue
                    .as_ref()
                    .is_some_and(|c| c.checked_at.elapsed().is_ok_and(|time| time < FRESH)))
        {
            return;
        }
        n.reading = true;
        let tx = self.mods_state.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(super::Event::Nightly(Event::Catalogue(sources::catalogue(
                force,
            ))));
        });
    }

    pub(super) fn installed_source_project(&self, id: &str, file: &str) -> Option<&Source> {
        self.discover_installed()?.source_project(
            id,
            file,
            self.mods_state.nightly.catalogue.as_ref(),
        )
    }

    pub(crate) fn source_listing(
        &self,
        origin: &sunrise_launcher::github_mods::Origin,
    ) -> Option<&Source> {
        listing_for_origin(self.mods_state.nightly.catalogue.as_ref()?, origin)
    }

    pub(super) fn source_collection_for(
        &self,
        source: &Source,
    ) -> Option<&sunrise_launcher_smod::index::Collection> {
        self.mods_state
            .nightly
            .catalogue
            .as_ref()?
            .collections
            .iter()
            .find(|collection| {
                collection
                    .members
                    .iter()
                    .any(|member| member.id == source.id())
            })
    }

    fn source_artwork(
        &self,
        source: &Source,
        commit: Option<&sources::SourceCommit>,
        allow_fetch: bool,
    ) -> (Option<PathBuf>, Option<sources::SourceIcon>) {
        let work = &self.mods_state.nightly.work;
        let commit = commit.or_else(|| {
            work.release_pins
                .get(source.id())
                .filter(|pin| pin.source() == source)
        });
        let icon = sources::SourceIcon::new(source, commit, allow_fetch);
        let loaded = icon.as_ref().and_then(|icon| work.icons.get(icon));
        let picture = loaded.cloned().flatten();
        (picture, icon.filter(|_| loaded.is_none()))
    }

    pub(crate) fn source_tile(
        &self,
        source: &Source,
        commit: Option<&sources::SourceCommit>,
        allow_fetch: bool,
        package: Option<std::sync::Arc<gpui::Image>>,
        side: f32,
        cx: &Context<Self>,
    ) -> Div {
        let (listed, load) = self.source_artwork(source, commit, allow_fetch);
        let picture = mod_artwork(listed, package);
        tile(picture, source.id(), source.name(), side)
            .relative()
            .children(
                load.map(|icon| when_visible(cx, move |this| this.request_nightly_icon(icon))),
            )
    }

    fn nightly_build_revision(&self) -> Option<String> {
        backend::build_revision(&self.profile_target(self.selected_instance()?)?)
    }
}

fn when_visible(
    cx: &Context<Launcher>,
    load: impl FnOnce(&mut Launcher) + 'static,
) -> impl IntoElement + 'static {
    let entity = cx.entity().downgrade();
    gpui::canvas(
        move |bounds, window, cx| {
            if window.content_mask().bounds.intersects(&bounds) {
                cx.defer(move |cx| {
                    entity.update(cx, |this, _| load(this)).ok();
                });
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0()
}

fn made_for_build(profile: Option<&str>, pin: Option<&str>) -> bool {
    // The manifest validated a full pin; the profile returned its full commit or a valid prefix.
    profile
        .zip(pin)
        .is_some_and(|(profile, pin)| pin.starts_with(profile))
}

pub(super) fn installed_source(row: &Row) -> Option<&sunrise_launcher::github_mods::Origin> {
    row.local
        .as_ref()?
        .github
        .as_ref()
        .filter(|origin| origin.source.is_some() && !origin.is_manual())
}

pub(super) fn source_provenance(origin: &sunrise_launcher::github_mods::Origin) -> String {
    if let Some(path) = origin.local_folder_path() {
        return trf("Source · {path}", &[("path", path)]);
    }
    let repo = format!("{}/{}", origin.owner, origin.repo);
    trf(
        "Source · {repo} @ {sha}",
        &[("repo", &repo), ("sha", short_sha(&origin.commit))],
    )
}

fn source_time(time: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(time)
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn receipt_fixture() -> (Source, model::Local) {
        let source = Source::for_origin(
            "lantern",
            "Fixture/Lantern",
            42,
            "sunrise-mod.toml",
            sources::Tracker::Branch("main".into()),
        )
        .unwrap();
        let origin = sunrise_launcher::github_mods::Origin {
            id: "lantern".into(),
            version: "1.0.0".into(),
            owner: "fixture".into(),
            repo: "Lantern".into(),
            commit: "1".repeat(40),
            sha256: "0".repeat(64),
            tag: None,
            asset: None,
            added_from: Some(sunrise_launcher::github_mods::AddedFrom::Discover),
            source: Some(sunrise_launcher::github_mods::SourceOrigin {
                repo_id: 42,
                manifest: "sunrise-mod.toml".into(),
                track: sunrise_launcher::github_mods::SourceTrack::Branch("main".into()),
                published_at: None,
            }),
        };
        let local = model::Local {
            id: origin.id.clone(),
            version: origin.version.clone(),
            name: "Lantern".into(),
            file: "sunrise-native-mods/lantern.smod".into(),
            github: Some(origin),
            native: true,
            served_by: None,
            unreadable: false,
        };
        (source, local)
    }

    #[test]
    fn a_home_row_without_a_package_icon_uses_its_source_list_icon() {
        use crate::home_mods::Picture;
        use sunrise_launcher_smod::{index, manifest::Package};

        let home = tempfile::tempdir().unwrap();
        let (_, local) = receipt_fixture();
        let archive = Package::write_files(
            &format!(
                "id='lantern'\nname='Lantern'\nversion='1.0.0'\napi='{}'\n[native]\ngame='26.3'\nsunrise='{}'\n",
                sunrise_launcher_smod::manifest::API,
                "1".repeat(40),
            ),
            &[
                ("Cargo.toml", b"[package]\nname='lantern'\nversion='1.0.0'\n"),
                ("src/lib.rs", b"pub fn hooks() {}\n"),
            ],
        )
        .unwrap();
        let path = home.path().join(local.file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &archive).unwrap();
        let mut origin = local.github.unwrap();
        origin.sha256 = index::sha256(&archive);
        std::fs::write(
            path.with_extension("origin.json"),
            serde_json::to_vec(&origin).unwrap(),
        )
        .unwrap();
        let source = serde_json::from_value(serde_json::json!({
            "id": "lantern", "name": "Lantern", "summary": "Lights the way.",
            "repo": "Fixture/Lantern", "repo_id": 42, "manifest": "sunrise-mod.toml",
            "track": {"branch": "main"}, "icon": "icons/lantern.png"
        }))
        .unwrap();
        let catalogue = Catalogue {
            sources: vec![source].into(),
            collections: Vec::new().into(),
            saved_at: None,
            checked_at: SystemTime::UNIX_EPOCH,
            error: None,
        };

        let rows = crate::home_mods::read(home.path());
        let [row] = rows.as_slice() else {
            panic!("expected one mod on Home");
        };
        let Picture::Local {
            package: None,
            source: Some(origin),
        } = &row.picture
        else {
            panic!("Home should retain the source receipt even without an embedded icon");
        };
        let listing = listing_for_origin(&catalogue, origin).unwrap();
        assert_eq!(
            sources::SourceIcon::new(listing, None, true),
            Some(sources::SourceIcon::pages("icons/lantern.png", true)),
        );
    }

    #[test]
    fn an_older_commit_of_the_same_listing_is_installed() {
        let (listing, local) = receipt_fixture();
        let rows = InstalledRows {
            local: vec![local],
            ..Default::default()
        };
        let origin = rows.local[0].github.as_ref().unwrap();
        // The installed commit is the receipt's commit, independent of where the branch is now.
        assert_eq!(origin.commit, "1".repeat(40));
        assert_eq!(
            rows.source_presence(&listing),
            SourcePresence::Installed {
                origin,
                file: &rows.local[0].file
            }
        );
    }

    #[test]
    fn installed_project_only_promotes_the_selected_archive_to_a_listed_source() {
        let (listing, local) = receipt_fixture();
        let file = local.file.clone();
        let mut rows = InstalledRows {
            local: vec![local.clone()],
            ..Default::default()
        };
        let mut catalogue = Catalogue {
            sources: Vec::new().into(),
            collections: Vec::new().into(),
            saved_at: None,
            checked_at: SystemTime::UNIX_EPOCH,
            error: Some("Offline".into()),
        };
        // Neither a pending reply nor a failed reply without a listing replaces local details.
        assert_eq!(rows.source_project(&local.id, &file, None), None);
        assert_eq!(
            rows.source_project(&local.id, &file, Some(&catalogue)),
            None
        );
        catalogue.sources = vec![listing.clone()].into();
        catalogue.error = None;
        assert_eq!(
            rows.source_project(&local.id, &file, Some(&catalogue)),
            Some(&listing)
        );
        assert_eq!(
            rows.source_project(&local.id, "different.smod", Some(&catalogue)),
            None
        );
        // A duplicate forces Installed back to the selected archive's own details, while
        // Discover still sees a conflict and cannot install over either package.
        rows.local.push(model::Local {
            file: "copy.smod".into(),
            github: None,
            ..local.clone()
        });
        assert_eq!(
            rows.source_project(&local.id, &file, Some(&catalogue)),
            None
        );
        assert_eq!(rows.source_presence(&listing), SourcePresence::Conflict);
    }

    #[test]
    fn an_occupied_id_without_the_listings_receipt_is_a_conflict() {
        let (listing, local) = receipt_fixture();
        let mut rows = InstalledRows {
            local: vec![local],
            ..Default::default()
        };
        let matches = rows.local[0].github.clone().unwrap();
        let mut other = matches.clone();
        other.source.as_mut().unwrap().repo_id += 1;
        rows.local[0].github = Some(other);
        assert_eq!(rows.source_presence(&listing), SourcePresence::Conflict);
        let mut other = matches.clone();
        other.owner = "DifferentOwner".into();
        rows.local[0].github = Some(other);
        assert_eq!(rows.source_presence(&listing), SourcePresence::Conflict);
        let mut other = matches.clone();
        other.source.as_mut().unwrap().manifest = "other/sunrise-mod.toml".into();
        rows.local[0].github = Some(other);
        assert_eq!(rows.source_presence(&listing), SourcePresence::Conflict);
        rows.local[0].github = Some(sunrise_launcher::github_mods::Origin {
            source: None,
            ..matches
        });
        assert_eq!(rows.source_presence(&listing), SourcePresence::Conflict);
        rows.local.clear();
        assert_eq!(rows.source_presence(&listing), SourcePresence::Missing);
    }

    #[test]
    fn only_a_matching_full_build_or_valid_profile_prefix_gets_the_badge() {
        let pin = "9425651aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let other = "9425651bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        assert!(made_for_build(Some(pin), Some(pin)));
        assert!(made_for_build(Some("9425651"), Some(pin)));
        assert!(!made_for_build(Some(other), Some(pin)));
        assert!(!made_for_build(Some("1234567"), Some(pin)));
        assert!(!made_for_build(Some(pin), None));
        assert!(!made_for_build(None, Some(pin)));
    }
}
