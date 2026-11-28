use super::*;
use std::sync::mpsc;

#[test]
fn performance_tracking_starts_without_hardware_probes() {
    let mut perf = Perf::new("test");
    perf.keep_local_log(false);
    assert!(perf.hardware.is_none());
    assert_eq!(perf.report.hardware.os, std::env::consts::OS);
    assert!(perf.report.hardware.cpu.is_none());
    assert!(perf.report.conditions.power.is_none());
}

/// A perf that has drawn its first frame, without the machine probes a real one starts.
fn drawn() -> Perf {
    let mut perf = Perf::new("test");
    perf.keep_local_log(false);
    perf.first_frame = Some(Instant::now());
    perf
}

fn ms(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

fn laid(offset: f32, max: f32) -> Target {
    Target::Laid { offset, max }
}

/// Ends switching, the last switch drawn, then opens the picker.
fn to_scrolling(bench: &mut Bench, perf: &Perf) {
    bench.began = Some(Instant::now() - SWITCHING);
    bench.awaiting = None;
    assert!(matches!(bench.tick(perf), Step::Scroll));
    assert!(bench.scrolling());
    assert!(!bench.timing_scroll(), "not before the list has its rows");
}

/// Ends scrolling, if it is still on, and resting; the next tick writes the result.
fn finish(bench: &mut Bench, perf: &Perf) -> serde_json::Value {
    let scroll = bench.scroll.as_mut().unwrap();
    if !scroll.ended {
        let now = Instant::now();
        scroll.began = Some(now - SCROLLING);
        if let Some(offset) = scroll.last_offset {
            assert_eq!(bench.after_frame(now, laid(offset, 100.)), None);
        }
        assert!(matches!(bench.tick(perf), Step::Wait), "scrolling ends");
    }
    assert!(!bench.scrolling());
    if bench.idle_cpu.is_none() {
        assert!(matches!(bench.tick(perf), Step::Wait), "resting begins");
    }
    bench.idle_cpu.as_mut().unwrap().0 = Instant::now() - IDLE;
    assert!(
        matches!(bench.tick(perf), Step::Quit),
        "the result, then quit"
    );
    let result = serde_json::from_str(&std::fs::read_to_string(&bench.out).unwrap()).unwrap();
    let _ = std::fs::remove_file(&bench.out);
    result
}

fn out(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("sunrise-bench-{name}-{}.json", std::process::id()))
}

#[test]
fn page_switch_samples_keep_stalls_and_leave_out_idle_gaps() {
    let perf = drawn();
    let mut bench = Bench::new(out("pages"), List::Builds);
    // Before the script starts, a frame is no sample.
    bench.frame(Instant::now(), Some(ms(16)));
    assert!(matches!(bench.tick(&perf), Step::Go(1)));
    let switched = bench.switched.unwrap();
    // The UI had rested for 2 s, then the switch waited 300 ms for its frame: a stall of
    // 300 ms, not a 2.3 s gap.
    bench.frame(switched + ms(300), Some(ms(2300)));
    // Drawn continuously while the switch animates.
    bench.frame(switched + ms(316), Some(ms(16)));
    assert_eq!(bench.pages_ms, [300., 16.]);
}

#[test]
fn a_stall_across_two_switches_is_timed_from_the_first() {
    let perf = drawn();
    let mut bench = Bench::new(out("two-switches"), List::Builds);
    assert!(matches!(bench.tick(&perf), Step::Go(1)));
    let first = bench.switched.unwrap();
    // The next switch is due before the window has drawn the first.
    bench.switched = Some(first - crate::wake::TICK);
    assert!(matches!(bench.tick(&perf), Step::Go(2)));
    assert!(
        bench.switched.unwrap() > first,
        "the pace follows the later switch"
    );
    // The frame finally comes 500 ms after the first switch.
    bench.frame(first + ms(500), Some(ms(900)));
    assert_eq!(bench.pages_ms, [500.], "not hidden behind the later switch");
}

