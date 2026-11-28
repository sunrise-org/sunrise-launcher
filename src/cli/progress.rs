//! Download progress presentation for terminal and redirected CLI output.
use std::io::{IsTerminal, Write};
use sunrise_launcher::native_mods;

/// Download progress on one terminal line, only when stderr is a terminal and the percentage moves.
pub(super) fn progress(label: String) -> impl Fn(f32) {
    let stderr = std::io::stderr();
    let line = DownloadLine::new(stderr.is_terminal(), stderr);
    move |fraction| line.update(&label, fraction)
}

/// Install may return without the last measured fraction reaching one.
pub(super) fn install_progress<T>(label: &str, install: impl FnOnce(&dyn Fn(f32)) -> T) -> T {
    let stderr = std::io::stderr();
    with_install_progress(label, stderr.is_terminal(), stderr, install)
}

fn with_install_progress<T>(
    label: &str,
    tty: bool,
    stderr: impl Write,
    install: impl FnOnce(&dyn Fn(f32)) -> T,
) -> T {
    let line = DownloadLine::new(tty, stderr);
    let result = install(&|fraction| line.update(label, fraction));
    if line.open.get() {
        let mut stderr = line.stderr.borrow_mut();
        let _ = writeln!(stderr);
        let _ = stderr.flush();
    }
    result
}

struct DownloadLine<W> {
    tty: bool,
    stderr: std::cell::RefCell<W>,
    last: std::cell::Cell<i32>,
    open: std::cell::Cell<bool>,
}

impl<W: Write> DownloadLine<W> {
    fn new(tty: bool, stderr: W) -> Self {
        Self {
            tty,
            stderr: std::cell::RefCell::new(stderr),
            last: std::cell::Cell::new(-1),
            open: std::cell::Cell::new(false),
        }
    }

    fn update(&self, label: &str, fraction: f32) {
        let pct = (fraction * 100.) as i32;
        if !self.tty || pct == self.last.replace(pct) {
            return;
        }
        let mut stderr = self.stderr.borrow_mut();
        let _ = write!(stderr, "\r{label} {pct:>3}%");
        let open = fraction < 1.0;
        if !open {
            let _ = writeln!(stderr);
        }
        self.open.set(open);
        let _ = stderr.flush();
    }
}

