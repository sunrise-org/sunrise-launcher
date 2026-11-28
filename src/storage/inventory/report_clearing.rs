//! The inventory's report, and clearing its rows.
use super::*;

impl Storage {
    /// A read-only count, or a manual Clear of one row or all of them under the build lease.
    pub fn report(&self, clear: Option<Category>, all: bool) -> Result<Report, Error> {
        if clear.is_none() && !all {
            return self.report_counting(|_| {});
        }
        // Only manual Clear takes the build lease; its selected units take their own
        // Cargo locks or runtime claim at removal, with no whole-target sizing preflight.
        let selection = if all { None } else { clear };
        let result = (|| {
            let _lease = self.build_lease()?;
            let entries = self.entries_until(&Cancel::default(), selection)?;
            self.apply_retaining(entries, clear, all, true)
        })();
        result.map_err(redact_error)
    }

    #[cfg(test)]
    pub(super) fn apply(
        &self,
        entries: Vec<Entry>,
        clear: Option<Category>,
        all: bool,
    ) -> Result<Report, Error> {
        self.apply_retaining(entries, clear, all, false)
    }

    #[cfg(test)]
    pub(crate) fn clear_observed(
        &self,
        clear: Option<Category>,
        all: bool,
        before_clear: impl FnMut(&Path),
    ) -> Result<Report, Error> {
        let _lease = self.build_lease()?;
        let entries = self.entries_until(&Cancel::default(), if all { None } else { clear })?;
        self.apply_retaining_observed(entries, clear, all, true, before_clear)
    }

    pub(super) fn apply_retaining(
        &self,
        entries: Vec<Entry>,
        clear: Option<Category>,
        all: bool,
        retain: bool,
    ) -> Result<Report, Error> {
        self.apply_retaining_observed(entries, clear, all, retain, |_| {})
    }

