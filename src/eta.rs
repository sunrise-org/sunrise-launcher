//! Smoothed time-remaining estimates for loading dialogs and status strips.
//!
//! Determinate stages (downloads, toolchains, native builds) estimate remaining time from an
//! exponential moving average of their progress rate after a short warmup. Indeterminate
//! preparation (`PrepareStep::Prepare`) has no live percentage, so it falls back to the
//! machine's own previous preparation duration saved in `eta-timings.json`.

use crate::i18n::{tr, trf};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use sunrise_launcher::backend::{self, PrepareStep};
use sunrise_launcher::browser::sunrise::InstallProgress;

/// Wait this long in a stage before showing an ETA so fast steps and bursty first chunks
/// don't flash noisy numbers.
const WARMUP: Duration = Duration::from_secs(2);

/// Minimum elapsed time between rate samples so bursty chunk callbacks don't skew the EMA.
const MIN_SAMPLE_DT: f64 = 0.15;

/// Hide the ETA if a determinate stage hasn't moved in this long.
const STALL_TIMEOUT: Duration = Duration::from_secs(15);

/// A fraction drop larger than this inside the same stage means a new sub-download started
/// (for example Mojang's file sets or sequential mod downloads), so reset the estimator.
const RESTART_DROP: f32 = 0.05;

/// Smoothing factor for the progress rate EMA (`fraction / second`).
const RATE_ALPHA: f64 = 0.25;

/// Smoothing factor for persisted historical durations (`PrepareStep::Prepare`).
const HISTORY_ALPHA: f64 = 0.35;

/// Bounds for determinate fraction display.
const MIN_FRACTION: f32 = 0.02;
const MAX_FRACTION: f32 = 0.995;

/// Discard absurdly large ETAs (> 3 hours).
const MAX_ETA_SECS: f64 = 3.0 * 3600.0;

/// Bounds for recording and reading a historical preparation duration.
const MIN_RECORD_PREPARE_SECS: f64 = 2.0;
const MAX_RECORD_PREPARE_SECS: f64 = 30.0 * 60.0;

/// Tracks a single active loading stage and estimates how much time is left.
#[derive(Clone, Debug, Default)]
pub(crate) struct StageEta {
    stage: Option<u8>,
    started: Option<Instant>,
    start_fraction: f32,
    last_sample: Option<(Instant, f32)>,
    last_advance: Option<Instant>,
    rate_ema: Option<f64>,
    last_fraction: f32,
}

impl StageEta {
    fn begin(&mut self, stage: u8, fraction: Option<f32>, now: Instant) {
        self.stage = Some(stage);
        self.started = Some(now);
        self.start_fraction = fraction.unwrap_or(0.0);
        self.last_sample = fraction.map(|f| (now, f));
        self.last_advance = Some(now);
        self.rate_ema = None;
        self.last_fraction = self.start_fraction;
    }

    /// Feed the current stage key and optional determinate fraction (`0.0..=1.0`).
    /// Switching to a different `stage`, or restarting a sub-download inside the same stage,
    /// resets the estimator cleanly.
    pub(crate) fn update(&mut self, stage: u8, fraction: Option<f32>, now: Instant) {
        let clamped = fraction
            .filter(|f| f.is_finite())
            .map(|f| f.clamp(0.0, 1.0));
        if self.stage != Some(stage) {
            self.begin(stage, clamped, now);
            return;
        }
        let Some(f) = clamped else {
            return;
        };
        if f + RESTART_DROP < self.last_fraction {
            self.begin(stage, Some(f), now);
            return;
        }
        if f > self.last_fraction + 1e-4 {
            if let Some((prev_time, prev_f)) = self.last_sample {
                let dt = now.saturating_duration_since(prev_time).as_secs_f64();
                let df = (f - prev_f) as f64;
                if dt >= MIN_SAMPLE_DT && df > 0.0 {
                    let inst_rate = df / dt;
                    self.rate_ema = Some(match self.rate_ema {
                        Some(prev) => RATE_ALPHA * inst_rate + (1.0 - RATE_ALPHA) * prev,
                        None => inst_rate,
                    });
                    self.last_sample = Some((now, f));
                }
            } else {
                self.last_sample = Some((now, f));
            }
            self.last_fraction = f;
            self.last_advance = Some(now);
        }
    }

