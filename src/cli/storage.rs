//! `storage`: what the launcher's managed files take up, and clearing a row of them.
use super::*;
use std::io::{self, Write};
use sunrise_launcher::storage::error_message;

/// The `storage` report. Counting says on stderr when each row is done (unless `--json` is
/// asked for), so a big cache shows progress while stdout still gets the one finished report;
/// a Clear needs no count first.
fn storage_report(
    storage: &Storage,
    row: Option<Category>,
    all: bool,
    as_json: bool,
) -> Result<sunrise_launcher::storage::Report, sunrise_launcher::storage::Error> {
    if as_json || row.is_some() || all {
        return storage.report(row, all);
    }
    storage.report_counting(|so_far| {
        let Some(done) = so_far.rows.last() else {
            return;
        };
        let _ = writeln!(
            io::stderr(),
            "{}",
            trf(
                "Counted {category}: {size}",
                &[
                    ("category", done.category.label()),
                    (
                        "size",
                        &sunrise_launcher::storage::format_bytes(done.remaining_bytes)
                    ),
                ]
            )
        );
    })
}

pub(super) fn report(clear: Option<ClearStorage>, as_json: bool) -> Result<i32, Error> {
    let (row, all) = match clear {
        Some(ClearStorage::Row(row)) => (Some(row), false),
        Some(ClearStorage::All) => (None, true),
        None => (None, false),
    };
    let report =
        match Storage::open().and_then(|storage| storage_report(&storage, row, all, as_json)) {
            Ok(report) => report,
            Err(error) => {
                let detail = sunrise_launcher::storage::error_log_detail(&error);
                let message = error_message(error);
                let _ = writeln!(io::stderr(), "{message}");
                match detail {
                    Some(detail) => {
                        sunrise_launcher::launcher_log::line(format!("{message} [{detail}]"))
                    }
                    None => sunrise_launcher::launcher_log::line(&message),
                }
                return Ok(1);
            }
        };
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    let mut kept_any = false;
    for row in report.rows.iter().filter(|row| row.after_clear) {
        if let Some(kept) = row.kept.first() {
            kept_any = true;
            let _ = writeln!(io::stderr(), "{}: {}", row.category.label(), kept.reason);
        }
    }
    Ok(i32::from(kept_any))
}