    fn apply_retaining_observed(
        &self,
        entries: Vec<Entry>,
        clear: Option<Category>,
        all: bool,
        retain: bool,
        mut before_clear: impl FnMut(&Path),
    ) -> Result<Report, Error> {
        // Validate every descendant before any deletion and deduplicate allocated blocks per row.
        let mut rows: Vec<_> = Category::ALL.into_iter().map(Row::new).collect();
        // Each folder is walked once, here: the walk validates every descendant and its sizes
        // serve the rows and the kept lines.
        let (entries, mut trees) = if retain {
            refused(entries, &mut rows, &Cancel::default(), |category| {
                all || clear == Some(category)
            })?
        } else {
            let trees = entries
                .iter()
                .map(|entry| entry.tree(&Cancel::default()))
                .collect::<Result<Vec<_>, Error>>()?;
            (entries, trees)
        };
        for row in &mut rows {
            row.bytes = allocated(&entries, &trees, row.category);
            row.remaining_bytes = row.bytes;
            row.measured = all || clear.is_none() || clear == Some(row.category);
            row.has_candidates = entries
                .iter()
                .zip(&trees)
                .any(|(entry, tree)| entry.category == row.category && entry.is_candidate(tree));
        }
        let mut blocked_build_tools = false;
        let mut refused_indices = HashSet::new();
        for (index, entry) in entries.iter().enumerate() {
            if all || clear == Some(entry.category) {
                if !trees[index].present {
                    continue;
                }
                before_clear(&entry.path);
                let result = if blocked_build_tools && entry.build_tool() {
                    Err(Error::Busy)
                } else {
                    entry.remove(&self.root, retain, entry.root_id)
                };
                match result {
                    // Only a cleared folder is walked again, to see what is left of it.
                    Ok(()) => trees[index] = entry.tree(&Cancel::default())?,
                    Err(
                        error @ (Error::Busy
                        | Error::UnprovenExclusion
                        | Error::IncompleteUnit
                        | Error::UnpublishedGame
                        | Error::ResourceDependent(_)
                        | Error::ReferencedProfile
                        | Error::UnprovenReferences
                        | Error::UnsupportedFilesystem
                        | Error::Unsafe(_)),
                    ) if retain => {
                        let row = rows
                            .iter_mut()
                            .find(|row| row.category == entry.category)
                            .expect("the rows have one for every category");
                        blocked_build_tools |= entry.build_tool() && matches!(error, Error::Busy);
                        row.keep(trees[index].size(&mut HashSet::new()), error, true);
                        refused_indices.insert(index);
                    }
                    Err(error @ Error::Io(_)) if retain => {
                        // A failed recursive removal can already have freed some files. If
                        // the rest can't be measured, conservatively keep its original size.
                        if let Ok(tree) = entry.tree(&Cancel::default()) {
                            trees[index] = tree;
                        }
                        let row = rows
                            .iter_mut()
                            .find(|row| row.category == entry.category)
                            .expect("all categories have a row");
                        row.keep(trees[index].size(&mut HashSet::new()), error, true);
                        refused_indices.insert(index);
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        for row in &mut rows {
            if all || clear == Some(row.category) {
                row.after_clear = true;
                let remaining = allocated(&entries, &trees, row.category);
                row.freed_bytes = row.bytes.saturating_sub(remaining);
                row.remaining_bytes = remaining;
                row.has_candidates =
                    entries
                        .iter()
                        .zip(&trees)
                        .enumerate()
                        .any(|(index, (entry, tree))| {
                            entry.category == row.category
                                && entry.is_candidate(tree)
                                && !refused_indices.contains(&index)
                        });
            }
        }
        Ok(Report::new(rows))
    }
}

/// A row's bytes: its folders' walks, each allocated block counted once across the row.
pub(super) fn allocated(entries: &[Entry], trees: &[Tree], category: Category) -> u64 {
    let mut seen = HashSet::new();
    entries
        .iter()
        .zip(trees)
        .filter(|(entry, _)| entry.category == category)
        .fold(0u64, |bytes, (_, tree)| {
            bytes.saturating_add(tree.size(&mut seen))
        })
}

/// A unit holding a link or file the launcher does not expect is refused whole: it is neither
/// counted nor cleared, and its row says so. The other units are not held up by it.
fn refused(
    entries: Vec<Entry>,
    rows: &mut [Row],
    cancel: &Cancel,
    clearing: impl Fn(Category) -> bool,
) -> Result<(Vec<Entry>, Vec<Tree>), Error> {
    let mut sound = Vec::new();
    let mut trees = Vec::new();
    for entry in entries {
        if cancel.is_cancelled() {
            break;
        }
        let row = rows
            .iter_mut()
            .find(|row| row.category == entry.category)
            .expect("the rows have one for every category");
        if let Some(tree) = measure_entry(&entry, row, cancel, clearing(entry.category))? {
            sound.push(entry);
            trees.push(tree);
        }
    }
    Ok((sound, trees))
}

pub(super) fn measure_entry(
    entry: &Entry,
    row: &mut Row,
    cancel: &Cancel,
    clearing: bool,
) -> Result<Option<Tree>, Error> {
    match entry.tree(cancel) {
        Ok(tree) => {
            if !clearing && entry.incomplete_target() && tree.present {
                row.keep(tree.size(&mut HashSet::new()), Error::IncompleteUnit, false);
            }
            Ok(Some(tree))
        }
        Err(Error::Unsafe(_) | Error::Io(_)) if cancel.is_cancelled() => Ok(None),
        Err(error @ (Error::Unsafe(_) | Error::Io(_))) => {
            row.keep(0, error, clearing);
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod manual_tests {
    use super::*;

    #[test]
    fn profile_game_cache_inventory_counts_only_the_rebuildable_entries() {
        let home = crate::storage::tests::Home::new();
        let files = [
            home.seed(".sunrise-cache/behaviours-v8.bin"),
            home.seed(".sunrise-cache/behaviours-v7.bin"),
            home.seed(".sunrise-cache/registry-v1.bin"),
            home.seed(".sunrise-cache/mod-content/out"),
            home.seed(".sunrise-cache/resourcepacks/out"),
            home.seed("cache/dx12-shaders.bin"),
        ];
        let canaries = [
            home.seed(".sunrise-cache/behaviours-v9.bin"),
            home.seed(".sunrise-cache/player-data"),
            home.seed("cache/unknown/keep"),
            home.seed("cache/.trash-unrelated/keep"),
        ];
        let root = Root::open(&home.0).unwrap();
        let mut entries = Vec::new();
        profile_game_caches(&mut entries, &root).unwrap();
        // Simulate a profile separate from both the build root and launcher data directory.
        let launcher = crate::storage::tests::Home::new();
        let storage = Storage::at(&launcher.0).unwrap();
        let data = Root::open(&launcher.0).unwrap();
        storage
            .detached_trash(&mut entries, &data, &[], &Cancel::default())
            .unwrap();
        assert_eq!(entries.len(), 6);
        for file in files {
            assert_eq!(
                entries
                    .iter()
                    .filter(|entry| file.starts_with(&entry.path))
                    .count(),
                1
            );
        }
        for file in canaries {
            assert!(!entries.iter().any(|entry| file.starts_with(&entry.path)));
        }
    }

    #[test]
    fn refused_units_are_residual_with_bytes_and_reason() {
        let home = crate::storage::tests::Home::new();
        let file = home.seed("game/output");
        let storage = Storage::at(&home.0).unwrap();
        let root = Root::open(&home.0).unwrap();
        let entries = vec![Entry::fixed(&root, Path::new("game"), Category::RustCompiler).unwrap()];
        let lock = home.seed("game/.cargo-lock");
        let lock = fs::File::options()
            .read(true)
            .write(true)
            .open(lock)
            .unwrap();
        lock.lock().unwrap();
        let directory = file.parent().unwrap();
        // Give inline-directory filesystems enough names to allocate directory blocks too.
        for index in 0..32 {
            fs::write(
                directory.join(format!("padding-{index:02}-{}", "x".repeat(180))),
                [],
            )
            .unwrap();
        }
        // Independent filesystem allocations, not the recursive production sum.
        let own_bytes = |path: &Path| {
            super::super::super::allocation(path, &fs::symlink_metadata(path).unwrap())
                .unwrap()
                .1
        };
        let bytes = own_bytes(directory)
            + fs::read_dir(directory)
                .unwrap()
                .map(|entry| own_bytes(&entry.unwrap().path()))
                .sum::<u64>();
        let report = storage.apply_retaining(entries, None, true, true).unwrap();
        let row = report
            .rows
            .iter()
            .find(|row| row.category == Category::RustCompiler)
            .unwrap();
        assert_eq!(row.bytes, bytes);
        assert_eq!(row.freed_bytes, 0);
        assert_eq!(row.remaining_bytes, bytes);
        assert_eq!(row.kept.len(), 1);
        assert_eq!(row.kept[0].bytes, bytes);
        assert!(!row.kept[0].reason.is_empty());
        assert!(file.exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_unit_with_an_unexpected_link_is_refused_on_its_row_and_holds_up_no_other() {
        let home = crate::storage::tests::Home::new();
        let good = home.seed("rust/old/payload");
        let bad = home.seed("cache/payload");
        home.seed("worlds/keep");
        std::os::unix::fs::symlink(home.0.join("worlds"), home.0.join("cache/link")).unwrap();
        let storage = Storage::at(&home.0).unwrap();
        let root = Root::open(&home.0).unwrap();
        let entries = vec![
            Entry::fixed(&root, Path::new("cache"), Category::OtherCaches).unwrap(),
            Entry::fixed(&root, Path::new("rust/old"), Category::RustCompiler).unwrap(),
        ];
        let report = storage.apply_retaining(entries, None, true, true).unwrap();
        let row = |category| {
            report
                .rows
                .iter()
                .find(|row| row.category == category)
                .unwrap()
        };
        let refused = row(Category::OtherCaches);
        assert_eq!((refused.bytes, refused.freed_bytes), (0, 0));
        assert_eq!(refused.kept.len(), 1);
        assert!(refused.kept[0].reason.contains("link"));
        assert!(bad.exists());
        assert!(home.0.join("worlds/keep").exists());
        assert!(row(Category::RustCompiler).freed_bytes > 0);
        assert!(row(Category::RustCompiler).kept.is_empty());
        assert!(!good.exists());
    }

    #[cfg(unix)]
    #[test]
    fn retained_compiler_attempt_with_outside_link_is_cleared_without_following_it() {
        let home = crate::storage::tests::Home::new();
        let payload = home.seed("native/.nb-retained/output");
        let outside = home.seed("player-data/keep");
        let link = payload.with_file_name("link");
        std::os::unix::fs::symlink(outside.parent().unwrap(), &link).unwrap();
        let storage = Storage::at(&home.0).unwrap();
        let root = Root::open(&home.0).unwrap();
        let entries = vec![Entry::fixed(
            &root,
            Path::new("native/.nb-retained"),
            Category::BuildFiles,
        )
        .unwrap()];
        let report = storage
            .apply_retaining(entries, Some(Category::BuildFiles), false, true)
            .unwrap();
        let row = report
            .rows
            .iter()
            .find(|row| row.category == Category::BuildFiles)
            .unwrap();
        assert!(row.freed_bytes > 0);
        assert!(row.kept.is_empty());
        assert!(!payload.parent().unwrap().exists());
        assert!(outside.is_file());
    }

    #[test]
    fn a_refused_unit_is_walked_once_and_a_cleared_one_twice() {
        let home = crate::storage::tests::Home::new();
        home.seed("rust/old/payload");
        let storage = Storage::at(&home.0).unwrap();
        let root = Root::open(&home.0).unwrap();
        #[cfg(unix)]
        {
            home.seed("cache/payload");
            std::os::unix::fs::symlink(home.0.join("rust"), home.0.join("cache/link")).unwrap();
        }
        #[cfg_attr(not(unix), allow(unused_mut))]
        let mut entries =
            vec![Entry::fixed(&root, Path::new("rust/old"), Category::RustCompiler).unwrap()];
        #[cfg(unix)]
        entries.push(Entry::fixed(&root, Path::new("cache"), Category::OtherCaches).unwrap());
        let count = entries.len();
        let walks = crate::storage::walks();
        let report = storage.apply_retaining(entries, None, true, true).unwrap();
        // The refused folder was walked once (and stopped there); the sound one once, then once
        // more after it was cleared.
        assert_eq!(crate::storage::walks() - walks, count + 1);
        assert!(
            report
                .rows
                .iter()
                .find(|row| row.category == Category::RustCompiler)
                .unwrap()
                .freed_bytes
                > 0
        );
        #[cfg(unix)]
        assert_eq!(
            report
                .rows
                .iter()
                .find(|row| row.category == Category::OtherCaches)
                .unwrap()
                .kept
                .len(),
            1
        );
    }
}