/// Keep detailed terminal updates, but only the first and completed download updates in logs.
pub(super) fn native_progress(tty: bool) -> impl Fn(native_mods::Step) -> Option<String> {
    use native_mods::Step;
    let previous = std::cell::RefCell::new(None);
    let shown = std::cell::RefCell::new(String::new());
    move |step| {
        let show = tty
            || match (&step, previous.borrow().as_ref()) {
                (Step::Rust(done), Some(Step::Rust(last))) => *done >= 1.0 && *last < 1.0,
                (
                    Step::Tools { build, done, total },
                    Some(Step::Tools {
                        build: last_build,
                        done: last_done,
                        total: last_total,
                    }),
                ) if build == last_build => {
                    *total > 0 && done >= total && (*last_total == 0 || last_done < last_total)
                }
                _ => true,
            };
        // A line already on screen is never printed again: a download reports every chunk, and
        // most of them land on the same whole percent or megabyte.
        let label = show
            .then(|| step.label())
            .filter(|label| *shown.borrow() != *label);
        if let Some(label) = &label {
            shown.replace(label.clone());
        }
        *previous.borrow_mut() = Some(step);
        label
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_progress_terminates_the_partial_line_before_reporting_the_result() {
        for (tty, last, result, expected) in [
            (true, Some(0.75), Ok("build"), "\rInstalling build  75%\n"),
            (true, Some(0.75), Err("failed"), "\rInstalling build  75%\n"),
            (true, Some(1.0), Ok("build"), "\rInstalling build 100%\n"),
            (true, None, Ok("build"), ""),
            (false, Some(0.75), Ok("build"), ""),
        ] {
            let mut stderr = Vec::new();
            let installed = with_install_progress("Installing build", tty, &mut stderr, |report| {
                if let Some(last) = last {
                    report(last);
                }
                result
            });
            assert_eq!(stderr, expected.as_bytes());
            assert_eq!(installed, result);
            let stdout = installed.map(|tag| {
                serde_json::to_string_pretty(
                    &serde_json::json!({ "installed": tag, "prepared": false }),
                )
                .unwrap()
            });
            assert_eq!(
                stdout.as_deref().map_err(|error| *error),
                result.map(|_| "{\n  \"installed\": \"build\",\n  \"prepared\": false\n}")
            );
        }
    }

    #[test]
    fn native_progress_limits_redirected_rust_downloads_to_start_and_end() {
        let progress = native_progress(false);
        let fractions = [
            0.0, 0.0, 0.01, 0.97, 0.5, 0.971, 0.974, 0.98, 0.9999, 1.0, 1.0,
        ];
        let labels: Vec<_> = fractions
            .into_iter()
            .filter_map(|done| progress(native_mods::Step::Rust(done)))
            .collect();
        assert_eq!(
            labels,
            [
                format!(
                    "Downloading Rust {} (once): 0%",
                    sunrise_launcher::rust_toolchain::VERSION
                ),
                format!(
                    "Downloading Rust {} (once): 100%",
                    sunrise_launcher::rust_toolchain::VERSION
                ),
            ]
        );
    }

    #[test]
    fn native_progress_keeps_detailed_terminal_downloads() {
        let progress = native_progress(true);
        for (done, percent) in [
            (0.0, Some(0)),
            (0.97, Some(97)),
            (0.971, None),
            (0.974, None),
            (0.98, Some(98)),
            (1.0, Some(100)),
            (1.0, None),
        ] {
            assert_eq!(
                progress(native_mods::Step::Rust(done)),
                percent.map(|percent| format!(
                    "Downloading Rust {} (once): {percent}%",
                    sunrise_launcher::rust_toolchain::VERSION
                ))
            );
        }
        for done in [0, 1, 97, 100] {
            let step = native_mods::Step::Tools {
                build: "sdk".into(),
                done: done * 1_000_000,
                total: 100_000_000,
            };
            assert_eq!(
                progress(step),
                Some(format!("Getting the mod tools for sdk: {done} of 100 MB"))
            );
        }
    }

    #[test]
    fn native_progress_limits_each_redirected_sdk_download() {
        let progress = native_progress(false);
        let tools = |build: &str, done, total| native_mods::Step::Tools {
            build: build.into(),
            done,
            total,
        };
        for (done, expected) in [
            (0, Some("Getting the mod tools for first: 0 of 100 MB")),
            (1, None),
            (97, None),
            (50, None),
            (99, None),
            (100, Some("Getting the mod tools for first: 100 of 100 MB")),
            (100, None),
        ] {
            assert_eq!(
                progress(tools("first", done * 1_000_000, 100_000_000)),
                expected.map(str::to_owned)
            );
        }
        assert_eq!(
            progress(tools("second", 20_000_000, 100_000_000)),
            Some("Getting the mod tools for second: 20 of 100 MB".into())
        );
        assert_eq!(progress(tools("second", 50_000_000, 100_000_000)), None);
        assert_eq!(
            progress(tools("second", 100_000_000, 100_000_000)),
            Some("Getting the mod tools for second: 100 of 100 MB".into())
        );
        assert_eq!(
            progress(tools("third", 0, 0)),
            Some("Getting the mod tools for third: 0 of 1 MB".into())
        );
        assert_eq!(progress(tools("third", 0, 0)), None);
        assert_eq!(
            progress(tools("third", 100_000_000, 100_000_000)),
            Some("Getting the mod tools for third: 100 of 100 MB".into())
        );
    }

    #[test]
    fn native_progress_never_repeats_the_line_just_shown() {
        let compile = |name: &str| native_mods::Step::Compile {
            done: 3,
            total: Some(9),
            name: name.into(),
            profile: Default::default(),
        };
        for tty in [false, true] {
            let progress = native_progress(tty);
            let labels: Vec<_> = ["a", "a", "a", "b", "b"]
                .into_iter()
                .filter_map(|name| progress(compile(name)))
                .collect();
            assert_eq!(
                labels,
                [
                    "Compiling the game (optimized): 3 of 9 crates, now a",
                    "Compiling the game (optimized): 3 of 9 crates, now b",
                ],
                "tty: {tty}"
            );
        }
    }

    #[test]
    fn native_progress_keeps_phase_order_and_restarts_download_reporting() {
        use native_mods::Step;
        let progress = native_progress(false);
        assert_eq!(
            progress(Step::Rust(0.97)),
            Some(format!(
                "Downloading Rust {} (once): 97%",
                sunrise_launcher::rust_toolchain::VERSION
            ))
        );
        assert_eq!(progress(Step::Rust(0.98)), None);
        let labels: Vec<_> = [
            Step::PrebuiltMods,
            Step::Compile {
                done: 1,
                total: Some(2),
                name: "first".into(),
                profile: Default::default(),
            },
            Step::Compile {
                done: 2,
                total: Some(2),
                name: "second".into(),
                profile: Default::default(),
            },
            Step::Install,
            Step::Rust(0.0),
            Step::Rust(1.0),
        ]
        .into_iter()
        .filter_map(progress)
        .collect();
        assert_eq!(
            labels,
            vec![
                "Compiling the mods against the prebuilt game".to_owned(),
                "Compiling the game (optimized): 1 of 2 crates, now first".to_owned(),
                "Compiling the game (optimized): 2 of 2 crates, now second".to_owned(),
                "Installing the build".to_owned(),
                format!(
                    "Downloading Rust {} (once): 0%",
                    sunrise_launcher::rust_toolchain::VERSION
                ),
                format!(
                    "Downloading Rust {} (once): 100%",
                    sunrise_launcher::rust_toolchain::VERSION
                ),
            ]
        );
    }
}
