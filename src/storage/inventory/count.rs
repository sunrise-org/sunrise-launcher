//! The read-only count behind the Storage page and `storage`. It finds the units once and walks
//! each of them once, row by row; it needs no build lease and decides nothing about deleting.
use super::*;

#[cfg(test)]
mod tests;

impl Storage {
    /// Measures the rows one after another without the build lease, deleting anything or asking
    /// what any unit's executables are: discovery runs once, each unit is walked once, and the
    /// report so far goes to `counted` as soon as a row's own units are done. A refusal that
    /// only a Clear can judge shows up there, not here.
    ///
    /// `Ok(None)` means `cancel` was set first: whatever the last `counted` call held is all
    /// that is known, and a partly walked row is never handed over as if it were whole.
    pub fn count(
        &self,
        cancel: &Cancel,
        mut counted: impl FnMut(&Report),
    ) -> Result<Option<Report>, Error> {
        if cancel.is_cancelled() {
            return Ok(None);
        }
        let discovery = self.entries_until(cancel, None);
        if cancel.is_cancelled() {
            return Ok(None);
        }
        let mut remaining = discovery.map_err(redact_error)?;
        let mut report = Report::new(Vec::new()).tagged(cancel);
        for category in Category::ALL {
            let (units, rest): (Vec<_>, Vec<_>) = remaining
                .into_iter()
                .partition(|entry| entry.category == category);
            remaining = rest;
            let mut row = Row::new(category);
            let mut seen = HashSet::new();
            for entry in units {
                if cancel.is_cancelled() {
                    return Ok(None);
                }
                let measured = measure_entry(&entry, &mut row, cancel, false);
                if cancel.is_cancelled() {
                    return Ok(None);
                }
                if let Some(tree) = measured.map_err(redact_error)? {
                    row.has_candidates |= entry.is_candidate(&tree);
                    row.bytes = row.bytes.saturating_add(tree.size(&mut seen));
                }
            }
            if cancel.is_cancelled() {
                return Ok(None);
            }
            row.measured = true;
            row.remaining_bytes = row.bytes;
            report.push(row);
            counted(&report);
        }
        Ok(Some(report))
    }

    /// The same count with nobody to cancel it, for the command line: `progress` hears of each
    /// finished row.
    pub fn report_counting(&self, progress: impl FnMut(&Report)) -> Result<Report, Error> {
        Ok(self
            .count(&Cancel::default(), progress)?
            .expect("a count nobody can cancel finishes"))
    }
}
