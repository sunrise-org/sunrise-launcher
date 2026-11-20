//! The GUI bridge: preparation off the UI thread, one consent for the complete missing set.
use gpui::{AnyWindowHandle, AppContext, Context, PromptLevel};
use std::{
    path::PathBuf,
    process::Command,
    sync::mpsc::{self, Sender},
};
use sunrise_launcher::{
    backend::Msg,
    browser::sunrise,
    i18n::{tr, trf},
    jars::{self, Permission},
    launch::{self, Launch},
};
use sunrise_launcher_smod::manifest::JarRef;

pub fn client(
    mut command: Command,
    tag: String,
    launch: Launch,
    run: Option<String>,
    (account, profile_owner): (Option<String>, std::sync::Arc<sunrise::txn::Lease>),
    session_file: Option<PathBuf>,
    tx: Sender<Msg>,
) {
    std::thread::spawn(move || {
        let mut registry_notices = Vec::new();
        let prepared =
            sunrise::before_play(&mut command, &tag, &launch, &profile_owner).and_then(|notices| {
                registry_notices = notices;
                with_consent(&tx, |permission| {
                    jars::prepare_launch(&mut command, &launch, permission)
                })
                .map_err(|error| error.to_string())?;
                launch::wrap(command, &launch)
            });
        if let Err(mpsc::SendError(Msg::JarClient { session_file, .. })) = tx.send(Msg::JarClient {
            run,
            account,
            profile_owner,
            session_file,
            registry_notices,
            prepared: Box::new(prepared),
        }) {
            sunrise_launcher::account::discard_launch_session(session_file);
        }
    });
}

/// Used only from background jobs: the UI owns the one complete-set prompt.
pub fn with_consent<T>(tx: &Sender<Msg>, prepare: impl FnOnce(Permission<'_>) -> T) -> T {
    let ask = |missing: &[&JarRef]| {
        let (reply, answer) = mpsc::channel();
        let jars = missing.iter().map(|jar| (*jar).clone()).collect();
        tx.send(Msg::JarConsent { jars, reply }).is_ok() && answer.recv().unwrap_or(false)
    };
    prepare(Permission::Ask(&ask))
}

pub fn consent(
    window: AnyWindowHandle,
    jars: Vec<JarRef>,
    reply: Sender<bool>,
    cx: &mut Context<crate::Launcher>,
) {
    let message = trf(
        "Download {count} original Java jars from Modrinth?",
        &[("count", &jars.len().to_string())],
    );
    let mut detail = tr("Only assets and data are used; Java code is never run. Each file is checked against the manifest's SHA-512.").to_owned();
    for jar in jars {
        detail.push_str("\n\n");
        detail.push_str(&trf(
            "{id} {version}\nSHA-512: {hash}",
            &[
                ("id", jar.id()),
                ("version", jar.version()),
                ("hash", jar.sha512()),
            ],
        ));
    }
    match cx.update_window(window, |_, window, cx| {
        window.prompt(
            PromptLevel::Warning,
            &message,
            Some(&detail),
            &[tr("Cancel"), tr("Download jars")],
            cx,
        )
    }) {
        Ok(answer) => cx
            .spawn(async move |_, _| {
                let _ = reply.send(matches!(answer.await, Ok(1)));
            })
            .detach(),
        Err(_) => {
            let _ = reply.send(false);
        }
    }
}
