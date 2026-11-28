//! The CLI and Storage page share one fixed-path inventory.
mod count;
mod disk_walk;
#[cfg(test)]
mod regressions;
mod report_clearing;
use super::*;
use crate::{backend, instances, native_mods};
use serde::Serialize;
use std::sync::Arc;

use disk_walk::*;
use report_clearing::*;

// Only files the game can regenerate from authoritative inputs may be offered here.
// Both behaviours cache versions are named explicitly: v8 is written by live 26.3+
// builds and v7 by older installed builds. Never replace with a pattern.
const PROFILE_GAME_CACHES: [&str; 6] = [
    ".sunrise-cache/behaviours-v8.bin",
    ".sunrise-cache/behaviours-v7.bin",
    ".sunrise-cache/registry-v1.bin",
    ".sunrise-cache/mod-content",
    ".sunrise-cache/resourcepacks",
    "cache/dx12-shaders.bin",
];

const BUILD_CACHES: [BuildCache; 11] = [
    BuildCache::Source,
    BuildCache::Tools,
    BuildCache::PrebuiltOutput,
    BuildCache::InternalsSync,
    BuildCache::Prebuilt,
    BuildCache::ServerBuilds,
    BuildCache::KeptMods,
    BuildCache::ModBuildCache,
    BuildCache::Rust,
    BuildCache::Cargo,
    BuildCache::Scratch,
];

/// Whether a measured row has candidates for an explicit Clear. A candidate is not a promise
/// that the live exclusion checks at deletion time will pass.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Eligibility {
    NotMeasured,
    Empty,
    Candidate,
    WhollyRefused,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeptKind {
    Busy,
    OlderBuild,
    Incomplete,
    Unpublished,
    ReferencedProfile,
    UnprovenReferences,
    ResourceDependent,
    UnsupportedFilesystem,
    Unsafe,
    Unreadable,
}

#[derive(Clone, Serialize)]
pub struct Row {
    pub category: Category,
    pub bytes: u64,
    pub freed_bytes: u64,
    pub remaining_bytes: u64,
    #[serde(skip)]
    measured: bool,
    #[serde(skip)]
    has_candidates: bool,
    #[serde(skip)]
    pub after_clear: bool,
    #[serde(skip)]
    pub older_build_kept: bool,
    /// Refused or busy units are retained, with their allocated bytes and a visible reason.
    pub kept: Vec<Kept>,
}

impl Row {
    fn new(category: Category) -> Self {
        Self {
            category,
            bytes: 0,
            freed_bytes: 0,
            remaining_bytes: 0,
            measured: false,
            has_candidates: false,
            after_clear: false,
            older_build_kept: false,
            kept: Vec::new(),
        }
    }

    /// The measured remainder, if this row was included in the operation.
    pub fn measured_bytes(&self) -> Option<u64> {
        self.measured.then_some(self.remaining_bytes)
    }

    pub fn eligibility(&self) -> Eligibility {
        if !self.measured {
            Eligibility::NotMeasured
        } else if self.has_candidates {
            Eligibility::Candidate
        } else if self.kept.is_empty() {
            Eligibility::Empty
        } else {
            Eligibility::WhollyRefused
        }
    }

    /// Byte upper bound; not a replacement for `eligibility()` (a link still occupies a slot).
    pub fn clearable_bytes(&self) -> u64 {
        if self.has_candidates {
            self.remaining_bytes
        } else {
            0
        }
    }

    pub fn only_kept_kind(&self, kind: KeptKind) -> bool {
        !self.kept.is_empty() && self.kept.iter().all(|unit| unit.kind == kind)
    }

    fn keep(&mut self, bytes: u64, error: Error, log: bool) {
        let kind = match &error {
            Error::Busy => KeptKind::Busy,
            Error::UnprovenExclusion => KeptKind::OlderBuild,
            Error::IncompleteUnit => KeptKind::Incomplete,
            Error::UnpublishedGame => KeptKind::Unpublished,
            Error::ReferencedProfile => KeptKind::ReferencedProfile,
            Error::UnprovenReferences => KeptKind::UnprovenReferences,
            Error::ResourceDependent(_) => KeptKind::ResourceDependent,
            Error::UnsupportedFilesystem => KeptKind::UnsupportedFilesystem,
            Error::Unsafe(_) => KeptKind::Unsafe,
            Error::Io(_) => KeptKind::Unreadable,
        };
        let detail = error_log_detail(&error);
        let reason = error_message(error);
        if log {
            match detail {
                Some(detail) => crate::launcher_log::line(format!(
                    "Storage Clear kept {:?} unit ({bytes} bytes): {reason} [{detail}]",
                    self.category
                )),
                None => crate::launcher_log::line(format!(
                    "Storage Clear kept {:?} unit ({bytes} bytes): {reason}",
                    self.category
                )),
            }
        }
        self.kept.push(Kept {
            bytes,
            reason,
            kind,
        });
        self.older_build_kept = self.only_kept_kind(KeptKind::OlderBuild);
    }
}

#[derive(Clone, Serialize)]
pub struct Kept {
    pub bytes: u64,
    pub reason: String,
    #[serde(skip)]
    pub kind: KeptKind,
}

/// The rows counted or cleared so far: all of them once it [`is_complete`](Self::is_complete).
#[derive(Clone, Serialize)]
pub struct Report {
    pub rows: Vec<Row>,
    pub total_bytes: u64,
    /// The count or Clear this report came from, so a window can drop one it has cancelled.
    #[serde(skip)]
    origin: Option<Cancel>,
}

impl Report {
    fn new(rows: Vec<Row>) -> Self {
        Self {
            total_bytes: rows
                .iter()
                .fold(0u64, |total, row| total.saturating_add(row.bytes)),
            rows,
            origin: None,
        }
    }

    /// Whether every row is in: a count in progress hands over the rows it has finished.
    pub fn is_complete(&self) -> bool {
        self.rows.len() == Category::ALL.len()
    }

    pub fn tagged(mut self, origin: &Cancel) -> Self {
        self.origin = Some(origin.clone());
        self
    }

    pub fn is_from(&self, origin: &Cancel) -> bool {
        self.origin.as_ref().is_some_and(|own| own.is(origin))
    }

    fn push(&mut self, row: Row) {
        self.total_bytes = self.total_bytes.saturating_add(row.bytes);
        self.rows.push(row);
    }
}

impl Category {
    pub const ALL: [Self; 6] = [
        Self::BuildFiles,
        Self::PrebuiltGames,
        Self::SpareGameBuilds,
        Self::RustCompiler,
        Self::ModBuildCache,
        Self::OtherCaches,
    ];
}
