use super::*;

/// What the Skins tab's Save left the account wearing.
pub struct SavedLook {
    /// The skin's URL and model ("classic" or "slim"), when the services said.
    pub skin: Option<(String, String)>,
    pub capes: Vec<crate::account::capes::Cape>,
}

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
    /// The capes account `.0` owns, from the request numbered `.1` (a listing or a change).
    Capes(
        String,
        u64,
        Result<Vec<crate::account::capes::Cape>, String>,
    ),
    /// A skin's or cape's picture from Mojang's texture server, by its URL (none: it couldn't be
    /// had).
    Picture(String, Option<Vec<u8>>),
    /// The OptiFine cape of a name: its picture, or none (no cape, or the server couldn't say).
    OptifineCape(String, Option<Vec<u8>>),
    /// A Save on the Skins tab for account `.0`, numbered `.1`, ended: what the account wears
    /// after it, or why it stopped.
    LookSaved(String, u64, Box<Result<SavedLook, String>>),
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
