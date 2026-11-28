use super::{
    version_storage, InstallOutcome, InstallPhase, Job, LauncherCheckError, Links,
    NightlyReceiptWarning, PlayStep, Release, StageUpdateError, Staged, Updates,
};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

pub enum Msg {
    Playing(Option<u64>),
    Storage(
        crate::storage::Cancel,
        Result<Box<crate::storage::Report>, String>,
    ),
    Releases(Result<Vec<Release>, String>),
    Links(Links),
    Progress(String, InstallPhase),
    Installed(String, Result<InstallOutcome, String>),
    NightlyReceiptRetried(Arc<NightlyReceiptWarning>, Result<(), String>),
    /// A Storage removal finished; any failed removal names its build.
    Removed(Vec<(String, String)>),
    /// Automatic cleanup finished; any failed removal names its build.
    Pruned(Vec<(String, String)>),
    BuildInventory(u64, Vec<version_storage::Build>),
    Line(Job, String),
    Exit(Job, Result<(), String>),
    /// A relayed game acknowledged creation, but its actual exit was not observed.
    ClientObservationLost(String),
    /// Boxed: two releases, the largest message by far.
    Updates(Box<Updates>),
    LauncherChecked(Box<Result<Option<Release>, LauncherCheckError>>),
    LauncherProgress(f32),
    LauncherUpdated(Result<Option<Staged>, StageUpdateError>),
    /// `reuse_prepared_everywhere` prepared these versions.
    Reused(Vec<(String, String)>),
    /// None when secrets are kept in the OS keychain, else what to say: they last only until
    /// the launcher closes, and why.
    Keychain(Option<String>),
    /// Whether an account in the launcher owned the game at its last check (the owner gate).
    Owner(bool),
    /// Credential-free result from a background account operation.
    AccountStatus(String, u64, crate::account::session::Status),
    AccountProgress(String, u64, crate::account::session::Phase),
    AccountHead(String, Option<String>, Option<Vec<[u8; 4]>>, bool),
    AccountRemoved(String, u64, Result<bool, String>),
    /// What account `.0` wears, read for the Skins tab by the request numbered `.1`.
    SkinsProfile(String, u64, Result<crate::account::capes::Profile, String>),
    /// A skin's or cape's picture from Mojang's texture server, by its URL (none: it couldn't be
    /// had).
    Picture(String, Option<Vec<u8>>),
    /// A Save on the Skins tab for account `.0`, numbered `.1`, ended.
    LookSaved(String, u64, Box<crate::account::look::Outcome>),
    /// Skin files added to account `.0`'s saved skins: each one, or why not.
    SkinsAdded(
        String,
        Vec<Result<crate::skin_library::Saved, String>>,
        crate::skin_library::Listing,
    ),
    /// Account `.0`'s saved skins, listed.
    SkinsListed(String, crate::skin_library::Listing),
    /// What a later profile confirmed of account `.0`'s last Save, recorded in accounts.json, or
    /// why not.
    LookRecorded(String, crate::account::look::Record, Result<(), String>),
    /// The saved skin `.1` of account `.0` removed, or why not, and the list after.
    SkinRemoved(
        String,
        String,
        Result<(), String>,
        crate::skin_library::Listing,
    ),
    /// A saved or default skin's file for the Skins tab, by the page's key for it and the
    /// number of the read that asked for it (none: it couldn't be read).
    SkinFile(String, u64, Option<Vec<u8>>),
    /// The skin account `.0` wears, at URL `.1`, kept among its saved skins, or why not, and the
    /// list after.
    SkinKept(
        String,
        String,
        Result<crate::skin_library::Saved, String>,
        crate::skin_library::Listing,
    ),
    /// The owner gate's answer for the game about to start: the selected account's session
    /// file (none = offline) and why it plays offline, if it does.
    Session(Result<Option<crate::account::GameSession>, crate::account::NoOwner>),
    /// The step Play is at before the game starts, for the button's label.
    PlayStep(PlayStep),
    /// How far Play's download of the game build is, for the status strip's row.
    PlayDownload(f32),
    /// Live detail line under "Preparing…" while Play is working (e.g. file counts).
    PlayDetail(String),
    ManagedBuild {
        instance: String,
        profile: String,
        base: String,
        mods: Vec<crate::native_status::BuildMod>,
    },
    ManagedBuildLog(std::sync::Arc<str>),
    ManagedClient {
        instance: Box<crate::instances::Instance>,
        result: Box<Result<crate::managed_play::Started, crate::managed_play::StartError>>,
    },
    /// The exit worker has tried to save the completed profile's play time.
    PlayRecorded,
    /// The exit worker finished, including the profile's post-exit command.
    GameExitSettled,
    /// The worker checked the game's journal for the selected profile before Play changed it.
    PlayAdmission(
        PathBuf,
        Result<(), crate::browser::installed::GameInstallProblem>,
    ),
    SwitchProgress(crate::browser::sunrise::InstallProgress),
    /// A counted line from inside a switch's step ("Checking the game files: 120 of 4000"), from
    /// the play-detail sink the switch worker holds while it runs.
    SwitchDetail(String),
    SwitchQuestion(
        Box<crate::browser::sunrise::SwitchQuestion>,
        std::sync::mpsc::Sender<crate::browser::sunrise::SwitchDecision>,
    ),
    Switched(
        PathBuf,
        Box<Result<crate::browser::sunrise::Switched, String>>,
    ),
    ProviderSwitchProgress(crate::browser::sunrise::InstallProgress),
    /// The staged switch is proved; `true` publishes it.
    ProviderSwitchQuestion(
        Box<crate::browser::sunrise::ProviderSwitchPreview>,
        std::sync::mpsc::Sender<bool>,
    ),
    /// `Ok(true)` when the profile now uses the new port, `Ok(false)` when it stayed.
    ProviderSwitched(PathBuf, Result<bool, String>),
    /// Several reviewed projects provide a required dependency; the reply names the chosen one.
    ProviderChoice(
        Box<crate::browser::sunrise::ProviderChoiceNeeded>,
        std::sync::mpsc::Sender<Option<String>>,
    ),
    /// A release's verified publisher changed; `true` accepts the new one.
    PublisherChangeQuestion(
        Box<crate::browser::sunrise::PublisherChangeQuestion>,
        std::sync::mpsc::Sender<bool>,
    ),
    ReceiptAcknowledged(PathBuf, Result<bool, String>),
    JarConsent {
        jars: Vec<sunrise_launcher_smod::manifest::JarRef>,
        reply: std::sync::mpsc::Sender<bool>,
    },
    JarClient {
        run: Option<Box<crate::instances::Instance>>,
        account: Option<String>,
        session_file: Option<PathBuf>,
        registry_notices: Vec<String>,
        prepared: Box<Result<Command, String>>,
    },
    /// Where the native-mods build is.
    NativeProgress(crate::native_mods::Step),
    /// A build reached native mods that neither came verified from the registry nor were
    /// confirmed: the unreviewed-code warning answers `reply` (true builds them).
    ConfirmUnreviewed(
        Vec<crate::native_mods::Unreviewed>,
        std::sync::mpsc::Sender<bool>,
    ),
    /// The native-mods build ended: the version built, or why not.
    Native(Result<String, crate::native_mods::Failed>),
    /// An imported game build is installed (`build_import::install`): its version, or why not.
    Imported(Result<String, String>),
}