#[test]
fn the_picker_waits_for_the_last_switch_to_be_drawn() {
    let perf = drawn();
    let mut bench = Bench::new(out("boundary"), List::Builds);
    assert!(matches!(bench.tick(&perf), Step::Go(1)));
    let switched = bench.switched.unwrap();
    // Switching time is up before the window has drawn the last switch.
    bench.began = Some(Instant::now() - SWITCHING);
    for _ in 0..3 {
        assert!(matches!(bench.tick(&perf), Step::Wait), "the picker waits");
        assert!(!bench.scrolling());
    }
    // The frame comes 700 ms late: a page sample, the stall in full.
    bench.frame(switched + ms(700), Some(ms(900)));
    assert_eq!(bench.pages_ms, [700.]);
    assert!(matches!(bench.tick(&perf), Step::Scroll));
}

#[test]
fn scrolling_is_timed_from_the_frame_that_drew_the_loaded_rows() {
    let mut perf = drawn();
    let mut bench = Bench::new(out("scroll"), List::Builds);
    to_scrolling(&mut bench, &perf);
    let now = Instant::now();
    // A cold start: the release list is still on its way, longer than the scroll phase.
    bench.scroll.as_mut().unwrap().opened = now - SCROLLING - ms(500);
    for target in [Target::Loading, Target::Loading, Target::LayingOut] {
        bench.frame(now, Some(ms(40)));
        assert_eq!(
            bench.after_frame(now, target),
            None,
            "nothing to scroll yet"
        );
        assert!(matches!(bench.tick(&perf), Step::Wait));
        assert!(bench.scrolling() && !bench.timing_scroll(), "still waiting");
    }
    // The first frame that drew the rows: the timing begins; the first step goes in.
    bench.frame(now, Some(ms(16)));
    assert_eq!(bench.after_frame(now, laid(0., 100.)), Some(-24.));
    assert!(bench.timing_scroll());
    assert!(matches!(bench.tick(&perf), Step::Wait), "timed from here");
    // That step moved it: a sample.
    let t1 = now + ms(17);
    bench.frame(t1, Some(ms(17)));
    assert_eq!(bench.after_frame(t1, laid(-24., 100.)), Some(-24.));
    // A frame that didn't move it isn't one.
    let t2 = t1 + ms(16);
    bench.frame(t2, Some(ms(16)));
    bench.after_frame(t2, laid(-24., 100.));
    // A real stall while scrolling is kept.
    let t3 = t2 + ms(400);
    bench.frame(t3, Some(ms(400)));
    bench.after_frame(t3, laid(-48., 100.));
    assert_eq!(
        bench.scroll_ms,
        [17., 400.],
        "the wait for the rows is none"
    );
    // Drawn while resting: no sample of either kind.
    perf.frame(t3 + ms(16));
    let result = finish(&mut bench, &perf);
    bench.frame(t3 + ms(16), Some(ms(16)));
    assert_eq!(
        bench.after_frame(t3 + ms(16), laid(-72., 100.)),
        None,
        "the wheel has stopped"
    );
    assert_eq!(bench.scroll_ms.len(), 2);
    assert_eq!(result["scroll_ms"], serde_json::json!([17., 400.]));
    assert!(result["scroll_skipped"].is_null());
    assert_eq!(result["pages_ms"], serde_json::json!([]));
    assert!(result["first_frame_epoch_ms"].as_f64().is_some());
}

#[test]
fn a_wheel_step_in_flight_at_the_deadline_is_measured_once() {
    let perf = drawn();
    let mut bench = Bench::new(out("last-wheel-step"), List::Builds);
    to_scrolling(&mut bench, &perf);
    let now = Instant::now();
    assert_eq!(bench.after_frame(now, laid(0., 100.)), Some(-24.));
    bench.scroll.as_mut().unwrap().began = Some(now - SCROLLING);
    assert!(matches!(bench.tick(&perf), Step::Wait));
    assert!(
        bench.scrolling(),
        "the dispatched wheel step still owes a frame"
    );

    let after = now + ms(400);
    bench.frame(after, Some(ms(400)));
    assert_eq!(bench.after_frame(after, laid(-24., 100.)), None);
    assert_eq!(bench.scroll_samples(), [400.]);
    assert!(!bench.scrolling());

    bench.frame(after + ms(16), Some(ms(16)));
    assert_eq!(bench.after_frame(after + ms(16), laid(-48., 100.)), None);
    assert_eq!(bench.scroll_samples(), [400.]);
}

