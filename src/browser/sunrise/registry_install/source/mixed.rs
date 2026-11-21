//! Two independently published batches, with one review of the exact proposed world changes.
use super::*;
use crate::worlds::WorldRisk;

type JointReview<'a> = dyn Fn(&ChangeImpact, Option<&ChangeImpact>) -> bool + 'a;

pub struct CombinedUpdate {
    pub registry: Result<InstalledBatch, InstallError>,
    pub sources: Result<(), SourceError>,
}

/// Run registry work first. Only its verified publication can advance the source batch's input
/// evidence. Neither a later state read nor accepting a review authorizes a different generation.
#[allow(clippy::too_many_arguments)]
pub fn update_registry_and_sources(
    instance: &Instance,
    identity: Identity,
    requested: Vec<String>,
    prepared: &mut [PreparedSource],
    choose: impl Fn() -> Result<FirstBuild, String>,
    registry_hooks: InstallHooks<'_>,
    source_hooks: InstallHooks<'_>,
    cancellation: &native_mods::Cancellation,
    authorize_source: &dyn Fn(&[PreparedSource]) -> Result<bool, String>,
    review: &JointReview<'_>,
    start_sources: impl FnOnce(),
) -> CombinedUpdate {
    update_registry_and_sources_with_registry(
        instance,
        identity,
        requested,
        prepared,
        choose,
        registry_hooks,
        source_hooks,
        cancellation,
        authorize_source,
        review,
        start_sources,
        fetch_with_official,
    )
}

#[allow(clippy::too_many_arguments)]
pub(in crate::browser::sunrise::registry_install) fn update_registry_and_sources_with_registry(
    instance: &Instance,
    identity: Identity,
    requested: Vec<String>,
    prepared: &mut [PreparedSource],
    choose: impl Fn() -> Result<FirstBuild, String>,
    registry_hooks: InstallHooks<'_>,
    source_hooks: InstallHooks<'_>,
    cancellation: &native_mods::Cancellation,
    authorize_source: &dyn Fn(&[PreparedSource]) -> Result<bool, String>,
    review: &JointReview<'_>,
    start_sources: impl FnOnce(),
    fetch_registry: impl Fn(
        &str,
        bool,
    ) -> Result<(index::Index, String, Option<OfficialRegistry>), String>,
) -> CombinedUpdate {
    let mut mixed = MixedSources {
        instance,
        identity,
        prepared,
        review,
        cancellation,
        checked: None,
        denied: false,
        published: None,
    };
    let registry = registry_edit(
        instance,
        identity,
        || Action::Update(UpdateRequest::Bulk(requested), &|_| false),
        &choose,
        registry_hooks,
        fetch_registry,
        Some(&mut mixed),
    );
    if mixed.denied {
        return CombinedUpdate {
            registry,
            sources: Err(SourceError::Cancelled),
        };
    }
    start_sources();
    let sources = (|| {
        let reviewed = match mixed.checked {
            Some(Ok(impacts)) => Some(impacts),
            Some(Err(error)) => return Err(error),
            // With no registry candidate the source worker shows its own update review.
            // A fresh add would skip that review and is not part of Update all.
            None if mixed.prepared.iter().all(PreparedSource::is_update) => None,
            None => return Err(changed_source_target()),
        };
        // If registry publication failed, keep the original pin. The source edit must
        // still prove its graph and match any world evidence already reviewed.
        if let Some((active, inputs)) = mixed.published {
            for source in &mut *mixed.prepared {
                source.before = inputs;
                source.active = Some(active.clone());
            }
        }
        let changed = std::cell::Cell::new(false);
        let result = apply_sources(
            instance,
            identity,
            mixed.prepared,
            choose,
            SourceHooks {
                install: source_hooks,
                cancellation,
                authorize: authorize_source,
                approve_worlds: &|impact| {
                    if let Some(checked) = &reviewed {
                        let same = impact.matches_world_review(checked);
                        changed.set(!same);
                        same
                    } else {
                        review(impact, None)
                    }
                },
            },
        );
        if changed.get() {
            Err(changed_source_target())
        } else {
            result
        }
    })();
    CombinedUpdate { registry, sources }
}

pub(in crate::browser::sunrise::registry_install) struct MixedSources<'a> {
    instance: &'a Instance,
    identity: Identity,
    prepared: &'a mut [PreparedSource],
    review: &'a JointReview<'a>,
    cancellation: &'a native_mods::Cancellation,
    checked: Option<Result<Vec<WorldRisk>, SourceError>>,
    denied: bool,
    published: Option<(managed_play::Active, [u8; 32])>,
}

impl MixedSources<'_> {
    pub(in crate::browser::sunrise::registry_install) fn combine(
        &mut self,
        impact: &ChangeImpact,
        edit: &managed_play::Edit<'_>,
        original: &Path,
        registry: &Path,
        registry_inputs: [u8; 32],
    ) -> bool {
        let source_impact = (|| {
            let before = managed_play::inputs(original, None)?;
            let base = FirstBuild::installed(edit.base())?;
            if self.prepared.iter().any(|p| {
                p.identity != self.identity
                    || p.before != before
                    || p.base != edit.base()
                    || p.active.as_ref() != edit.current()
                    || !p.checked_base.same_cached_base(&base)
                    || !p.is_update()
            }) {
                return Err(changed_source_target());
            }
            let entries = self
                .prepared
                .iter()
                .map(PreparedSource::entry)
                .collect::<Vec<_>>();
            // Overlay the source replacements on the exact registry postimage without
            // changing it. Registry publication cannot authorize a source predecessor.
            let stage = registry
                .parent()
                .expect("candidate has a journal directory");
            let combined = tempfile::tempdir_in(stage).map_err(|e| at(stage, e))?;
            managed_play::copy_install_inputs(registry, combined.path())?;
            if managed_play::inputs(combined.path(), None)? != registry_inputs {
                return Err(changed_source_target());
            }
            let origins = github::predecessors(combined.path(), &entries)?;
            let staged = github::stage_entries(
                combined.path(),
                &entries,
                &origins,
                edit.current(),
                edit.base(),
            )?;
            check_source_set(combined.path(), &staged.off, edit.base(), self.cancellation)?;
            Ok(github::source_review(
                &self.instance.home,
                registry,
                combined.path(),
                &origins,
                &entries,
            )?
            .0)
        })();
        // A source that could not be proved isn't a proposal. Still review the
        // registry batch and leave its outcome independent of the source refusal.
        let allowed = (self.review)(impact, source_impact.as_ref().ok());
        self.checked = Some(source_impact.map(ChangeImpact::into_evidence));
        self.denied = !allowed;
        allowed
    }

    pub(in crate::browser::sunrise::registry_install) fn published(
        &mut self,
        active: managed_play::Active,
        inputs: [u8; 32],
    ) {
        self.published = Some((active, inputs));
    }
}
