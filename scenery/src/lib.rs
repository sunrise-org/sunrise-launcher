//! The launcher's scenery: Sunrise's coast, the themes' painted skies and Hypr Neon's rain (each
//! a GPUI view of its own), the plates, and how they move. Kept apart from the launcher so its
//! drawing compiles once and the launcher's own changes rebuild less.
pub mod art;
pub mod clock;
pub mod coast;
pub mod glass;
pub mod model;
pub mod rain;
pub mod sky;
