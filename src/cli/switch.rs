//! Human approval for the same sealed plan used by the version picker. The signal handler
//! cannot interrupt local publication once the cutoff has been crossed.
use super::*;
use std::io::{self, BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use sunrise::txn::{Lease, Switch};

static BEFORE_CUTOFF: AtomicBool = AtomicBool::new(true);

#[cfg(unix)]
extern "C" fn interrupt(_: libc::c_int) {
    if BEFORE_CUTOFF.swap(false, Ordering::SeqCst) {
        // SAFETY: _exit is async-signal-safe. No live change has started; admission cleans
        // the journal once the last inherited owner exits.
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
        // SAFETY: the pseudo-handle refers to this process; no result can be published yet.
        unsafe {
            TerminateProcess(GetCurrentProcess(), 130);
        }
    }
    1
}

fn catch_interrupt() -> io::Result<()> {
    #[cfg(unix)]
    {
        // SAFETY: sigaction is zero-initialized, its mask is initialized, and interrupt has
        // the C ABI and performs only lock-free atomics and _exit.
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
        // SAFETY: the handler has the required ABI and remains valid for this CLI process.
        if unsafe { windows_sys::Win32::System::Console::SetConsoleCtrlHandler(Some(interrupt), 1) }
            == 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

pub(super) fn run(instance: &instances::Instance, tag: &str, as_json: bool) -> Result<(), String> {
    if instance.is_pack_profile() {
        return Err(tr("A modpack profile keeps the game build it was installed on.").into());
    }
    catch_interrupt().map_err(|e| e.to_string())?;
    let owner = Arc::new(Lease::admit(&instance.home)?);
    let progress = native_progress(io::stderr().is_terminal());
    let plan = Switch::stage(
        owner,
        tag,
        instance.native_profile,
        |step| {
            if let Some(line) = progress(step) {
                let _ = writeln!(io::stderr(), "{line}");
            }
        },
        &registry_consent,
    )?;
    let tty = io::stdin().is_terminal() && io::stdout().is_terminal();
    let mut output = io::stdout().lock();
    let Some(plan) = plan else {
        return result(&mut output, instance, tag, false, None, as_json).map_err(|e| e.to_string());
    };
    let lines = plan.question().lines();
    if as_json {
        writeln!(output, "{}", serde_json::json!({"profile":instance.id,"from":plan.prior(),"to":tag,"world_risks":lines,"waiting_for_update":plan.effects().waiting_for_update,"removed":plan.effects().removed}))
            .map_err(|e| e.to_string())?;
    } else {
        match plan.prior() {
            Some(prior) => writeln!(output, "{prior} → {tag}"),
            None => writeln!(
                output,
                "{}",
                trf("Profile game build: {build}", &[("build", tag)])
            ),
        }
        .map_err(|e| e.to_string())?;
        for line in lines {
            terminal_line(&mut output, &line).map_err(|e| e.to_string())?;
        }
        show_effects(&mut output, plan.effects()).map_err(|e| e.to_string())?;
    }
    output.flush().map_err(|e| e.to_string())?;
    if plan.question().needed()
        && !approve(
            tty,
            io::stdin().lock(),
            &mut output,
            tr("Switch anyway? [y/N]"),
        )
        .map_err(|e| e.to_string())?
    {
        return Err(tr("Switch cancelled. Nothing was switched.").into());
    }
    let receipt = plan.commit(|| {
        if as_json {
            writeln!(output, "{{\"applying\":true}}")
        } else {
            writeln!(output, "{}", tr("Applying the switch."))
        }
        .and_then(|()| output.flush())
        .map_err(|e| e.to_string())?;
        if BEFORE_CUTOFF.swap(false, Ordering::SeqCst) {
            Ok(())
        } else {
            Err(tr("Switch cancelled. Nothing was switched.").into())
        }
    })?;
    // Delivery can fail after the marker; it cannot undo the committed generation.
    result(&mut output, instance, tag, true, receipt.as_ref(), as_json).map_err(|e| e.to_string())
}

fn terminal_line(output: &mut impl Write, line: &str) -> io::Result<()> {
    let safe = line.replace(char::is_control, " ");
    writeln!(output, "{safe}")
}

fn show_effects(output: &mut impl Write, effects: &sunrise::txn::SwitchEffects) -> io::Result<()> {
    for (heading, mods) in [
        (tr("Turned off"), &effects.waiting_for_update),
        (tr("Removed"), &effects.removed),
    ] {
        for (name, reason) in mods {
            terminal_line(output, &format!("{heading} — {name}: {reason}"))?;
        }
    }
    Ok(())
}

pub(super) fn approve_world_change(
    question: &sunrise_launcher::worlds::SwitchQuestion,
    prompt: &str,
) -> Result<bool, String> {
    if !question.needed() {
        return Ok(true);
    }
    let tty = io::stdin().is_terminal() && io::stdout().is_terminal();
    let mut output = io::stdout().lock();
    for line in question.lines() {
        terminal_line(&mut output, &line).map_err(|e| e.to_string())?;
    }
    output.flush().map_err(|e| e.to_string())?;
    approve(tty, io::stdin().lock(), &mut output, prompt).map_err(|e| e.to_string())
}

fn approve(
    tty: bool,
    mut input: impl BufRead,
    output: &mut impl Write,
    prompt: &str,
) -> io::Result<bool> {
    if !tty {
        return Ok(false);
    }
    write!(output, "{prompt} ")?;
    output.flush()?;
    let mut line = String::new();
    let count = input.read_line(&mut line)?;
    Ok(count > 0 && line.ends_with('\n') && matches!(line.trim(), "y" | "Y"))
}

fn result(
    output: &mut impl Write,
    instance: &instances::Instance,
    tag: &str,
    changed: bool,
    receipt: Option<&sunrise::txn::Receipt>,
    as_json: bool,
) -> io::Result<()> {
    let empty = sunrise::txn::SwitchEffects::default();
    let effects = receipt.map_or(&empty, |r| r.effects());
    if as_json {
        writeln!(
            output,
            "{}",
            serde_json::json!({"profile":instance.id,"build":tag,"changed":changed,"waiting_for_update":effects.waiting_for_update,"removed":effects.removed})
        )?;
    } else {
        writeln!(
            output,
            "{}",
            trf("Profile game build: {build}", &[("build", tag)])
        )?;
        show_effects(output, effects)?;
    }
    output.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_switch_is_not_an_agent_tool_and_play_does_not_accept_a_build_override() {
        assert!(Cli::try_parse_from(["launcher", "switch", "nightly-abc", "--yes"]).is_err());
        assert!(Cli::try_parse_from(["launcher", "play", "--version", "nightly-abc"]).is_err());
        assert!(Cli::try_parse_from([
            "launcher",
            "modpack",
            "play",
            "pack",
            "--version",
            "nightly-abc"
        ])
        .is_err());
        assert!(!crate::cli_tools::all()
            .iter()
            .any(|tool| matches!(tool.name.as_str(), "switch" | "instance_version")));
    }

    #[cfg(unix)]
    #[test]
    fn interrupt_worker() {
        let Ok(when) = std::env::var("SUNRISE_TEST_SWITCH_INTERRUPT") else {
            return;
        };
        catch_interrupt().unwrap();
        if when == "after" {
            assert!(BEFORE_CUTOFF.swap(false, Ordering::SeqCst));
        }
        println!("handler ready");
        io::stdout().flush().unwrap();
        let mut command = String::new();
        io::stdin().read_line(&mut command).unwrap();
        assert_eq!(command, "done\n");
    }

    #[cfg(unix)]
    #[test]
    fn sigint_exits_only_before_cutoff() {
        use std::process::Stdio;
        for when in ["before", "after"] {
            let mut worker = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "cli::switch::tests::interrupt_worker",
                    "--nocapture",
                ])
                .env("SUNRISE_TEST_SWITCH_INTERRUPT", when)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let mut output = io::BufReader::new(worker.stdout.take().unwrap());
            assert!((&mut output)
                .lines()
                .any(|line| line.unwrap() == "handler ready"));
            // SAFETY: this PID belongs to the still-running child with its handler installed.
            assert_eq!(
                unsafe { libc::kill(worker.id() as libc::pid_t, libc::SIGINT) },
                0
            );
            let mut input = worker.stdin.take().unwrap();
            if when == "after" {
                writeln!(input, "done").unwrap();
            }
            assert_eq!(
                worker.wait().unwrap().code(),
                Some(if when == "before" { 130 } else { 0 })
            );
            drop(input);
        }
    }

    #[test]
    fn approval_requires_a_flushed_terminal_and_a_complete_explicit_answer() {
        let mut out = Vec::new();
        assert!(!approve(false, &b"y\n"[..], &mut out, "confirm").unwrap());
        assert!(out.is_empty());
        for answer in [&b""[..], &b"y"[..], &b"\n"[..], &b"n\n"[..]] {
            assert!(!approve(true, answer, &mut out, "confirm").unwrap());
        }
        assert!(approve(true, &b"y\n"[..], &mut out, "confirm").unwrap());
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Err(io::ErrorKind::BrokenPipe.into())
            }
        }
        assert_eq!(
            approve(true, &b"y\n"[..], &mut Broken, "confirm")
                .unwrap_err()
                .kind(),
            io::ErrorKind::BrokenPipe
        );
    }
}
