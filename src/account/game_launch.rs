//! The owner gate: the session the game starts with, and the session file it reads once.
use super::*;

/// The owner gate's pass for one launch.
#[derive(Debug, Default)]
pub struct Launch {
    /// The selected account and its renewed session; none when its sign-in couldn't be renewed
    /// now (`offline` says why).
    pub session: Option<(Account, Secrets)>,
    /// Why the selected account plays without its session this time.
    pub offline: Option<String>,
    /// An account the services said owns the game just now, the selected one first: who the
    /// launcher may download the game for.
    pub owner: Option<(Account, Secrets)>,
    /// A renewed sign-in the keychain refused to keep: it plays now, and may have to sign in
    /// again next time.
    pub unsaved: Option<String>,
}

/// The last [`Launch::unsaved`], for the window or the terminal to show once.
static UNSAVED: Mutex<Option<String>> = Mutex::new(None);

/// A Minecraft token for the signed-in `account` with at least [`RENEW_BEFORE`] left: the one
/// the keychain keeps, or one renewed through the refresh token (and kept) when it runs out.
/// For the profile's own changes (its cape), never for the game.
pub fn token_for(
    ep: &Endpoints,
    client_id: Option<&str>,
    store: &dyn SecretStore,
    account: &Account,
) -> Result<String, String> {
    let _operation = session::lock(store)?;
    let secrets = store
        .get(&account.uuid)?
        .and_then(|s| serde_json::from_str::<Secrets>(&s).ok())
        .ok_or_else(|| {
            crate::i18n::trf(
                "{name} is no longer signed in on this computer. Sign in again.",
                &[("name", &account.name)],
            )
        })?;
    if secrets.sign_in_required {
        return Err(crate::i18n::tr(SIGN_IN_AGAIN).into());
    }
    if secrets.minecraft_expires_at > now() + RENEW_BEFORE {
        return Ok(secrets.minecraft_token);
    }
    let keep = |secrets: &Secrets| {
        serde_json::to_string(secrets)
            .map_err(|e| e.to_string())
            .and_then(|text| store.set(&account.uuid, &text).map_err(String::from))
    };
    match check(ep, client_id, account, secrets) {
        Check::Owns(_, secrets) => {
            keep(&secrets)?;
            Ok(secrets.minecraft_token)
        }
        Check::DoesNot(secrets) => {
            let _ = keep(&secrets);
            Err(crate::i18n::tr(DOES_NOT_OWN).into())
        }
        Check::NoAnswer(e, secrets) => {
            let _ = keep(&secrets);
            Err(e.into())
        }
    }
}

/// What the last launch couldn't save in the keychain, once.
pub fn take_unsaved() -> Option<String> {
    UNSAVED.lock().ok()?.take()
}

