//! The outside addresses the launcher has built in, vanilla's aside, in one place.

/// The mod registry's policy, which the launcher's mod pages link.
pub const MOD_POLICY: &str = "https://playsunrise.net/mods/policy";

/// The mod registry's index, published by sunrise-org/sunrise-mods (`SUNRISE_MOD_INDEX` overrides it).
pub const MOD_INDEX: &str = "https://sunrise-org.github.io/sunrise-mods/index.json";

/// The registry's Nightly source listings. A listing does not vet the source or authorize code.
pub const MOD_SOURCES: &str = "https://sunrise-org.github.io/sunrise-mods/sources.json";

/// Where the registry's icons and screenshots are served from: the same GitHub Pages site.
pub const MOD_IMAGES: &str = "https://sunrise-org.github.io/sunrise-mods/";

/// A registry project's page on the website is `<this>/<id>`.
pub const MOD_PAGES: &str = "https://playsunrise.net/mods";

/// The registry's form for reporting a project.
pub const MOD_REPORT: &str =
    "https://github.com/sunrise-org/sunrise-mods/issues/new?template=report.yml";

/// The telemetry ingest server (`crates/sunrise-telemetry-server` in the game repository): opt-in
/// telemetry, the "I'm playing" ping, benchmark uploads (`SUNRISE_TELEMETRY_URL` overrides it).
pub const TELEMETRY: &str = "https://telemetry.playsunrise.net";

/// The GitHub profile of who made the launcher's look, credited in Settings' About (its picture is
/// `<this>.png`).
pub const LAUNCHER_LOOK_CREDIT: &str = "https://github.com/Xmilon";

/// The official launcher installer's product home.
pub const LAUNCHER_ABOUT: &str = "https://playsunrise.net";

/// Where the game repository publishes each game build's mod SDK (`sdk-<build id>`) and the game
/// releases whose `sdk.json` names their build; a release's tag and file follow.
/// `SUNRISE_SDK_BASE` overrides it.
pub const SDK_RELEASES: &str = "https://github.com/sunrise-org/sunrise/releases/download";
