//! Installs queued behind the active operation. Each keeps its original profile and game.
use std::collections::VecDeque;
use std::path::PathBuf;
use sunrise_launcher::browser::{Project, SourceId};

/// The profile an install or build was asked for.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Target {
    pub(crate) instance: String,
    pub(super) home: PathBuf,
    /// The Minecraft version that profile plays.
    pub(super) game: String,
}

/// An install that was asked for and waits its turn.
pub(crate) enum Request {
    One {
        target: Target,
        source: SourceId,
        id: String,
        known: Option<Box<Project>>,
        /// The title shown when this request was made, even if it later pauses for consent.
        shown: Option<String>,
    },
    /// A collection's mods and title.
    Many(Target, Vec<String>, String),
    Github(Target, crate::github_mods_page::GithubInput),
}

impl Request {
    /// The profile and game it is for.
    pub(crate) fn target(&self) -> &Target {
        match self {
            Request::One { target, .. }
            | Request::Many(target, ..)
            | Request::Github(target, ..) => target,
        }
    }
}

#[derive(Default)]
pub(super) struct Queue {
    waiting: VecDeque<Request>,
}

impl Queue {
    /// A request just asked for: it starts now (returned) unless an install or a build runs or
    /// others wait, and then it goes behind them. This is the only way in: what the queue hands
    /// out through [`Self::next`] starts without passing here again.
    pub(super) fn submit(&mut self, request: Request, busy: bool) -> Option<Request> {
        if busy || !self.waiting.is_empty() {
            self.waiting.push_back(request);
            None
        } else {
            Some(request)
        }
    }

    /// A running request that paused for consent was already ahead of every waiting request.
    pub(super) fn resume(&mut self, request: Request) {
        self.waiting.push_front(request);
    }

    /// Whether mod `id` waits for its turn to install.
    pub(super) fn waits(&self, id: &str) -> bool {
        self.waiting
            .iter()
            .any(|r| matches!(r, Request::One { id: waiting, .. } if waiting == id))
    }

    /// Nothing else starts while the current install, build, or game owns the slot.
    pub(super) fn next(&mut self, blocked: bool) -> Option<Request> {
        if blocked {
            None
        } else {
            self.waiting.pop_front()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(id: &str) -> Request {
        Request::One {
            target: Target {
                instance: "main".into(),
                home: "/home/main".into(),
                game: "26.2".into(),
            },
            source: SourceId::Sunrise,
            id: id.into(),
            known: None,
            shown: Some(id.into()),
        }
    }

    #[test]
    fn installs_run_in_order_after_the_busy_profile_is_released() {
        let mut queue = Queue::default();
        assert!(queue.submit(one("first"), false).is_some());
        assert!(queue.submit(one("second"), true).is_none());
        assert!(queue.submit(one("third"), false).is_none());
        assert!(queue.waits("second") && queue.waits("third"));
        assert!(queue.next(true).is_none());
        for expected in ["second", "third"] {
            assert!(matches!(queue.next(false), Some(Request::One { id, .. }) if id == expected));
        }
        assert!(queue.next(false).is_none());
    }

    #[test]
    fn a_paused_request_resumes_whole_before_work_queued_behind_it() {
        let mut queue = Queue::default();
        let Request::One {
            mut target,
            source,
            id,
            known,
            shown,
        } = one("Lumen")
        else {
            unreachable!()
        };
        target.instance = "other".into();
        target.home = "/home/other".into();
        target.game = "26.3".into();
        queue.submit(one("behind"), true);
        assert!(queue.next(true).is_none());
        queue.resume(Request::One {
            target: target.clone(),
            source,
            id,
            known,
            shown,
        });
        assert!(
            matches!(queue.next(false), Some(Request::One { target: actual, shown, .. })
            if actual == target && shown.as_deref() == Some("Lumen"))
        );
        assert!(matches!(queue.next(false), Some(Request::One { id, .. }) if id == "behind"));
    }
}