#[test]
fn no_list_an_empty_one_or_one_that_fits_ends_scrolling_at_once() {
    for (target, says) in [
        (Target::NoProfile, Skip::NoProfile),
        (Target::Closed, Skip::Closed),
        (Target::Empty, Skip::Empty),
        (laid(0., 0.), Skip::Fits),
    ] {
        let perf = drawn();
        let mut bench = Bench::new(out("still"), List::Builds);
        to_scrolling(&mut bench, &perf);
        let now = Instant::now();
        bench.frame(now, Some(ms(16)));
        assert_eq!(bench.after_frame(now, target), None, "{target:?}");
        assert!(!bench.scrolling(), "{target:?}: no frames asked for");
        let result = finish(&mut bench, &perf);
        assert!(
            result["scroll_ms"].is_null(),
            "{target:?}: no result, not an empty fast one"
        );
        assert_eq!(result["scroll_skipped"], says.says(), "{target:?}");
    }
}

#[test]
fn a_list_that_never_comes_ends_with_why() {
    for (waiting, says) in [
        (Target::Loading, Skip::ReleasesLate),
        (Target::LayingOut, Skip::NotLaidOut),
    ] {
        let perf = drawn();
        let mut bench = Bench::new(out("late"), List::Builds);
        to_scrolling(&mut bench, &perf);
        assert_eq!(bench.after_frame(Instant::now(), waiting), None);
        bench.scroll.as_mut().unwrap().opened = Instant::now() - LIST_WAIT;
        assert!(matches!(bench.tick(&perf), Step::Wait));
        assert!(!bench.scrolling(), "{waiting:?}: given up on");
        let result = finish(&mut bench, &perf);
        assert!(result["scroll_ms"].is_null());
        assert_eq!(result["scroll_skipped"], says.says(), "{waiting:?}");
    }
}

#[test]
fn the_wheel_reaches_each_end_before_turning_round() {
    let mut down = true;
    // A list that overflows by less than a step still moves, all the way.
    assert_eq!(wheel_step(0., 10., &mut down), Some(-10.));
    assert_eq!(wheel_step(-10., 10., &mut down), Some(10.));
    assert!(!down);
    assert_eq!(wheel_step(0., 10., &mut down), Some(-10.));
    assert!(down);
    // A longer one: full steps, then only as far as the bottom.
    let (mut offset, mut down) = (0f32, true);
    let mut steps = Vec::new();
    for _ in 0..6 {
        let step = wheel_step(offset, 60., &mut down).unwrap();
        steps.push(step);
        offset = (offset + step).clamp(-60., 0.);
    }
    assert_eq!(steps, [-24., -24., -12., 24., 24., 12.]);
    assert_eq!(wheel_step(0., 0., &mut down), None);
}

#[test]
fn a_frame_sample_marks_the_report_unsaved() {
    let mut perf = drawn();
    let now = Instant::now();
    perf.last_frame = Some(now);
    perf.dirty = false;
    assert_eq!(perf.frame(now + ms(16)), Some(ms(16)));
    assert!(perf.dirty, "the next save writes it");
    perf.dirty = false;
    assert_eq!(perf.frame(now + ms(1016)), Some(ms(1000)));
    assert!(!perf.dirty, "a resting gap is no UI frame time");
}

#[test]
fn background_hardware_details_preserve_recorded_metrics() {
    let mut perf = Perf::new("test");
    perf.keep_local_log(false);
    perf.record("launcher.start_ms", 125.);
    let (sender, receiver) = mpsc::channel();
    perf.hardware = Some(receiver);
    assert_eq!(perf.report().metrics["launcher.start_ms"].mean, 125.);
    sender
        .send(Machine {
            hardware: Hardware {
                cpu: Some("Detected CPU".into()),
                ..Hardware::default()
            },
            power: Some("ac".into()),
            load_average: Some(0.5),
        })
        .unwrap();
    let report = perf.report();
    assert_eq!(report.hardware.cpu.as_deref(), Some("Detected CPU"));
    assert_eq!(report.conditions.power.as_deref(), Some("ac"));
    assert_eq!(report.metrics["launcher.start_ms"].mean, 125.);
    assert!(perf.hardware.is_none());
}