/// The owner gate: the game starts only while an account in the launcher owns Minecraft: Java
/// Edition. The accounts are asked again, the selected one first, until one owns it; each keeps
/// the answer in the keychain. An account that gets no answer (no network) keeps its last one, so
/// being offline never blocks a player who passed before, while "doesn't own" blocks at once.
/// The caller saves `accounts` (renewed names and skins).
pub fn launch(
    ep: &Endpoints,
    client_id: Option<&str>,
    store: &dyn SecretStore,
    accounts: &mut Accounts,
) -> Result<Launch, NoOwner> {
    let selected = accounts.selected.clone();
    let mut order = accounts.accounts.clone();
    order.sort_by_key(|a| Some(&a.uuid) != selected.as_ref());
    let mut pass = Launch::default();
    let mut owned = false;
    let mut unreadable = None;
    for account in order {
        if owned {
            break;
        }
        let plays = Some(&account.uuid) == selected.as_ref();
        let operation = session::lock(store);
        let read = match store.get(&account.uuid) {
            Ok(text) => text.and_then(|s| serde_json::from_str::<Secrets>(&s).ok()),
            Err(why) => {
                // Not "signed out": the keyring didn't say (#12179). With no owner, the refusal
                // says this instead.
                let said = crate::i18n::trf(
                    "Can't read {name}'s sign-in: {why}",
                    &[("name", &account.name), ("why", &why.0)],
                );
                eprintln!("Sunrise: {said}");
                if plays {
                    pass.offline = Some(said.clone());
                }
                unreadable = Some(said);
                continue;
            }
        };
        let Some(secrets) = read else {
            if plays {
                pass.offline = Some(crate::i18n::trf(
                    "{name} is no longer signed in on this computer. Sign in again.",
                    &[("name", &account.name)],
                ));
            }
            continue;
        };
        let before = serde_json::to_string(&secrets).ok();
        let checked = match &operation {
            Ok(_) => check(ep, client_id, &account, secrets),
            Err(e) => Check::NoAnswer(e.clone().into(), secrets),
        };
        let (said, secrets) = match checked {
            Check::Owns(renewed, secrets) => {
                accounts.update(renewed.clone());
                if plays {
                    pass.session = Some((renewed.clone(), secrets.clone()));
                }
                pass.owner = Some((renewed, secrets.clone()));
                (None, secrets)
            }
            Check::DoesNot(secrets) => (Some(crate::i18n::tr(DOES_NOT_OWN).to_string()), secrets),
            Check::NoAnswer(e, secrets) => {
                // No answer, but the token the keychain keeps is still good: the game plays
                // signed in with it rather than without a session (#542).
                // Only when the services couldn't answer: a refusal (401/403, a bad profile) or a
                // locked keychain never starts the game on the kept token.
                if plays && matches!(e, SignInError::Unavailable(_)) && still_valid(&secrets) {
                    pass.session = Some((account.clone(), secrets.clone()));
                    (None, secrets)
                } else {
                    (Some(e.to_string()), secrets)
                }
            }
        };
        owned = secrets.owns;
        if plays {
            pass.offline = said;
        }
        let after = serde_json::to_string(&secrets).ok();
        if after != before {
            if let Some(text) = after {
                // The game still starts; the player hears why the next launch may ask again.
                if let Err(e) = store.set(&account.uuid, &text) {
                    let said = format!(
                        "{}'s renewed sign-in couldn't be saved in the keychain ({e}): you may \
                         have to sign in again next time.",
                        account.name
                    );
                    if let Ok(mut last) = UNSAVED.lock() {
                        *last = Some(said.clone());
                    }
                    pass.unsaved = Some(said);
                }
            }
        }
    }
    if owned {
        Ok(pass)
    } else {
        Err(NoOwner {
            keyring: unreadable,
        })
    }
}

/// The owner gate before the game starts: [`launch`], where the account that plays (the selected
/// one, else the first) must own the game itself, by the services' answer or, when they can't be
/// asked, its last one. Never another account's ownership for a name of its own.
pub fn launch_for_game(
    ep: &Endpoints,
    client_id: Option<&str>,
    store: &dyn SecretStore,
    accounts: &mut Accounts,
) -> Result<Launch, NoOwner> {
    if accounts.selected().is_none() {
        accounts.selected = accounts.accounts.first().map(|a| a.uuid.clone());
    }
    let pass = launch(ep, client_id, store, accounts)?;
    let owns = accounts
        .selected()
        .and_then(|a| secrets_in(store, &a.uuid))
        .is_some_and(|s| s.owns);
    if owns && pass.session.is_none() {
        // A signed launcher never starts the game for a signed-in player without a session
        // (#542): online-mode servers would refuse them. Play stops with the reason; pressing
        // Play again retries. Only developer builds play offline, and they never get here.
        let why = pass
            .offline
            .unwrap_or_else(|| crate::i18n::tr(NEEDS_OWNER).to_string());
        return Err(NoOwner {
            keyring: Some(crate::i18n::trf(
                "Couldn't sign in to play online: {why} Press Play to try again.",
                &[("why", &why)],
            )),
        });
    }
    if owns {
        Ok(pass)
    } else {
        // Why that account can't play (it doesn't own the game, signed out, an unread keyring).
        Err(NoOwner {
            keyring: pass.offline,
        })
    }
}

/// How long a kept Minecraft token must still last for the game to start with it when the
/// services can't be asked: a running game is handed a renewed session before then.
pub(super) const STILL_VALID_FOR: u64 = 300;

/// Whether the kept sign-in can start the game by itself: not marked for a new sign-in, owns
/// the game by the last answer, and its token outlasts [`STILL_VALID_FOR`] by its own `exp`.
pub(super) fn still_valid(secrets: &Secrets) -> bool {
    !secrets.sign_in_required
        && secrets.owns
        && !secrets.minecraft_token.is_empty()
        && secrets.minecraft_expires_at > now() + STILL_VALID_FOR
        && token_exp(&secrets.minecraft_token).is_none_or(|exp| exp > now() + STILL_VALID_FOR)
}

/// The `exp` claim of a JWT access token, read locally (#618); none when the token isn't a
/// JWT. The signature isn't checked here: the token came from the services over TLS and stays
/// in the keychain, and the services check it again on every use.
pub(super) fn token_exp(token: &str) -> Option<u64> {
    use base64::Engine as _;
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()?
        .get("exp")?
        .as_u64()
}

