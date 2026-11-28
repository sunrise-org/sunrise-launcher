//! The window's message pump (`Launcher::drain`) runs as soon as a worker says something, not on
//! its next tick: a search's results, a progress step or a toast shows on the frame it arrives.
//! The tick stays for what only time changes (a toast running out, the server list's polls, the
//! update check).

use std::sync::mpsc::{Receiver, Sender};
use std::sync::LazyLock;
use std::time::Duration;
use tokio::sync::Notify;

/// How often the pump runs when nothing arrives.
pub(crate) const TICK: Duration = Duration::from_millis(80);

/// The shortest time between two drains: a burst (a download's progress, a preparer's log) is
/// drained together, about once a frame, instead of once a message.
pub(crate) const SETTLE: Duration = Duration::from_millis(8);

static ARRIVED: LazyLock<Notify> = LazyLock::new(Notify::new);

/// Runs the pump now. Wakes that come while it runs make it run once more, never more.
pub(crate) fn wake() {
    #[cfg(test)]
    if UNDER_TEST_SCHEDULER.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    ARRIVED.notify_one();
}

/// Set by a window test (a `gpui::TestAppContext` in its own process): GPUI's test scheduler
/// refuses a foreground task woken from another thread, panicking in `assert_correct_thread`,
/// which on this module's 64 KiB thread shows only as a stack overflow. Such a test drives the
/// pump's tick with its own clock instead.
#[cfg(test)]
pub(crate) static UNDER_TEST_SCHEDULER: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Waits for the next [`wake`].
pub(crate) async fn woken() {
    ARRIVED.notified().await;
}

/// `std::sync::mpsc::channel()`, whose messages each [`wake`] the pump as they arrive. A small
/// thread passes them on; without one, the pump's tick still reads them.
pub(crate) fn channel<T: Send + 'static>() -> (Sender<T>, Receiver<T>) {
    let (tx, from_workers) = std::sync::mpsc::channel::<T>();
    let (to_pump, rx) = std::sync::mpsc::channel();
    let passing = std::thread::Builder::new()
        .name("launcher-wake".into())
        .stack_size(64 * 1024)
        .spawn(move || {
            for message in from_workers {
                if to_pump.send(message).is_err() {
                    break;
                }
                wake();
            }
        });
    match passing {
        Ok(_) => (tx, rx),
        Err(_) => std::sync::mpsc::channel(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_arrives_and_wakes_the_pump() {
        let (tx, rx) = channel();
        tx.send(7).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok(7));
        // The wake follows the message: the pump's wait returns without its tick.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while futures_util::FutureExt::now_or_never(woken()).is_none() {
            assert!(
                std::time::Instant::now() < deadline,
                "the pump was not woken"
            );
            std::thread::yield_now();
        }
    }
}