#[test]
fn a_stall_after_the_final_scroll_draw_begins_is_still_recorded() {
    let mut bench = Bench::new(PathBuf::from("/dev/null"), List::Builds);
    let t0 = Instant::now();
    bench.scroll = Some(Scroll::opened(t0));
    // First laid-out frame starts the timed scroll phase and dispatches a wheel step.
    let step = bench.after_frame(
        t0,
        Target::Laid {
            offset: 0.,
            max: 200.,
        },
    );
    assert!(step.is_some());
    // The final draw begins right before the deadline (small gap), then stalls for 250 ms
    // during the draw before `after_frame` confirms movement past SCROLLING.
    let draw_start = t0 + SCROLLING - ms(5);
    bench.frame(draw_start, Some(ms(16)));
    let draw_done = draw_start + ms(250);
    let next = bench.after_frame(
        draw_done,
        Target::Laid {
            offset: -24.,
            max: 200.,
        },
    );
    assert!(next.is_none(), "scroll phase ends after the deadline");
    assert!(bench.ready_for_idle());
    assert_eq!(bench.scroll_samples().len(), 1);
    assert!(
        bench.scroll_samples()[0] >= 250.,
        "draw stall after final draw began must be recorded, got {:?}",
        bench.scroll_samples()
    );
}

#[test]
fn terminal_summary_matches_report_frames_percentiles() {
    let frames = Frames::of("pages", 0, &[10.0, 100.0], &[], &[]).unwrap();
    let line = summary(Some(&frames));
    assert_eq!(
        line,
        format!(
            "{} frames, median {:.1} ms, p95 {:.1} ms",
            frames.frames, frames.p50_ms, frames.p95_ms
        )
    );
    assert!(
        line.contains("p95 100.0 ms"),
        "p95 must match report Frames: {line}"
    );
}

/// A bench whose scrolling is over and that has one local install waiting.
fn ready_install(name: &str) -> (Perf, Bench) {
    let perf = drawn();
    let mut bench = Bench::new(out(name), List::Builds);
    bench.install = Some(Install {
        smod: PathBuf::from("bench-install.smod"),
        home: PathBuf::from("scratch-profile"),
        began: None,
        outcome: None,
    });
    bench.began = Some(Instant::now() - SWITCHING);
    bench.scroll = Some(Scroll::opened(Instant::now()));
    bench.scroll.as_mut().unwrap().ended = true;
    (perf, bench)
}

fn finish_install(bench: &mut Bench, perf: &Perf, outcome: Option<String>) -> serde_json::Value {
    assert!(matches!(bench.tick(perf), Step::Install));
    let (smod, home) = bench.start_install().unwrap();
    assert!(smod.ends_with("bench-install.smod"));
    assert!(home.ends_with("scratch-profile"));
    assert!(matches!(bench.tick(perf), Step::Wait), "the install runs");
    bench.installed(outcome);
    assert!(matches!(bench.tick(perf), Step::Wait), "resting begins");
    bench.idle_cpu.as_mut().unwrap().0 = Instant::now() - IDLE;
    assert!(matches!(bench.tick(perf), Step::Quit));
    let result = serde_json::from_str(&std::fs::read_to_string(&bench.out).unwrap()).unwrap();
    let _ = std::fs::remove_file(&bench.out);
    result
}

#[test]
fn a_local_install_is_offered_once_and_its_time_is_recorded() {
    let (perf, mut bench) = ready_install("install");
    let result = finish_install(&mut bench, &perf, None);
    assert!(result["install_ms"].as_f64().is_some());
    assert!(result["install_skipped"].is_null());
}

#[test]
fn a_failed_local_install_is_recorded_without_failing_the_benchmark() {
    let (perf, mut bench) = ready_install("install-failed");
    let result = finish_install(&mut bench, &perf, Some("no room".into()));
    assert!(result["install_ms"].is_null());
    assert_eq!(result["install_skipped"], "no room");
}
