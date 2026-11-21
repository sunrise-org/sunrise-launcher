use super::*;

pub enum Msg {
    Playing(Option<u64>),
    Storage(Result<Box<crate::storage::Report>, String>),
    Releases(Result<Vec<Release>, String>),
    Links(Links),
    Progress(String, f32),
    Installed(String, Result<(), String>),
    /// A version's files are gone (or could not be deleted).
    Removed(String, Result<(), String>),
    /// `prune_old_versions` ended; these versions were deleted.
    Pruned(Vec<String>),
    /// What each installed version takes on disk (`disk_use`).
    DiskUse(Vec<(String, u64)>),
    Line(Job, String),
    Exit(Job, Result<(), String>),
    /// A relayed game acknowledged creation, but its actual exit was not observed.
    ClientObservationLost(String),
    /// Boxed: two releases, the largest message by far.
    Updates(Box<Updates>),
    LauncherChecked(Box<Result<Option<Release>, String>>),
    LauncherProgress(f32),
    LauncherUpdated(Result<Option<Staged>, String>),
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
    /// The owner gate's answer for the game about to start: the selected account's session
    /// file (none = offline) and why it plays offline, if it does.
    Session(Result<Option<crate::account::GameSession>, crate::account::NoOwner>),
    /// The step Play is at before the game starts, for the button's label.
    PlayStep(PlayStep),
    ManagedClient {
        instance: Box<crate::instances::Instance>,
        result: Box<Result<crate::managed_play::Started, crate::managed_play::StartError>>,
    },
    /// The exit worker has tried to save the completed profile's play time.
    PlayRecorded,
    /// The worker checked the game's journal for the selected profile before Play changed it.
    PlayAdmission(
        PathBuf,
        Result<(), crate::browser::installed::GameInstallProblem>,
    ),
    /// The existing build picker changed the pin, or refused an unsettled game journal.
    PinnedBuild {
        home: PathBuf,
        result: Result<(), String>,
        close_versions: bool,
    },
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
    /// The native packages read for the consent request identified by this folder Arc.
    NativeConsentRead(Arc<PathBuf>, Vec<crate::native_status::BuildInput>),
    /// Where the native-mods build is.
    NativeProgress(crate::native_mods::Step),
    NativeRegistryConsent(crate::native_mods::Consent, std::sync::mpsc::Sender<bool>),
    /// The native-mods build ended: the version built, or why not.
    Native(Result<String, crate::native_mods::Failed>),
    /// An imported game build is installed (`build_import::import`): its version, or why not.
    Imported(Result<String, String>),
}
