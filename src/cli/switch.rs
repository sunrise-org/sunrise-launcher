//! Human approval for the same sealed plan as the desktop chooser.
use super::*;
use std::{
    io::{self, BufRead, Write},
    sync::atomic::{AtomicBool, Ordering},
};
use sunrise_launcher::browser::sunrise::{
    switch_build_v2, SwitchDecision, SwitchHooks, Switched, Target, V2SwitchHooks,
};

static BEFORE_CUTOFF: AtomicBool = AtomicBool::new(true);

#[cfg(unix)]
extern "C" fn interrupt(_: libc::c_int) {
    if BEFORE_CUTOFF.swap(false, Ordering::SeqCst) {
        // SAFETY: _exit is async-signal-safe. Only private stage bytes exist before cutoff.
        unsafe { libc::_exit(130) }
    }
}

#[cfg(windows)]
unsafe extern "system" fn interrupt(kind: u32) -> i32 {
    use windows_sys::Win32::System::{
        Console::CTRL_C_EVENT,
        Threading::{GetCurrentProcess, TerminateProcess},
    };
    if kind != CTRL_C_EVENT {
        return 0;
    }
    if BEFORE_CUTOFF.swap(false, Ordering::SeqCst) {
        // SAFETY: this pseudo-handle is our process. Only private stage bytes exist.
        unsafe {
            TerminateProcess(GetCurrentProcess(), 130);
        }
    }
    1
}

fn catch_interrupt() -> io::Result<()> {
    #[cfg(unix)]
    {
        // SAFETY: the C handler performs only lock-free atomics and _exit.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = interrupt as *const () as usize;
            libc::sigemptyset(&mut action.sa_mask);
            if libc::sigaction(libc::SIGINT, &action, std::ptr::null_mut()) == -1 {
                return Err(io::Error::last_os_error());
            }
        }
    }
    #[cfg(windows)]
    {
        // SAFETY: the handler has the required ABI and outlives this CLI process.
        if unsafe { windows_sys::Win32::System::Console::SetConsoleCtrlHandler(Some(interrupt), 1) }
            == 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

pub(super) fn run(instance: &instances::Instance, tag: &str, as_json: bool) -> Result<(), Error> {
    let target = Target::of(instance)?;
    let identity = target.identity();
    catch_interrupt().map_err(|error| error.to_string())?;
    let tty = io::stdin().is_terminal() && io::stderr().is_terminal();
    let cancellation = native_mods::Cancellation::default();
    let hooks = SwitchHooks {
        cancellation: &cancellation,
        progress: &|_| {},
        approve: &|question| {
            let ask = || -> io::Result<SwitchDecision> {
                let mut out = io::stderr().lock();
                writeln!(
                    out,
                    "{}",
                    trf("Switch to {build}?", &[("build", &question.build)])
                )?;
                if question.has_omissions() {
                    writeln!(out, "{}", question.omission_summary())?;
                }
                for line in question
                    .hold_lines()
                    .chain(question.removed.iter().map(|(name, reason)| {
                        trf(
                            "{mod} was taken down: {reason}",
                            &[("mod", name), ("reason", reason)],
                        )
                    }))
                    .chain(question.world_lines())
                {
                    writeln!(out, "{}", line.replace(char::is_control, " "))?;
                }
                approve(tty, question.has_omissions(), io::stdin().lock(), out)
            };
            ask().map_err(|error| error.to_string())
        },
        at_cutoff: &|| {
            let mut out = io::stderr().lock();
            writeln!(out, "{}", tr("Applying the switch."))
                .and_then(|()| out.flush())
                .map_err(|error| error.to_string())?;
            if BEFORE_CUTOFF.swap(false, Ordering::SeqCst) {
                Ok(())
            } else {
                Err(cancelled())
            }
        },
    };
    let result = switch_build_v2(
        &target,
        identity,
        tag,
        V2SwitchHooks {
            switch: hooks,
            confirm_publisher_change: None,
            pin_dir: None,
            recover_held: false,
        },
    )?;
    let mut output = io::stdout().lock();
    match result {
        Switched::Stayed => return Err(cancelled().into()),
        Switched::AlreadySelected if as_json => writeln!(output, "{}", serde_json::json!({"profile": instance.id, "build": tag, "changed": false})),
        Switched::AlreadySelected => writeln!(output, "{}", trf("This profile already plays {build}.", &[("build", tag)])),
        Switched::Changed(active) if as_json => writeln!(output, "{}", serde_json::json!({
            "profile": instance.id, "build": active.base(), "image": active.executable(), "changed": true,
        })),
        Switched::Changed(active) => {
            let mut report = || -> io::Result<()> {
                writeln!(output, "{}", trf("Profile game build: {build}", &[("build", active.base())]))?;
                if let Some(changes) = active.pending_changes() {
                    if let Some(base) = changes.base().filter(|base| *base != active.base()) {
                        writeln!(output, "{}", trf("Report for game build: {build}", &[("build", base)]))?;
                    }
                    for line in changes.held_lines() {
                        writeln!(output, "{line}")?;
                    }
                    for (name, reason) in changes.removed {
                        writeln!(output, "{}", trf("{mod} was removed: {reason}", &[("mod", name), ("reason", reason)]).replace(char::is_control, " "))?;
                    }
                }
                Ok(())
            };
            report()
        }
    }.and_then(|()| output.flush()).map_err(|error| error.to_string().into())
}

fn cancelled() -> String {
    tr("Switch cancelled. Nothing was switched.").into()
}

fn approve(
    tty: bool,
    omissions: bool,
    mut input: impl BufRead,
    mut output: impl Write,
) -> io::Result<SwitchDecision> {
    if !tty {
        writeln!(output, "{}", tr("To decide, run Switch in a terminal."))?;
        return Ok(SwitchDecision::Cancel);
    }
    let prompt = if omissions {
        tr("[y] Switch and turn them off, [r] Switch and remove them, [N] Cancel")
    } else {
        tr("Switch anyway? [y/N]")
    };
    write!(output, "{prompt} ")?;
    output.flush()?;
    let mut reply = String::new();
    input.read_line(&mut reply)?;
    Ok(if !reply.ends_with('\n') {
        SwitchDecision::Cancel
    } else if reply.trim().eq_ignore_ascii_case("y") {
        SwitchDecision::TurnOff
    } else if omissions && reply.trim().eq_ignore_ascii_case("r") {
        SwitchDecision::Remove
    } else {
        SwitchDecision::Cancel
    })
}

/// `switch`: the profile it names (default: the selected one) to `tag`.
pub(super) fn profile(tag: String, profile: Option<String>, as_json: bool) -> Result<(), Error> {
    let instance = instances::read_profile(profile.as_deref())?;
    switch::run(&instance, &tag, as_json)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_an_explicit_terminal_answer_accepts_the_sealed_decision() {
        let decide = |tty, omissions, reply: &str| {
            approve(tty, omissions, reply.as_bytes(), Vec::new()).unwrap()
        };
        assert_eq!(decide(true, false, "y\n"), SwitchDecision::TurnOff);
        assert_eq!(decide(true, true, "r\n"), SwitchDecision::Remove);
        assert_eq!(decide(true, false, "r\n"), SwitchDecision::Cancel);
        for reply in ["", "\n", "no\n", "yes?\n", "y", "Y\r"] {
            assert_eq!(decide(true, true, reply), SwitchDecision::Cancel);
        }
        assert_eq!(decide(false, true, "r\n"), SwitchDecision::Cancel);
    }
}
