//! The wrapper receives a one-use invocation, never the profile or cache capabilities.
use super::{
    child::Child,
    leases::{ChildLeases, ProfileHandoff},
    relay_lifetime::{self, RelayedGame},
    relay_wrapper::Wrapper,
};
use crate::backend::lock::recover;
use crate::managed_play::StartError;
use serde::{Deserialize, Serialize};
use std::{
    ffi::{OsStr, OsString},
    io,
    path::PathBuf,
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use sunrise_launcher_smod::{
    holder,
    local_control::{self, Channel, Endpoint, Marker, MAX_RELAY_CLAIMS},
};

const ARG: &str = "--internal-game-relay";
const TIMEOUT: Duration = Duration::from_secs(10);
const PROTECTED: &[&str] = &[
    "SUNRISE_HOME",
    "SUNRISE_SESSION_FILE",
    "SUNRISE_LAUNCHER_EXE",
    "SUNRISE_ACCOUNT_SESSIONS",
    "SUNRISE_NO_DATA_FILE",
    "SUNRISE_WINDOWS_RUNTIME_SLOT",
    "SUNRISE_RUNTIME_LEASE_FDS",
    "SUNRISE_RUNTIME_LEASE_HANDLES",
    "SUNRISE_RUNTIME_DATA_ROOT",
    crate::developer_offline::ENV,
    crate::run_leases::SOURCE,
    crate::run_leases::GAME,
    holder::ENV,
    holder::ENTRY,
];

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Invocation {
    marker: Marker,
    #[cfg(windows)]
    issuer: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Offer {
    count: usize,
    program: OsString,
    args: Vec<OsString>,
    directory: OsString,
    // Corresponds to PROTECTED. The wrapper may set normal environment, never these fields.
    environment: Vec<Option<OsString>>,
}

#[derive(PartialEq, Eq, Serialize, Deserialize)]
enum Signal {
    Acquired,
    Commit,
    Started(u32),
    Refused(String),
}

enum Confirmation {
    Spawned(RelayedGame),
    Refused(io::Error),
    Unknown(io::Error),
}

fn commit(channel: Channel) -> Confirmation {
    // The acquiring relay can still fail before its signed-off spawn, or start the
    // game and die before its reply reaches us. Neither EOF nor a timeout tells us which.
    let reply = channel
        .relay_committed(Instant::now() + TIMEOUT)
        .send(&Signal::Commit)
        .and_then(Channel::receive::<Signal>);
    match reply {
        Ok((channel, Signal::Started(pid))) if pid != 0 => {
            Confirmation::Spawned(RelayedGame::new(channel, pid))
        }
        Ok((_, Signal::Refused(error))) => Confirmation::Refused(io::Error::other(error)),
        Ok(_) => Confirmation::Unknown(io::ErrorKind::InvalidData.into()),
        Err(error) => Confirmation::Unknown(error),
    }
}

#[derive(PartialEq, Eq)]
enum Grant {
    Pending,
    HandedOut,
    Revoked,
}

struct Token {
    state: Mutex<Grant>,
    revoked: Arc<AtomicBool>,
}
impl Token {
    fn new() -> Self {
        Self {
            state: Mutex::new(Grant::Pending),
            revoked: Arc::new(AtomicBool::new(false)),
        }
    }
    fn revoke(&self) {
        let mut state = recover(&self.state);
        if *state == Grant::Pending {
            *state = Grant::Revoked;
            self.revoked.store(true, Ordering::Release);
        }
    }
    fn handout<T>(
        &self,
        deadline: Instant,
        transfer: impl FnOnce() -> io::Result<T>,
    ) -> io::Result<T> {
        let mut state = recover(&self.state);
        if *state != Grant::Pending {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        let sent = if Instant::now() >= deadline {
            Err(io::ErrorKind::TimedOut.into())
        } else {
            transfer()
        };
        if sent.is_ok() {
            *state = Grant::HandedOut;
        } else {
            *state = Grant::Revoked;
            self.revoked.store(true, Ordering::Release);
        }
        sent
    }
}

pub(crate) fn start(
    game: &Command,
    prefix: &[OsString],
    leases: &ChildLeases,
    profile: ProfileHandoff<'_>,
    piped: bool,
) -> Result<Child, StartError> {
    let invocation = Invocation {
        marker: Marker::runtime().map_err(|e| e.to_string())?,
        #[cfg(windows)]
        issuer: std::process::id(),
    };
    let mut endpoint = Endpoint::bind(&invocation.marker).map_err(|e| e.to_string())?;
    let encoded = serde_json::to_string(&invocation).map_err(|e| e.to_string())?;
    let args: Vec<_> = game.get_args().map(OsStr::to_owned).collect();
    let launcher = std::env::current_exe().map_err(|e| e.to_string())?;
    let words = [launcher.into_os_string(), ARG.into(), encoded.into()]
        .into_iter()
        .chain(args.iter().cloned());
    let (program, prefix) = prefix.split_first().expect("this route has a wrapper");
    let mut command = Command::new(program);
    command.args(prefix).args(words);
    crate::launch::same_place(game, &mut command);
    let label = program.to_string_lossy();
    let failure = |e: io::Error| {
        crate::i18n::trf(
            "Could not start Sunrise through {wrapper}: {reason}",
            &[("wrapper", &label), ("reason", &e.to_string())],
        )
    };
    let files: Vec<_> = leases.files().into_iter().chain([profile.file]).collect();
    if files.len() > MAX_RELAY_CLAIMS {
        return Err(failure(io::ErrorKind::InvalidInput.into()).into());
    }
    let offer = Offer {
        count: files.len(),
        program: game.get_program().to_owned(),
        args,
        directory: game
            .get_current_dir()
            .expect("the admitted working directory")
            .as_os_str()
            .to_owned(),
        environment: PROTECTED
            .iter()
            .map(|key| {
                game.get_envs()
                    .find(|(name, _)| {
                        if cfg!(windows) {
                            name.to_str()
                                .is_some_and(|name| name.eq_ignore_ascii_case(key))
                        } else {
                            name == key
                        }
                    })
                    .map_or_else(
                        || std::env::var_os(key),
                        |(_, value)| value.map(OsStr::to_owned),
                    )
            })
            .collect(),
    };
    let wrapper = Wrapper::start(command, piped).map_err(failure)?;
    #[cfg(windows)]
    let job = wrapper
        .child
        .wrapper_job()
        .expect("Windows wrapper started in its Job");
    let wrapper = Mutex::new(wrapper);
    let token = Token::new();
    let done = AtomicBool::new(false);
    let deadline = Instant::now() + TIMEOUT;
    let outcome = std::thread::scope(|scope| -> io::Result<Confirmation> {
        std::thread::Builder::new()
            .name("wrapper-leader".into())
            .spawn_scoped(scope, || {
                while !done.load(Ordering::Acquire) && Instant::now() < deadline {
                    let exited = recover(&wrapper).leader_exited();
                    if !matches!(exited, Ok(false)) {
                        token.revoke();
                        return;
                    }
                    std::thread::park_timeout(Duration::from_millis(2));
                }
                if !done.load(Ordering::Acquire) {
                    token.revoke();
                }
            })?;
        let handshake = (|| {
            let stream =
                endpoint.accept_while(deadline, || !token.revoked.load(Ordering::Acquire))?;
            let channel = Channel::authenticate_relay(
                stream,
                &invocation.marker,
                true,
                deadline,
                Some(token.revoked.clone()),
            )?
            .send(&offer)?;
            if recover(&wrapper).leader_exited()? {
                token.revoke();
            }
            let channel = token.handout(deadline, || {
                #[cfg(unix)]
                {
                    let channel = channel.send_relay_claims(&files)?;
                    profile.handed_off.store(true, Ordering::Relaxed);
                    Ok(channel)
                }
                #[cfg(windows)]
                windows::send_claims(channel, &job, &files)
            })?;
            let (channel, ack) = channel.receive::<Signal>()?;
            if ack != Signal::Acquired {
                return Err(io::ErrorKind::InvalidData.into());
            }
            Ok(commit(channel))
        })();
        done.store(true, Ordering::Release);
        handshake
    });
    // Close the endpoint before returning failure (particularly if no relay ever connected).
    token.revoke();
    drop(endpoint);
    let mut wrapper = wrapper.into_inner().unwrap();
    match outcome {
        Ok(Confirmation::Spawned(game)) => {
            let child = Child::relayed(game, &mut wrapper.child);
            wrapper.retire();
            Ok(child)
        }
        Err(error) | Ok(Confirmation::Refused(error)) => {
            wrapper.retire();
            Err(failure(error).into())
        }
        Ok(Confirmation::Unknown(error)) => {
            wrapper.retire();
            Err(StartError::UnknownSpawn(crate::i18n::trf(
                "Sunrise may have started. Close the game if it opened before trying again.\nWrapper: {wrapper}\n{reason}",
                &[("wrapper", &label), ("reason", &error.to_string())],
            )))
        }
    }
}

/// Consume private modes before the launcher initializes, including before GUI/keychain work.
/// Ordinary invocations don't enter this function's process-control paths.
#[doc(hidden)]
pub fn private(mode: &OsStr, args: impl Iterator<Item = OsString>) -> Option<i32> {
    #[cfg(any(target_os = "linux", windows))]
    if mode == super::relay_wrapper::ARG {
        return Some(super::relay_wrapper::run(args).unwrap_or_else(|error| {
            crate::elog!(
                "{}",
                crate::i18n::trf(
                    "The game wrapper stopped: {reason}",
                    &[("reason", &error.to_string())]
                )
            );
            78
        }));
    }
    if mode != ARG {
        return None;
    }
    Some(run(args).unwrap_or_else(|_| {
        crate::elog!("{}", crate::i18n::tr("Sunrise didn't start: the game launch is no longer active or could not be verified."));
        78
    }))
}

fn run(mut args: impl Iterator<Item = OsString>) -> io::Result<i32> {
    let marker = args.next().ok_or(io::ErrorKind::InvalidInput)?;
    let encoded = marker
        .to_str()
        .filter(|s| s.len() <= 4096)
        .ok_or(io::ErrorKind::InvalidInput)?;
    let invocation: Invocation = serde_json::from_str(encoded)?;
    let marker = &invocation.marker;
    let deadline = Instant::now() + TIMEOUT;
    #[cfg(unix)]
    let stream = local_control::connect(marker, deadline)?;
    #[cfg(windows)]
    let stream = local_control::connect_relay(marker, invocation.issuer, deadline)?;
    let channel = Channel::authenticate_relay(stream, marker, false, deadline, None)?;
    let (channel, offer) = channel.receive::<Offer>()?;
    let directory = PathBuf::from(offer.directory);
    let program = PathBuf::from(offer.program);
    if !program.is_absolute()
        || !directory.is_absolute()
        || offer.environment.len() != PROTECTED.len()
        || offer.count == 0
        || offer.count > MAX_RELAY_CLAIMS
        || !args.eq(offer.args.iter().cloned())
    {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let mut game = Command::new(program);
    game.current_dir(directory).args(offer.args);
    for (name, value) in PROTECTED.iter().zip(offer.environment) {
        match value {
            Some(value) => game.env(name, value),
            None => game.env_remove(name),
        };
    }
    #[cfg(unix)]
    let (channel, claims) = channel.receive_relay_claims(offer.count)?;
    #[cfg(windows)]
    let (channel, claims) = windows::receive_claims(channel, offer.count)?;
    let (channel, permit) = channel.send(&Signal::Acquired)?.receive::<Signal>()?;
    if permit != Signal::Commit {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    let channel = channel.relay_committed(Instant::now() + TIMEOUT);
    #[cfg(unix)]
    let child = super::leases::spawn_unix(game, &claims.iter().collect::<Vec<_>>(), None, None)
        .map(Child::from);
    #[cfg(windows)]
    let child = windows::spawn(game, &claims);
    drop(claims);
    let mut child = match child {
        Ok(child) => child,
        Err(error) => {
            let _ = channel.send(&Signal::Refused(error.to_string()));
            return Err(error);
        }
    };
    // The child already owns its claims. Even if the launcher never receives this
    // acknowledgment (or dies), the relay must not terminate the game.
    let status = match channel.send(&Signal::Started(child.id())) {
        Ok(channel) => relay_lifetime::observe(child, channel)?,
        Err(_) => child.wait()?,
    };
    #[cfg(unix)]
    return super::supervisor::exit_code(status);
    #[cfg(windows)]
    Ok(status.code().unwrap_or(1))
}

#[cfg(windows)]
#[path = "relay_windows.rs"]
mod windows;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;

    #[test]
    fn revoke_and_handout_have_one_winner() {
        for _ in 0..32 {
            let token = Token::new();
            let barrier = Barrier::new(2);
            let handout = std::thread::scope(|scope| {
                let send = scope.spawn(|| {
                    barrier.wait();
                    token.handout(Instant::now() + TIMEOUT, || Ok(42))
                });
                barrier.wait();
                token.revoke();
                send.join().unwrap()
            });
            assert_eq!(handout.is_ok(), !token.revoked.load(Ordering::Acquire));
            token.revoke();
            assert_eq!(handout.is_ok(), !token.revoked.load(Ordering::Acquire));
            assert!(token
                .handout::<()>(Instant::now() + TIMEOUT, || panic!("reuse"))
                .is_err());
        }
    }
}