/// Write the session for the game, readable by this user only, in a fresh file it deletes
/// when it reads it.
pub fn write_session_file(
    dir: &Path,
    account: &Account,
    secrets: &Secrets,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    // The folder too is this user's only (the files already are).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    // Leftovers of games that never started (refused by the game after two minutes anyway).
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let old = e
                .metadata()
                .and_then(|m| m.modified())
                .is_ok_and(|m| m.elapsed().is_ok_and(|age| age > Duration::from_secs(120)));
            if old {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let mut name = [0u8; 12];
    getrandom::fill(&mut name).map_err(|e| e.to_string())?;
    let path = dir.join(format!(
        "session-{}.json",
        name.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(&path).map_err(|e| e.to_string())?;
    let body = json!({
        "name": account.name,
        "uuid": account.uuid,
        "access_token": secrets.minecraft_token,
        "user_type": "msa",
        "xuid": secrets.xuid,
    });
    std::io::Write::write_all(&mut file, body.to_string().as_bytes()).map_err(|e| e.to_string())?;
    Ok(path)
}

/// How long a game takes a session file: it refuses one older than this (the game's
/// `MAX_SESSION_FILE_AGE`), so the launcher deletes it then if it is still there.
pub(super) const SESSION_FILE_LIFE: Duration = Duration::from_secs(120);

/// The session files this launcher wrote for games it starts, until they are gone: a game
/// deletes each as it reads it; the rest go when the game would refuse them anyway
/// ([`SESSION_FILE_LIFE`], a game that never started or crashed first) or when the launch failed
/// ([`discard_unread_sessions`]). Nothing with a token stays on disk.
pub(super) struct UnreadSession {
    pub(super) path: PathBuf,
    pub(super) expire_only: bool,
}

static UNREAD: Mutex<Vec<UnreadSession>> = Mutex::new(Vec::new());

/// [`write_session_file`] for a game this launcher is about to start (see [`UNREAD`]).
fn write_launch_session_file(
    dir: &Path,
    account: &Account,
    secrets: &Secrets,
) -> Result<PathBuf, String> {
    let path = write_session_file(dir, account, secrets)?;
    let Ok(mut unread) = UNREAD.lock() else {
        return Ok(path);
    };
    if unread.is_empty() {
        // One reaper while files wait; it ends with the list (under its lock, so a file added
        // as it ends starts the next).
        let _ = std::thread::Builder::new()
            .name("Session files".into())
            .spawn(|| loop {
                std::thread::sleep(Duration::from_secs(5));
                let Ok(mut unread) = UNREAD.lock() else {
                    return;
                };
                reap(&mut unread, SESSION_FILE_LIFE);
                if unread.is_empty() {
                    return;
                }
            });
    }
    unread.push(UnreadSession {
        path: path.clone(),
        expire_only: false,
    });
    Ok(path)
}

/// Deletes the files of `unread` older than `life` (the game would refuse them) and forgets
/// those gone (read).
pub(super) fn reap(unread: &mut Vec<UnreadSession>, life: Duration) {
    unread.retain(|file| {
        let waiting = std::fs::metadata(&file.path)
            .and_then(|m| m.modified())
            .is_ok_and(|m| m.elapsed().is_ok_and(|age| age <= life));
        if !waiting {
            let _ = std::fs::remove_file(&file.path);
        }
        waiting
    });
}

/// A managed handoff can be unresolved. Nothing but its own definite failure/exit
/// or the game's usual expiry may delete that launch's file.
pub(crate) fn retain_launch_session(path: Option<&Path>) {
    if let (Some(path), Ok(mut unread)) = (path, UNREAD.lock()) {
        if let Some(file) = unread.iter_mut().find(|file| file.path == path) {
            file.expire_only = true;
        }
    }
}

/// Deletes the session files written for a launch whose game didn't start, or has ended: any it
/// didn't read is left over (a game deletes those it reads).
pub fn discard_unread_sessions() {
    let files: Vec<_> = UNREAD
        .lock()
        .map(|mut unread| unread.extract_if(.., |file| !file.expire_only).collect())
        .unwrap_or_default();
    for file in files {
        let _ = std::fs::remove_file(file.path);
    }
}

/// Discard only the session prepared for the launch that was refused.
pub fn discard_launch_session(file: Option<PathBuf>) {
    if let Some(file) = file {
        if let Ok(mut unread) = UNREAD.lock() {
            unread.retain(|session| session.path != file);
        }
        let _ = std::fs::remove_file(file);
    }
}

/// What the game about to start gets from the accounts.
#[derive(Debug, Default)]
pub struct GameSession {
    /// The playing account's session (`--session-file`); none: offline.
    pub file: Option<PathBuf>,
    /// Why the account plays offline this time, if it does.
    pub offline: Option<String>,
}

impl GameSession {
    /// Gives the game its playing account, or the offline reason shown to the player.
    pub fn apply(self, cmd: &mut std::process::Command) -> Option<String> {
        if let Some(file) = self.file {
            cmd.arg("--session-file").arg(file);
        }
        if let Some(reason) = &self.offline {
            cmd.env("SUNRISE_OFFLINE_REASON", reason);
        }
        self.offline
    }
}

/// The owner gate for the game about to start, then the selected account's session in a file
/// for it.
pub fn session_for_game(game_home: &Path) -> Result<GameSession, NoOwner> {
    let mut accounts = Accounts::load();
    let pass = launch_for_game(
        &Endpoints::live(),
        client_id().as_deref(),
        store(),
        &mut accounts,
    )?;
    if let Err(e) = accounts.save_renewed() {
        eprintln!("Sunrise: the accounts are not saved: {e}");
    }
    session_file(game_home, pass)
}

/// [`session_for_game`] for the account that was selected when Play was accepted, `accepted`:
/// installing, preparing and building can take minutes, and whoever is selected when they end
/// must not be the one who plays. Which account is selected now is not touched. An account
/// signed out meanwhile doesn't play, and no session file is written for it.
pub fn session_for_game_accepted(game_home: &Path, accepted: &str) -> Result<GameSession, NoOwner> {
    let mut accounts = Accounts::load();
    let pass = launch_accepted(
        &Endpoints::live(),
        client_id().as_deref(),
        store(),
        &mut accounts,
        accepted,
    )?;
    if let Err(e) = accounts.save_renewed() {
        eprintln!("Sunrise: the accounts are not saved: {e}");
    }
    session_file(game_home, pass)
}

pub(super) fn launch_accepted(
    ep: &Endpoints,
    client_id: Option<&str>,
    store: &dyn SecretStore,
    accounts: &mut Accounts,
    accepted: &str,
) -> Result<Launch, NoOwner> {
    check_launch_account(accounts, accepted)?;
    let chosen = accounts.selected.replace(accepted.to_owned());
    let pass = launch_for_game(ep, client_id, store, accounts);
    accounts.selected = chosen;
    pass
}

/// The accepted UUID must still be signed in, independent of who is selected now.
pub fn check_launch_account(accounts: &Accounts, accepted: &str) -> Result<(), NoOwner> {
    if !accounts.accounts.iter().any(|a| a.uuid == accepted) {
        return Err(NoOwner {
            keyring: Some(
                crate::i18n::tr(
                    "The account Play was pressed for was signed out before the game started. Press Play again.",
                )
                .into(),
            ),
        });
    }
    Ok(())
}

/// [`session_for_game`] for one launch as `who` (`play --account`, a signed-in account): who
/// plays changes for this launch only, so several games can play as several accounts at once
/// (#4952). What the services renewed (tokens, names, skins) is still kept; which account
/// `account select` picked is not touched.
pub fn session_for_game_as(game_home: &Path, who: &str) -> Result<GameSession, String> {
    let mut accounts = Accounts::load();
    let pass = launch_as(
        &Endpoints::live(),
        client_id().as_deref(),
        store(),
        &mut accounts,
        who,
    );
    if let Err(e) = accounts.save_renewed() {
        eprintln!("Sunrise: the accounts are not saved: {e}");
    }
    session_file(game_home, pass?).map_err(|e| e.to_string())
}

/// The owner gate as `who`, on `accounts`, which keeps its choice of player.
pub(super) fn launch_as(
    ep: &Endpoints,
    client_id: Option<&str>,
    store: &dyn SecretStore,
    accounts: &mut Accounts,
    who: &str,
) -> Result<Launch, String> {
    let chosen = accounts.selected.clone();
    let pass = accounts
        .play_as(who)
        .and_then(|()| launch_for_game(ep, client_id, store, accounts).map_err(|e| e.to_string()));
    accounts.selected = chosen;
    pass
}

/// The game's session files for a launch that passed the gate.
fn session_file(game_home: &Path, pass: Launch) -> Result<GameSession, NoOwner> {
    let mut out = GameSession {
        offline: pass.offline,
        ..GameSession::default()
    };
    if let Some((account, secrets)) = pass.session {
        // A session that can't be written is a stopped Play, not a signed-out game (#542).
        let file = write_launch_session_file(&game_home.join("sessions"), &account, &secrets)
            .map_err(|e| NoOwner { keyring: Some(e) })?;
        out.file = Some(file);
    }
    Ok(out)
}
