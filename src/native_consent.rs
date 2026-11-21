//! One deferred native-code action. Declining never invokes its installer or builder.
use std::path::{Path, PathBuf};
use std::sync::Arc;
use sunrise_launcher::{
    backend::Settings,
    browser::{installed::Entry, mod_list, sunrise::NativeQuestion},
    native_status::{BuildInput, Registry},
    profile_txn::Identity,
};

pub(super) struct Toggle {
    pub(super) entry: Entry,
    pub(super) home: PathBuf,
    pub(super) identity: Identity,
}

pub(super) enum PendingConsent {
    /// A build's package files are being read. The Arc also identifies this request, not just
    /// its folder: two questions opened on the same profile are different requests.
    ReadingBuild(Arc<PathBuf>),
    /// A build of these packages, named when the question opened from the files it takes in.
    Build(Vec<BuildInput>),
    Toggle(Box<Toggle>),
    /// The exact batch's native inputs, including required native dependencies.
    Registry(
        NativeQuestion,
        std::sync::mpsc::Sender<Result<bool, String>>,
    ),
}

impl PendingConsent {
    pub(super) fn finish_read(&mut self, read: &Arc<PathBuf>, inputs: Vec<BuildInput>) {
        if matches!(self, Self::ReadingBuild(active) if Arc::ptr_eq(active, read)) {
            *self = Self::Build(inputs);
        }
    }

    /// Who the question is about: a mod being installed, or the packages a build takes.
    /// Build package names and provenance stay in the individual rows.
    pub(super) fn name(&self) -> String {
        match self {
            Self::ReadingBuild(_) => String::new(),
            Self::Build(inputs) => match inputs.len() {
                0 => String::new(),
                1 => crate::i18n::tr("this mod").into(),
                count => crate::i18n::trf("{count} mods", &[("count", &count.to_string())]),
            },
            Self::Toggle(toggle) => toggle.entry.title.clone(),
            Self::Registry(question, _) => crate::mods::names(&question.names),
        }
    }

    /// The question's Mods rows: one per package, so two packages never read as one name or one
    /// as two. A build's rows carry each package's id and where its file is from.
    pub(super) fn rows(&self) -> Vec<BuildRow> {
        match self {
            Self::Build(inputs) => inputs.iter().map(build_row).collect(),
            _ => Vec::new(),
        }
    }
}

pub(super) struct BuildRow {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) label: String,
    pub(super) icon: Option<String>,
}

/// A build's package as the question names it: only a file the registry lists gets its name
/// and artwork. A package put in the folder under that mod's id never reads as that mod.
fn build_row(input: &BuildInput) -> BuildRow {
    let own = [("id", input.id.as_str()), ("file", input.file.as_str())];
    let on_disk = |label| (input.id.as_str(), None, crate::i18n::trf(label, &own));
    let (name, icon, label) = match &input.registry {
        Registry::Listed { name, icon } if !name.is_empty() => (
            name.as_str(),
            icon.clone(),
            crate::i18n::trf(
                "{name} ({id}, from the Sunrise registry)",
                &[("name", name), ("id", &input.id)],
            ),
        ),
        Registry::Listed { .. } | Registry::NotListed => {
            on_disk("{id} ({file}, not from the registry)")
        }
        Registry::Revoked => on_disk("{id} ({file}, taken down)"),
        Registry::Unchecked => on_disk("{id} ({file}, not checked against the registry)"),
    };
    BuildRow {
        id: input.id.clone(),
        name: name.to_owned(),
        label,
        icon,
    }
}