    /// Estimated time remaining for a determinate stage, or `None` while warming up, stalled,
    /// nearly complete, or indeterminate.
    pub(crate) fn remaining(&self, now: Instant) -> Option<Duration> {
        let started = self.started?;
        let elapsed = now.saturating_duration_since(started);
        if elapsed < WARMUP {
            return None;
        }
        let last_advance = self.last_advance?;
        if now.saturating_duration_since(last_advance) >= STALL_TIMEOUT {
            return None;
        }
        if self.last_fraction < MIN_FRACTION || self.last_fraction >= MAX_FRACTION {
            return None;
        }
        let rate = self.rate_ema.unwrap_or_else(|| {
            let secs = elapsed.as_secs_f64();
            let gained = (self.last_fraction - self.start_fraction).max(0.0) as f64;
            if secs > 0.0 && gained > 0.0 {
                gained / secs
            } else {
                0.0
            }
        });
        if !rate.is_finite() || rate <= 0.0 {
            return None;
        }
        let remaining_secs = (1.0 - self.last_fraction as f64) / rate;
        if !remaining_secs.is_finite() || remaining_secs > MAX_ETA_SECS {
            return None;
        }
        Duration::try_from_secs_f64(remaining_secs.max(0.0)).ok()
    }
}

/// Estimated time remaining for an indeterminate stage whose typical duration on this machine
/// is known (such as `PrepareStep::Prepare`). Drops the estimate once `elapsed >= expected`
/// instead of counting upward or sticking at zero.
pub(crate) fn remaining_from_expected(
    started: Instant,
    expected: Duration,
    now: Instant,
) -> Option<Duration> {
    let elapsed = now.saturating_duration_since(started);
    if elapsed < WARMUP || elapsed >= expected {
        return None;
    }
    Some(expected.saturating_sub(elapsed))
}

/// Distinct stage key for each `InstallProgress` phase so transitions reset `StageEta`.
pub(crate) fn install_stage_key(step: InstallProgress) -> u8 {
    match step {
        InstallProgress::Waiting => 0,
        InstallProgress::PreparingBuild => 1,
        InstallProgress::DownloadingBuild(_) => 2,
        InstallProgress::Minecraft(PrepareStep::Files, _) => 3,
        InstallProgress::Minecraft(PrepareStep::Java, _) => 4,
        InstallProgress::Minecraft(PrepareStep::Sounds, _) => 5,
        InstallProgress::Minecraft(PrepareStep::Prepare, _) => 6,
        InstallProgress::Checking => 7,
        InstallProgress::Downloading(_) => 8,
        InstallProgress::Preparing => 9,
        InstallProgress::Tools { .. } => 10,
        InstallProgress::Rust(_) => 11,
        InstallProgress::Building(_) => 12,
        InstallProgress::Publishing => 13,
        InstallProgress::Reviewing => 14,
        InstallProgress::CheckingBuild => 15,
        InstallProgress::UnpackingBuild => 16,
        InstallProgress::FinishingBuild => 17,
    }
}

/// A remaining duration rounded into calm, coarse steps so the text never jitters every second.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Left {
    FewSeconds,
    Seconds(u64),
    Minutes(u64),
    Hours(u64, u64),
}

fn rounded(remaining: Duration) -> Left {
    let secs = remaining
        .as_secs()
        .saturating_add(u64::from(remaining.subsec_nanos() >= 500_000_000));
    if secs < 5 {
        return Left::FewSeconds;
    }
    if secs <= 52 {
        return Left::Seconds(((secs.saturating_add(2) / 5) * 5).clamp(5, 50));
    }
    if secs <= 89 {
        return Left::Minutes(1);
    }
    let total_mins = secs.saturating_add(30) / 60;
    if total_mins < 60 {
        return Left::Minutes(total_mins);
    }
    Left::Hours(total_mins / 60, total_mins % 60)
}

/// A remaining duration as every progress line says it: "ETA: 2 min".
pub(crate) fn eta_label(remaining: Duration) -> String {
    let duration = match rounded(remaining) {
        Left::FewSeconds => tr("a few seconds").to_string(),
        Left::Seconds(seconds) => trf("{seconds} sec", &[("seconds", &seconds.to_string())]),
        Left::Minutes(minutes) => trf("{minutes} min", &[("minutes", &minutes.to_string())]),
        Left::Hours(hours, 0) => trf("{hours} hr", &[("hours", &hours.to_string())]),
        Left::Hours(hours, minutes) => trf(
            "{hours} hr {minutes} min",
            &[
                ("hours", &hours.to_string()),
                ("minutes", &minutes.to_string()),
            ],
        ),
    };
    trf("ETA: {duration}", &[("duration", &duration)])
}

