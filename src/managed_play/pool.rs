//! One job per item on scoped threads, for the passes that touch every file of a snapshot.
use crate::native_mods::Cancellation;
use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Mutex, PoisonError,
    },
};

/// The device's parallelism: the number of workers a pass uses.
pub(super) fn workers() -> usize {
    std::thread::available_parallelism().map_or(1, usize::from)
}

/// Runs `job` on every item with `workers` threads and returns the results in item order.
/// Each worker makes one `S` with `init` and reuses it for its items. The first error (or a
/// stop, checked before each item) stops every worker, and all of them are joined before this
/// returns. A panic in a worker is that error too: the scope would otherwise raise it again on
/// the caller's thread.
pub(super) fn run<T: Sync, S, R: Send>(
    items: &[T],
    workers: usize,
    stop: Option<&Cancellation>,
    init: impl Fn() -> S + Sync,
    job: impl Fn(&mut S, &T) -> Result<R, String> + Sync,
) -> Result<Vec<R>, String> {
    run_with_progress(items, workers, stop, None, init, job)
}

pub(super) fn run_with_progress<T: Sync, S, R: Send>(
    items: &[T],
    workers: usize,
    stop: Option<&Cancellation>,
    on_done: Option<&(dyn Fn(usize, usize) + Sync)>,
    init: impl Fn() -> S + Sync,
    job: impl Fn(&mut S, &T) -> Result<R, String> + Sync,
) -> Result<Vec<R>, String> {
    let cursor = AtomicUsize::new(0);
    let completed = AtomicUsize::new(0);
    let failed = AtomicBool::new(false);
    let first = Mutex::new(None::<String>);
    let fail = |error: String| {
        failed.store(true, Ordering::Relaxed);
        first
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert(error);
    };
    let work = || {
        let mut done = Vec::new();
        let mut state = init();
        while !failed.load(Ordering::Relaxed) {
            if let Some(Err(error)) = stop.map(Cancellation::check) {
                fail(error);
                break;
            }
            let at = cursor.fetch_add(1, Ordering::Relaxed);
            let Some(item) = items.get(at) else {
                break;
            };
            match job(&mut state, item) {
                Ok(result) => {
                    done.push((at, result));
                    if let Some(on_done) = on_done {
                        let n = completed.fetch_add(1, Ordering::Relaxed) + 1;
                        on_done(n, items.len());
                    }
                }
                Err(error) => fail(error),
            }
        }
        done
    };
    // Workers log for the operation (a switch's sealing) their caller works for.
    let op = crate::launcher_log::current();
    let finished = std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for _ in 0..workers.clamp(1, items.len().max(1)) {
            match std::thread::Builder::new().spawn_scoped(scope, || {
                let _in = crate::launcher_log::enter(&op);
                catch_unwind(AssertUnwindSafe(&work)).unwrap_or_else(|_| {
                    fail("a worker thread panicked".into());
                    Vec::new()
                })
            }) {
                Ok(handle) => handles.push(handle),
                Err(error) => {
                    fail(format!("could not start worker thread: {error}"));
                    break;
                }
            }
        }
        let mut finished = Vec::new();
        for handle in handles {
            match handle.join() {
                Ok(done) => finished.extend(done),
                Err(_) => fail("a worker thread panicked".into()),
            }
        }
        finished
    });
    if let Some(error) = first.into_inner().unwrap_or_else(PoisonError::into_inner) {
        return Err(error);
    }
    let mut slots: Vec<Option<R>> = items.iter().map(|_| None).collect();
    for (at, result) in finished {
        slots[at] = Some(result);
    }
    Ok(slots
        .into_iter()
        .map(|slot| slot.expect("every item ran when no worker failed"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_keep_item_order_for_any_worker_count() {
        let items: Vec<usize> = (0..200).collect();
        for workers in [1, 2, 7] {
            let doubled = run(&items, workers, None, || (), |_, n| Ok(n * 2)).unwrap();
            assert_eq!(doubled, items.iter().map(|n| n * 2).collect::<Vec<_>>());
        }
    }

    #[test]
    fn a_worker_panic_is_an_error_not_a_panic_on_the_caller() {
        let items: Vec<usize> = (0..50).collect();
        for workers in [1, 2] {
            let error = run(
                &items,
                workers,
                None,
                || (),
                |_, n| {
                    if *n == 20 {
                        panic!("injected");
                    }
                    Ok(())
                },
            )
            .unwrap_err();
            assert!(error.contains("panicked"), "{error}");
        }
    }

    #[test]
    fn the_first_error_stops_the_other_workers() {
        let items: Vec<usize> = (0..10_000).collect();
        let ran = AtomicUsize::new(0);
        let error = run(
            &items,
            2,
            None,
            || (),
            |_, n| {
                ran.fetch_add(1, Ordering::Relaxed);
                if *n == 3 {
                    return Err("refused".to_string());
                }
                Ok(())
            },
        )
        .unwrap_err();
        assert_eq!(error, "refused");
        assert!(ran.load(Ordering::Relaxed) < items.len());
    }

    #[test]
    fn a_stopped_attempt_returns_the_cancellation() {
        let stop = Cancellation::default();
        stop.stop();
        let items = [1, 2, 3];
        let error = run(&items, 2, Some(&stop), || (), |_, _| Ok(())).unwrap_err();
        assert_eq!(Some(error), stop.check().err());
    }
}