/// Consume exactly one request. Toggle sends the explicit permission to its On worker;
/// a scoped Registry retry uses its existing grant. Other allows persist the permission here.
/// Cancel and Escape both use `allow = false`, and cannot return an executable action.
pub(super) fn resolve(
    pending: &mut Option<PendingConsent>,
    allow: bool,
    supported: bool,
    settings: &mut Settings,
    home: &Path,
) -> Result<Option<PendingConsent>, String> {
    if pending.is_none() {
        return Ok(None);
    }
    if allow {
        if matches!(pending, Some(PendingConsent::ReadingBuild(_))) {
            return Ok(None);
        }
        if !supported {
            return Err(crate::i18n::tr("Unavailable on this platform").into());
        }
        // The scoped second answer doesn't replace (or resurrect) the grant pinned by
        // the running attempt. The worker still checks the original revision after reply.
        match pending {
            Some(PendingConsent::Toggle(_)) => (),
            Some(PendingConsent::Registry(question, _)) if question.retry => (),
            _ => settings.allow_native_mods()?,
        }
        return Ok(pending.take());
    }
    if let Some(action) = pending.take() {
        match action {
            PendingConsent::Build(inputs) => {
                // One rewrite: a failure can never leave some declined mods on.
                let ids: Vec<&str> = inputs.iter().map(|i| i.id.as_str()).collect();
                mod_list::set_all_enabled(home, &ids, false)?;
            }
            PendingConsent::Registry(_, reply) => {
                let _ = reply.send(Ok(false));
            }
            // Toggle was only a request to turn an off row on; there is nothing to undo.
            PendingConsent::Toggle(_) | PendingConsent::ReadingBuild(_) => {}
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_build_names_a_package_as_the_registry_mod_only_when_its_file_is_the_registrys() {
        let input = |registry: Registry| BuildInput {
            id: "jei".into(),
            file: "jei.smod".into(),
            registry,
        };
        let row = |i| PendingConsent::Build(vec![i]).rows().pop().unwrap();
        let listed = row(input(Registry::Listed {
            name: "JEI".into(),
            icon: Some("https://example.com/jei.png".into()),
        }));
        assert_eq!(listed.name, "JEI");
        assert_eq!(listed.icon.as_deref(), Some("https://example.com/jei.png"));
        assert_eq!(listed.label, "JEI (jei, from the Sunrise registry)");
        assert_eq!(
            row(input(Registry::NotListed)).label,
            "jei (jei.smod, not from the registry)"
        );
        assert_eq!(
            row(input(Registry::Listed {
                name: String::new(),
                icon: None
            }))
            .label,
            "jei (jei.smod, not from the registry)"
        );
        let revoked = row(input(Registry::Revoked));
        assert_eq!(revoked.name, "jei");
        assert!(revoked.icon.is_none());
        assert_eq!(revoked.label, "jei (jei.smod, taken down)");
    }

    #[test]
    fn a_package_is_not_called_the_registrys_when_the_registry_has_not_answered() {
        let build = PendingConsent::Build(vec![BuildInput {
            id: "jei".into(),
            file: "jei.smod".into(),
            registry: Registry::Unchecked,
        }]);
        assert_eq!(build.name(), "this mod");
        assert_eq!(
            build.rows()[0].label,
            "jei (jei.smod, not checked against the registry)"
        );
    }

    #[test]
    fn a_build_lists_one_row_per_package_whatever_their_names() {
        let input = |id: &str, registry: Option<&str>| BuildInput {
            id: id.into(),
            file: format!("{id}.smod"),
            registry: registry.map_or(Registry::NotListed, |name| Registry::Listed {
                name: name.into(),
                icon: None,
            }),
        };
        // A name with a comma, the same name twice, and a package the registry doesn't list.
        let build = PendingConsent::Build(vec![
            input("maps_tools", Some("Maps, Tools")),
            input("recipes", Some("Recipes")),
            input("recipes_two", Some("Recipes")),
            input("own", None),
        ]);
        assert_eq!(
            build
                .rows()
                .into_iter()
                .map(|r| r.label)
                .collect::<Vec<_>>(),
            [
                "Maps, Tools (maps_tools, from the Sunrise registry)",
                "Recipes (recipes, from the Sunrise registry)",
                "Recipes (recipes_two, from the Sunrise registry)",
                "own (own.smod, not from the registry)",
            ]
        );
        assert_eq!(build.name(), "4 mods");
        assert_eq!(PendingConsent::Build(Vec::new()).name(), "");
        assert!(PendingConsent::Build(Vec::new()).rows().is_empty());
    }

    #[test]
    fn an_old_read_cannot_finish_a_new_consent_request_for_the_same_profile() {
        let old = Arc::new(PathBuf::from("profile"));
        let current = Arc::new(PathBuf::from("profile"));
        let mut pending = PendingConsent::ReadingBuild(current.clone());
        pending.finish_read(&old, Vec::new());
        assert!(matches!(&pending, PendingConsent::ReadingBuild(_)));
        pending.finish_read(&current, Vec::new());
        assert!(matches!(&pending, PendingConsent::Build(_)));
    }

    #[test]
    fn cancel_escape_and_allow_consume_only_the_pending_build() {
        // Settings is process-global: exercise this transition in its own isolated process.
        let input = |id: &str| BuildInput {
            id: id.into(),
            file: format!("{id}.smod"),
            registry: Registry::Unchecked,
        };
        if std::env::var_os("SUNRISE_CONSENT_TEST_CHILD").is_none() {
            let home = tempfile::tempdir().unwrap();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "native_consent::tests::cancel_escape_and_allow_consume_only_the_pending_build",
                ])
                .env("SUNRISE_CONSENT_TEST_CHILD", "1")
                .env("SUNRISE_LAUNCHER_HOME", home.path())
                .env("SUNRISE_HOME", home.path())
                .status()
                .unwrap();
            assert!(status.success());
            return;
        }
        let home = tempfile::tempdir().unwrap();
        let mut settings = Settings::default();
        // Decline returns no dispatchable action: no installer or builder can run.
        // The build layer separately tests its HTTP backstop with a mock proxy.
        for _ in ["Cancel", "Escape"] {
            let mut pending = Some(PendingConsent::Build(vec![input("sample")]));
            assert!(
                resolve(&mut pending, false, true, &mut settings, home.path())
                    .unwrap()
                    .is_none()
            );
            assert!(pending.is_none());
            assert!(!mod_list::is_enabled(home.path(), "sample"));
            assert!(settings.native_mods_allowed_at.is_none());
        }
        mod_list::set_enabled(home.path(), "sample", true).unwrap();
        mod_list::set_enabled(home.path(), "second", true).unwrap();
        mod_list::set_enabled(home.path(), "unrelated", true).unwrap();
        let mut pending = Some(PendingConsent::Build(vec![
            input("sample"),
            input("second"),
        ]));
        resolve(&mut pending, false, true, &mut settings, home.path()).unwrap();
        assert!(
            !mod_list::is_enabled(home.path(), "sample")
                && !mod_list::is_enabled(home.path(), "second"),
            "NoConsent cancellation turns every one of the build's mods off"
        );
        assert!(mod_list::is_enabled(home.path(), "unrelated"));
        let mut unsupported = Some(PendingConsent::Build(vec![input("sample")]));
        assert!(resolve(&mut unsupported, true, false, &mut settings, home.path()).is_err());
        assert!(unsupported.is_some());
        assert!(Settings::load().native_mods_allowed_at.is_none());
        if sunrise_launcher::rust_toolchain::host().is_some() {
            let mut pending = Some(PendingConsent::Build(vec![input("sample")]));
            let action = resolve(&mut pending, true, true, &mut settings, home.path())
                .unwrap()
                .unwrap();
            assert!(Settings::load().native_mods_allowed_at.is_some());
            match action {
                PendingConsent::Build(inputs) => {
                    mod_list::set_enabled(home.path(), &inputs[0].id, true).unwrap()
                }
                _ => panic!("wrong pending action"),
            }
            assert!(mod_list::is_enabled(home.path(), "sample"));
            assert!(
                resolve(&mut pending, true, true, &mut settings, home.path())
                    .unwrap()
                    .is_none(),
                "the action cannot execute twice"
            );
        }
    }
}
