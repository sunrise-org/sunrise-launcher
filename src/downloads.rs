//! Bounded, streaming HTTP transfers shared by the launcher's Minecraft pipeline.
//!
//! `SUNRISE_DOWNLOAD_*` settings are read once when the shared engine starts. Invalid values
//! retain defaults; `Config` also supports direct configuration. Sizes are bytes, times seconds.
//!
//! | Suffix | Default | Meaning / bounds |
//! | --- | ---: | --- |
//! | FILES | 16 | active files, 1–1024 |
//! | CONNECTIONS | 32 | global HTTP requests, 1–1024 |
//! | PER_HOST | 16 | requests per hostname, 1–1024 |
//! | SEGMENTS | 8 | range workers per large file, 1–16 |
//! | THRESHOLD | 33554432 | range-probe threshold (32 MiB) |
//! | MINIMUM_SEGMENT | 4194304 | minimum chunk (4 MiB), at least 1 |
//! | CONNECT_TIMEOUT | 10 | connection timeout, at least 1 |
//! | READ_TIMEOUT | 30 | read inactivity timeout, at least 1 |
//! | RETRIES | 5 | additional transient-error attempts |
//! | BUFFER | 131072 | buffer per stream, clamped to 64–256 KiB |
//!
//! Redirect targets must use HTTPS. Retry-After delays are capped at 60 seconds. Each destination
//! directory is swept once before use for generated partials whose owning process has exited.

mod metrics;
mod resume;
mod runtime;
mod transfer;

pub use metrics::*;
pub use runtime::*;
pub use transfer::*;

pub(crate) fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_else(|| "an unknown host".into())
}

pub const CANCELLED: &str = "Download cancelled. Download again to go on where it stopped.";

#[cfg(test)]
use self::{
    resume::{partial_path, sweep_partials},
    transfer::{redirect_target, retry_after},
};

#[cfg(test)]
mod tests;