/// Append a formatted ETA to a status line when one is available.
pub(crate) fn with_eta(label: String, remaining: Option<Duration>) -> String {
    match remaining {
        Some(duration) => trf(
            "{status} · {eta}",
            &[("status", &label), ("eta", &eta_label(duration))],
        ),
        None => label,
    }
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
struct Timings {
    #[serde(default)]
    prepare_secs: Option<f64>,
}

fn valid_prepare_secs(secs: f64) -> Option<f64> {
    (secs.is_finite() && (MIN_RECORD_PREPARE_SECS..=MAX_RECORD_PREPARE_SECS).contains(&secs))
        .then_some(secs)
}

fn timings_path() -> PathBuf {
    backend::data_dir().join("eta-timings.json")
}

fn cached_timings() -> &'static Mutex<Option<Timings>> {
    static CACHE: OnceLock<Mutex<Option<Timings>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

fn load_from(path: &Path) -> Timings {
    let mut timings = fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Timings>(&text).ok())
        .unwrap_or_default();
    timings.prepare_secs = timings.prepare_secs.and_then(valid_prepare_secs);
    timings
}

fn save_to(path: &Path, timings: &Timings) {
    let Ok(bytes) = serde_json::to_vec_pretty(timings) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let tmp = path.with_extension("json.tmp");
    if fs::write(&tmp, &bytes).is_ok() && fs::rename(&tmp, path).is_ok() {
        return;
    }
    let _ = fs::remove_file(&tmp);
}

fn blend_prepare_secs(existing: Option<f64>, elapsed: Duration) -> Option<f64> {
    let existing = existing.and_then(valid_prepare_secs);
    let Some(secs) = valid_prepare_secs(elapsed.as_secs_f64()) else {
        return existing;
    };
    Some(match existing {
        Some(prev) => HISTORY_ALPHA * secs + (1.0 - HISTORY_ALPHA) * prev,
        None => secs,
    })
}

/// Load `eta-timings.json` off the window thread once at startup so render never touches disk.
pub(crate) fn preload_timings() {
    let path = timings_path();
    std::thread::spawn(move || {
        let loaded = load_from(&path);
        if let Ok(mut guard) = cached_timings().lock() {
            if guard.is_none() {
                *guard = Some(loaded);
            }
        }
    });
}

/// Expected duration of `PrepareStep::Prepare` on this machine, from the in-memory cache.
pub(crate) fn expected_prepare_duration() -> Option<Duration> {
    let guard = cached_timings().lock().ok()?;
    let secs = guard.as_ref()?.prepare_secs.and_then(valid_prepare_secs)?;
    Duration::try_from_secs_f64(secs).ok()
}

/// Record a completed `PrepareStep::Prepare` duration in memory and persist it off the window thread.
pub(crate) fn record_prepare_duration(elapsed: Duration) {
    let path = timings_path();
    let updated = {
        let Ok(mut guard) = cached_timings().lock() else {
            return;
        };
        let timings = guard.get_or_insert_with(Timings::default);
        timings.prepare_secs = blend_prepare_secs(timings.prepare_secs, elapsed);
        timings.clone()
    };
    std::thread::spawn(move || save_to(&path, &updated));
}

/// Player-facing label for one install stage.
pub(crate) fn stage_label(step: InstallProgress) -> String {
    match step {
        InstallProgress::CheckingBuild => tr("Checking the build…").into(),
        InstallProgress::UnpackingBuild => tr("Unpacking the build").into(),
        InstallProgress::FinishingBuild => tr("Putting the build in place").into(),
        InstallProgress::PreparingBuild => tr("Getting the build ready…").into(),
        InstallProgress::Checking => tr("Checking the mod set…").into(),
        InstallProgress::Waiting => tr("Waiting for another build to finish…").into(),
        InstallProgress::Downloading(f) => trf(
            "Downloading {percent}%",
            &[("percent", &format!("{:.0}", f * 100.))],
        ),
        InstallProgress::DownloadingBuild(None) => tr("Downloading the build…").into(),
        InstallProgress::DownloadingBuild(Some(f)) => trf(
            "Downloading the build {percent}%",
            &[("percent", &format!("{:.0}", f.clamp(0., 1.) * 100.))],
        ),
        InstallProgress::Minecraft(step, files) => {
            use sunrise_launcher::backend::PrepareStep;
            let count = files.map(|(done, total)| (done.to_string(), total.to_string()));
            match (step, count) {
                (PrepareStep::Files, None) => tr("Finding Minecraft's files…").into(),
                (PrepareStep::Files, Some((done, total))) => trf(
                    "Downloading Minecraft's files ({done} of {total})",
                    &[("done", &done), ("total", &total)],
                ),
                (PrepareStep::Java, None) => tr("Finding Java…").into(),
                (PrepareStep::Java, Some((done, total))) => trf(
                    "Downloading Java ({done} of {total} files)",
                    &[("done", &done), ("total", &total)],
                ),
                (PrepareStep::Sounds, _) => tr("Adding the sounds…").into(),
                (PrepareStep::Prepare, _) => tr("Preparing Minecraft's files…").into(),
            }
        }
        InstallProgress::Preparing => tr("Getting ready…").into(),
        InstallProgress::Rust(fraction) => trf(
            "Downloading build tools {percent}%",
            &[("percent", &format!("{:.0}", fraction * 100.0))],
        ),
        InstallProgress::Tools { total, .. } => trf(
            "Getting build tools ({size})",
            &[("size", &sunrise_launcher::storage::format_bytes(total))],
        ),
        InstallProgress::Building(Some(fraction)) => trf(
            "Building {percent}%",
            &[(
                "percent",
                &format!("{:.0}", (fraction * 100.0).clamp(0.0, 100.0)),
            )],
        ),
        InstallProgress::Building(None) => tr("Building").into(),
        InstallProgress::Publishing => tr("Finishing…").into(),
        InstallProgress::Reviewing => tr("Review changes…").into(),
    }
}

