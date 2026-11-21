//! The immutable plan is never enlarged or replaced during publication or undo. Write one
//! checksummed slot at a time in the file allocated during seal; a torn update leaves the
//! previous complete slot. This avoids allocating another journal file under ENOSPC, not a
//! promise that an in-place update cannot fail (e.g. on a copy-on-write filesystem).
use super::{at, durable, regular, unsettled};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

const NAME: &str = "progress";
const BLOCK: usize = 4096;
const MAGIC: &[u8; 8] = b"SRTXN\0\x02\0";
const CHECKSUM_AT: usize = BLOCK - 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(super) enum Phase {
    Ready = 0,
    Publishing,
    Committed,
    Undoing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct State {
    sequence: u64,
    pub(super) phase: Phase,
    pub(super) frontier: usize,
}

impl State {
    const READY: Self = Self {
        sequence: 0,
        phase: Phase::Ready,
        frontier: 0,
    };

    fn follows(self, before: Self, steps: usize) -> bool {
        use Phase::*;
        before.sequence.checked_add(1) == Some(self.sequence)
            && match (before.phase, self.phase) {
                (Ready | Publishing, Publishing) => {
                    before.frontier.checked_add(1) == Some(self.frontier)
                }
                (Publishing, Committed) => before.frontier == steps && self.frontier == steps,
                (Publishing, Undoing) => before.frontier == self.frontier,
                (Undoing, Undoing) => self.frontier.checked_add(1) == Some(before.frontier),
                _ => false,
            }
    }

    fn encode(self, plan: [u8; 32]) -> [u8; BLOCK] {
        let mut block = [0u8; BLOCK];
        block[..8].copy_from_slice(MAGIC);
        block[8..40].copy_from_slice(&plan);
        block[40..48].copy_from_slice(&self.sequence.to_le_bytes());
        block[48] = self.phase as u8;
        block[49..57].copy_from_slice(&(self.frontier as u64).to_le_bytes());
        let digest = Sha256::digest(&block[..CHECKSUM_AT]);
        block[CHECKSUM_AT..].copy_from_slice(&digest);
        block
    }

    // Only a checksum mismatch is a torn write. A complete but foreign/new-schema slot
    // is evidence we don't own, and must not be silently downgraded to the other slot.
    fn decode(
        block: &[u8; BLOCK],
        plan: [u8; 32],
        slot: u64,
        steps: usize,
    ) -> Result<Option<Self>, String> {
        if Sha256::digest(&block[..CHECKSUM_AT])[..] != block[CHECKSUM_AT..] {
            return Ok(None);
        }
        if &block[..8] != MAGIC || block[8..40] != plan {
            return Err(unsettled());
        }
        let sequence = u64::from_le_bytes(block[40..48].try_into().expect("eight bytes"));
        let frontier = usize::try_from(u64::from_le_bytes(
            block[49..57].try_into().expect("eight bytes"),
        ))
        .map_err(|_| unsettled())?;
        let phase = match block[48] {
            0 if sequence == 0 && frontier == 0 => Phase::Ready,
            1 if sequence == frontier as u64 && (1..=steps).contains(&frontier) => {
                Phase::Publishing
            }
            2 if sequence == steps as u64 + 1 && frontier == steps => Phase::Committed,
            3 if sequence > 1 && frontier <= steps => Phase::Undoing,
            _ => return Err(unsettled()),
        };
        if sequence % 2 != slot {
            return Err(unsettled());
        }
        Ok(Some(Self {
            sequence,
            phase,
            frontier,
        }))
    }
}

pub(super) struct Slots {
    file: File,
    path: PathBuf,
    plan: [u8; 32],
    state: State,
    steps: usize,
}

impl Slots {
    pub(super) fn create(stage: &Path, plan: [u8; 32]) -> io::Result<()> {
        let mut bytes = [0u8; 2 * BLOCK];
        bytes[..BLOCK].copy_from_slice(&State::READY.encode(plan));
        // Write a nonzero second block so a common sparse allocation optimization does
        // not skip its storage. It is not a state until a checksum-valid update lands.
        bytes[BLOCK..].fill(0xff);
        durable::replace(stage, &stage.join(NAME), &bytes)
    }

    pub(super) fn open(stage: &Path, plan: [u8; 32], steps: usize) -> Result<Self, String> {
        let path = stage.join(NAME);
        let meta = std::fs::symlink_metadata(&path).map_err(|e| at(&path, e))?;
        if !regular(&meta) || !meta.is_file() || meta.len() != (2 * BLOCK) as u64 {
            return Err(at(&path, unsettled()));
        }
        let mut file = File::options()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|e| at(&path, e))?;
        let mut read = |i| {
            let mut block = [0u8; BLOCK];
            file.read_exact(&mut block).map_err(|e| at(&path, e))?;
            State::decode(&block, plan, i, steps).map_err(|e| at(&path, e))
        };
        let state = match (read(0)?, read(1)?) {
            (None, Some(state)) | (Some(state), None) => state,
            (Some(a), Some(b)) if a.follows(b, steps) => a,
            (Some(a), Some(b)) if b.follows(a, steps) => b,
            _ => return Err(at(&path, unsettled())),
        };
        Ok(Self {
            file,
            path,
            plan,
            state,
            steps,
        })
    }

    pub(super) fn current(&self) -> State {
        self.state
    }

    pub(super) fn sync(&self) -> Result<(), String> {
        self.flush(self.state.phase)
    }

    fn flush(&self, _phase: Phase) -> Result<(), String> {
        #[cfg(test)]
        if _phase == Phase::Committed
            && take_marker_fault(|fault| match fault {
                MarkerFault::Flush(remaining) if *remaining != 0 => {
                    *remaining -= 1;
                    true
                }
                _ => false,
            })
        {
            return Err(at(&self.path, storage_full()));
        }
        durable::flush(&self.file).map_err(|e| at(&self.path, e))
    }

    pub(super) fn advance(&mut self, phase: Phase, frontier: usize) -> Result<(), String> {
        self.advance_at(phase, frontier, || Ok(()))
    }

    pub(super) fn advance_at(
        &mut self,
        phase: Phase,
        frontier: usize,
        written: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        let state = State {
            sequence: self.state.sequence.checked_add(1).ok_or_else(unsettled)?,
            phase,
            frontier,
        };
        if frontier > self.steps || !state.follows(self.state, self.steps) {
            return Err(unsettled());
        }
        self.file
            .seek(SeekFrom::Start((state.sequence % 2) * BLOCK as u64))
            .map_err(|e| at(&self.path, e))?;
        let block = state.encode(self.plan);
        #[cfg(test)]
        if phase == Phase::Committed
            && take_marker_fault(|fault| {
                if *fault == MarkerFault::TornWrite {
                    *fault = MarkerFault::None;
                    true
                } else {
                    false
                }
            })
        {
            self.file
                .write_all(&block[..57])
                .map_err(|e| at(&self.path, e))?;
            return Err(at(&self.path, storage_full()));
        }
        self.file.write_all(&block).map_err(|e| at(&self.path, e))?;
        written()?;
        self.flush(phase)?;
        self.state = state;
        Ok(())
    }
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum MarkerFault {
    #[default]
    None,
    TornWrite,
    Flush(u8),
}

#[cfg(test)]
thread_local! {
    static MARKER_FAULT: std::cell::Cell<MarkerFault> = const { std::cell::Cell::new(MarkerFault::None) };
}

#[cfg(test)]
pub(super) fn fail_marker(fault: MarkerFault) {
    MARKER_FAULT.set(fault);
}

#[cfg(test)]
fn take_marker_fault(test: impl FnOnce(&mut MarkerFault) -> bool) -> bool {
    let mut fault = MARKER_FAULT.get();
    let inject = test(&mut fault);
    MARKER_FAULT.set(fault);
    inject
}

#[cfg(test)]
pub(super) fn storage_full() -> io::Error {
    #[cfg(unix)]
    let code = libc::ENOSPC;
    #[cfg(windows)]
    let code = windows_sys::Win32::Foundation::ERROR_DISK_FULL as i32;
    io::Error::from_raw_os_error(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_torn_commit_keeps_the_undo_frontier_but_foreign_evidence_does_not_downgrade() {
        let tmp = tempfile::tempdir().unwrap();
        let stage = tmp.path();
        let digest = [7; 32];
        Slots::create(stage, digest).unwrap();
        let mut slots = Slots::open(stage, digest, 2).unwrap();
        slots.advance(Phase::Publishing, 1).unwrap();
        slots.advance(Phase::Publishing, 2).unwrap();
        let commit = State {
            sequence: 3,
            phase: Phase::Committed,
            frontier: 2,
        };
        slots.file.seek(SeekFrom::Start(BLOCK as u64)).unwrap();
        slots.file.write_all(&commit.encode(digest)[..57]).unwrap();
        drop(slots);
        let mut slots = Slots::open(stage, digest, 2).unwrap();
        assert_eq!(slots.current().frontier, 2);
        assert_eq!(slots.current().phase, Phase::Publishing);
        assert!(slots.advance(Phase::Undoing, 1).is_err());
        slots.file.seek(SeekFrom::Start(BLOCK as u64)).unwrap();
        slots.file.write_all(&commit.encode([8; 32])).unwrap();
        drop(slots);
        assert!(Slots::open(stage, digest, 2).is_err());
    }
}
