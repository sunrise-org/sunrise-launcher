use super::*;

pub enum Msg {
    Playing(Option<u64>),
    Storage(Result<Box<crate::storage::Report>, String>),
    Releases(Result<Vec<Release>, String>),
    Links(Links),
    Progress(String, f32),
    Installed(String, Result<(), String>),
    /// Manual build cleanup ended, with any rows it could not delete.
    Removed(Vec<(String, String)>),
    /// Automatic build cleanup ended; refresh the installed list.
    Pruned(Vec<(String, String)>),
    BuildInventory(u64, Vec<version_storage::Build>),
    Line(Job, String),
    Exit(Job, Result<(), String>),
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
    JarConsent {
        jars: Vec<sunrise_launcher_smod::manifest::JarRef>,
        reply: std::sync::mpsc::Sender<bool>,
    },
    JarClient {
        run: Option<String>,
        account: Option<String>,
        profile_owner: Arc<crate::browser::sunrise::txn::Lease>,
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
    PlayImage(Result<String, String>),
    /// An imported game build is installed (`build_import::import`): its version, or why not.
    Imported(Result<String, String>),
}