/// Determinate fraction of one install stage, when it has one.
pub(crate) fn stage_fraction(step: InstallProgress) -> Option<f32> {
    match step {
        InstallProgress::DownloadingBuild(f) => f.map(|f| f.clamp(0., 1.)),
        InstallProgress::Downloading(f) | InstallProgress::Rust(f) => Some(f),
        InstallProgress::Minecraft(_, Some((done, total))) if total > 0 => {
            Some(done as f32 / total as f32)
        }
        InstallProgress::Building(f) => f,
        InstallProgress::Tools { done, total } if total > 0 => Some(done as f32 / total as f32),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_build_downloads_have_an_eta_and_finishing_is_not_switch_publication() {
        let stages = [
            InstallProgress::CheckingBuild,
            InstallProgress::UnpackingBuild,
            InstallProgress::FinishingBuild,
            InstallProgress::PreparingBuild,
            InstallProgress::Publishing,
        ];
        let keys = stages.map(install_stage_key);
        for (i, key) in keys.into_iter().enumerate() {
            assert!(!keys[..i].contains(&key));
            let mut eta = StageEta::default();
            let start = Instant::now();
            let download_key = install_stage_key(InstallProgress::DownloadingBuild(Some(0.)));
            eta.update(download_key, Some(0.0), start);
            let ready = start + Duration::from_secs(3);
            eta.update(download_key, Some(0.25), ready);
            assert!(eta.remaining(ready).is_some());
            eta.update(key, None, ready);
            assert_eq!(eta.remaining(ready), None);
        }
    }

    #[test]
    fn determinate_eta_warms_up_and_smooths_rate() {
        let mut eta = StageEta::default();
        let t0 = Instant::now();
        eta.update(2, Some(0.0), t0);
        // Too early: still within warmup.
        eta.update(2, Some(0.10), t0 + Duration::from_secs(1));
        assert_eq!(eta.remaining(t0 + Duration::from_secs(1)), None);

        // At 2s with 20% done (rate = 0.1/s), 80% left -> ~8s.
        eta.update(2, Some(0.20), t0 + Duration::from_secs(2));
        let rem = eta.remaining(t0 + Duration::from_secs(2)).unwrap();
        assert!((rem.as_secs_f64() - 8.0).abs() < 0.2, "got {rem:?}");

        // Switching stage resets the estimator.
        eta.update(8, Some(0.05), t0 + Duration::from_secs(3));
        assert_eq!(eta.remaining(t0 + Duration::from_secs(3)), None);
    }

    #[test]
    fn determinate_eta_resets_when_fraction_restarts_mid_stage() {
        let mut eta = StageEta::default();
        let t0 = Instant::now();
        eta.update(3, Some(0.0), t0);
        eta.update(3, Some(0.80), t0 + Duration::from_secs(4));
        assert!(eta.remaining(t0 + Duration::from_secs(4)).is_some());

        // Second file set starts back at 0.0 under the same stage key.
        eta.update(3, Some(0.0), t0 + Duration::from_secs(5));
        assert_eq!(eta.remaining(t0 + Duration::from_secs(5)), None);
        eta.update(3, Some(0.25), t0 + Duration::from_secs(7));
        let rem = eta.remaining(t0 + Duration::from_secs(7)).unwrap();
        assert!((rem.as_secs_f64() - 6.0).abs() < 0.2, "got {rem:?}");
    }

    #[test]
    fn fallback_rate_ignores_progress_present_when_stage_began() {
        let mut eta = StageEta::default();
        let t0 = Instant::now();
        // Stage begins already 50% complete; tiny rapid updates stay under MIN_SAMPLE_DT.
        eta.update(2, Some(0.50), t0);
        eta.update(2, Some(0.501), t0 + Duration::from_millis(50));
        // Only 0.001 gained over 2s -> rate ~0.0005/s, remaining ~998s (not ~2s from 0.501/2s).
        let rem = eta.remaining(t0 + Duration::from_secs(2)).unwrap();
        assert!(rem.as_secs() > 500, "got {rem:?}");
    }

    #[test]
    fn determinate_eta_hides_when_stalled_or_nearly_done() {
        let mut eta = StageEta::default();
        let t0 = Instant::now();
        eta.update(2, Some(0.0), t0);
        eta.update(2, Some(0.5), t0 + Duration::from_secs(5));
        assert!(eta.remaining(t0 + Duration::from_secs(6)).is_some());
        // Stalled past STALL_TIMEOUT -> hidden.
        assert_eq!(eta.remaining(t0 + Duration::from_secs(25)), None);

        // Nearly complete -> hidden.
        eta.update(2, Some(0.999), t0 + Duration::from_secs(26));
        assert_eq!(eta.remaining(t0 + Duration::from_secs(26)), None);
    }

    #[test]
    fn expected_duration_eta_warms_up_and_drops_when_exceeded() {
        let t0 = Instant::now();
        let expected = Duration::from_secs(30);
        assert_eq!(
            remaining_from_expected(t0, expected, t0 + Duration::from_secs(1)),
            None
        );
        assert_eq!(
            remaining_from_expected(t0, expected, t0 + Duration::from_secs(10)),
            Some(Duration::from_secs(20))
        );
        assert_eq!(
            remaining_from_expected(t0, expected, t0 + Duration::from_secs(30)),
            None
        );
        assert_eq!(
            remaining_from_expected(t0, expected, t0 + Duration::from_secs(45)),
            None
        );
    }

    #[test]
    fn eta_label_rounds_into_calm_coarse_steps() {
        assert_eq!(eta_label(Duration::from_secs(3)), "ETA: a few seconds");
        assert_eq!(eta_label(Duration::from_secs(7)), "ETA: 5 sec");
        assert_eq!(eta_label(Duration::from_secs(18)), "ETA: 20 sec");
        assert_eq!(eta_label(Duration::from_secs(52)), "ETA: 50 sec");
        assert_eq!(eta_label(Duration::from_secs(60)), "ETA: 1 min");
        assert_eq!(eta_label(Duration::from_secs(150)), "ETA: 3 min");
        assert_eq!(eta_label(Duration::from_secs(3600)), "ETA: 1 hr");
        assert_eq!(eta_label(Duration::from_secs(4500)), "ETA: 1 hr 15 min");
        // A huge duration never overflows.
        let _ = eta_label(Duration::MAX);
    }

    #[test]
    fn timings_round_trip_blend_and_drop_out_of_range_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("eta-timings.json");
        let first = blend_prepare_secs(None, Duration::from_secs(20));
        assert_eq!(first, Some(20.0));
        let second = blend_prepare_secs(first, Duration::from_secs(30)).unwrap();
        assert!((second - 23.5).abs() < 1e-6, "got {second}");
        // Out-of-range durations are ignored, and corrupt stored values are dropped.
        assert_eq!(
            blend_prepare_secs(Some(second), Duration::from_millis(500)),
            Some(second)
        );
        assert_eq!(
            blend_prepare_secs(Some(1e30), Duration::from_secs(15)),
            Some(15.0)
        );
        save_to(
            &path,
            &Timings {
                prepare_secs: Some(second),
            },
        );
        let loaded = load_from(&path);
        assert!((loaded.prepare_secs.unwrap() - second).abs() < 1e-6);

        // Damaged / huge values in eta-timings.json are dropped instead of panicking.
        fs::write(&path, r#"{"prepare_secs": 1e30}"#).unwrap();
        assert_eq!(load_from(&path).prepare_secs, None);
        fs::write(&path, r#"{"prepare_secs": -10.0}"#).unwrap();
        assert_eq!(load_from(&path).prepare_secs, None);
    }
}
