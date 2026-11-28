//! The installed mods as rows: their names, badges, build lines and the actions a row
//! offers.

use super::*;

impl Launcher {
    /// The installed mods as rows. Uses only the index served over HTTPS this run for registry
    /// names and statuses. Never asks the network or writes.
    pub(super) fn rows(&self) -> Vec<Row> {
        let Some(snap) = &self.mods_state.snapshot else {
            return Vec::new();
        };
        let unfit_updates = snap
            .rows
            .managed
            .as_ref()
            .map_or_else(BTreeSet::new, |managed| {
                sunrise::known_unfit_updates(managed.base())
            });
        let v2 = sunrise::v2::shown_index();
        let build = snap
            .rows
            .managed
            .as_ref()
            .map(|managed| managed.base().to_owned())
            .or_else(|| self.selected_exact_build());
        let mut rows = model::rows(
            &snap.rows.entries,
            &model::World {
                v2: v2.as_ref().map(|catalog| &*catalog.index),
                v2_failure: self.mods_state.v2_failure.as_deref(),
                build: build.as_deref(),
                unfit_updates: &unfit_updates,
                enabled: &snap.rows.enabled,
                unbuilt: &snap.rows.unbuilt,
                chosen: &snap.rows.chosen,
                local: &snap.rows.local,
            },
        );
        let mut regroup = false;
        for row in &mut rows {
            let held = snap.rows.switch_held.get(&row.id);
            if let Some(reason) = held {
                row.held_for = Some(model::Hold {
                    build: crate::sheets::build_name(reason.build()),
                    reason: reason.clone(),
                    retry: snap.rows.local_holds.contains(&row.id),
                });
                row.on = false;
                if !matches!(row.need, Some(model::Need::TakenDown(_))) {
                    row.need = None;
                }
                regroup = true;
            }
        }
        // A mod the game was made without isn't waiting for a build: it waits for its author.
        let left = if snap.rows.managed.is_none() {
            self.native_left_out()
        } else {
            Vec::new()
        };
        if !left.is_empty() {
            let on = self
                .selected_instance()
                .and_then(|i| self.version_of(i))
                .map(|tag| crate::sheets::build_name(tag))
                .unwrap_or_default();
            model::mark_left_out(&mut rows, &left, &on, crate::sheets::build_name);
            regroup = true;
        }
        if regroup {
            model::sort(&mut rows);
        }
        rows
    }

    /// The installed mods `ids` as people read them: each one's name, and the newer version the
    /// registry has for it when there is one. Reads what is already in memory.
    pub(crate) fn mods_named(&self, ids: &[String]) -> Vec<(String, Option<String>)> {
        let rows = self.rows();
        ids.iter()
            .map(|id| match rows.iter().find(|r| &r.id == id) {
                Some(row) => (row.title.clone(), row.update.clone()),
                None => (id.clone(), None),
            })
            .collect()
    }

    /// The native mods turned on for the selected setup, as a build's lines name them: id,
    /// title, version and picture.
    pub(crate) fn mods_lines(&mut self) -> Vec<(String, String, String, Option<String>)> {
        self.mods_refresh();
        self.build_lines(native_status::build_mods(&self.mods_home()), true)
    }

    pub(crate) fn build_lines(
        &self,
        inputs: Vec<native_status::BuildMod>,
        selected: bool,
    ) -> Vec<(String, String, String, Option<String>)> {
        let rows = if selected { self.rows() } else { Vec::new() };
        inputs
            .into_iter()
            .map(|native_status::BuildMod { id, title, version }| {
                match rows.iter().find(|r| r.id == id && r.version == version) {
                    Some(row) => (id, row.title.clone(), version, row.icon.clone()),
                    None => (id, title, version, None),
                }
            })
            .collect()
    }

    /// Updates the installed mod `id` to the newer version the registry has, as its row does.
    pub(crate) fn update_mod(&mut self, id: &str) {
        if let Some(row) = self.rows().into_iter().find(|r| r.id == id) {
            self.update_one(&row);
        }
    }

    /// Home's inventory can finish before Installed's worker. Leave the labels blank until the
    /// current setup has confirmed rows, and rebuild them on the worker's next publication.
    pub(crate) fn home_mod_facts(&mut self, inventory: u64) -> Arc<HashMap<String, HomeModFacts>> {
        let unfit_revision = sunrise::unfit_updates_revision();
        if let Some(labels) = &self.mods_state.home_mod_labels {
            if labels.matches(inventory, unfit_revision) {
                return labels.rows.clone();
            }
        }
        self.mods_refresh();
        let ready = self
            .mods_state
            .snapshot
            .as_ref()
            .is_some_and(Snapshot::ready);
        if !ready {
            // Mid-read the labels of the last read stay: the lines under the names don't blink.
            return self
                .mods_state
                .home_mod_labels
                .as_ref()
                .map(|labels| labels.rows.clone())
                .unwrap_or_default();
        }
        let mut labels = HashMap::new();
        {
            for row in self.rows().into_iter().filter(|row| row.on) {
                let error = self.mods_state.errors.get(&row.id).map(String::as_str);
                let badge = row.manual_badge();
                let cells = model::cells(&row, error, None);
                labels.insert(
                    row.id,
                    HomeModFacts {
                        kind: cells.kind,
                        badge,
                        status: cells.status.map(|state| state.label),
                    },
                );
            }
        }
        let rows = Arc::new(labels);
        self.mods_state.home_mod_labels = Some(HomeModLabels {
            inventory,
            unfit_revision,
            rows: rows.clone(),
        });
        rows
    }

    /// Build… on a row: the one native build path. The player stays where they are: the play bar shows how far it is.
    pub(super) fn build(&mut self) {
        self.build_native();
    }
}
