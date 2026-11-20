//! Windows background processes must keep logs, input and exit status without a console.
#![cfg(windows)]

mod support;
use std::{fs, path::Path, sync::mpsc, time::Duration};
use sunrise_launcher::{
    backend::{self, Job, Msg},
    process,
};
use support::console_probe as probe;

fn logged_child(fail: bool) -> (Vec<String>, Result<(), String>) {
    let (sender, receiver) = mpsc::channel();
    let mut command = std::process::Command::new(probe());
    if fail {
        command.arg("--fail");
    }
    let mut child = backend::spawn(Job::Prepare, command, sender, None).unwrap();
    child.send("input reaches the child");
    let mut lines = Vec::new();
    loop {
        match receiver.recv_timeout(Duration::from_secs(10)).unwrap() {
            Msg::Line(Job::Prepare, line) => lines.push(line),
            Msg::Exit(Job::Prepare, result) => return (lines, result),
            _ => panic!("unexpected job message"),
        }
    }
}

#[test]
fn hidden_children_keep_stdout_stderr_stdin_and_success() {
    let (lines, result) = logged_child(false);
    assert!(result.is_ok(), "{result:?}");
    for expected in [
        "console_window_visible=false",
        "stdout is captured",
        "stderr is captured",
        "stdin=input reaches the child",
    ] {
        assert!(
            lines.iter().any(|line| line == expected),
            "missing {expected}: {lines:?}"
        );
    }
}

#[test]
fn hidden_children_report_failure_and_keep_their_logs() {
    let (lines, result) = logged_child(true);
    assert!(result.unwrap_err().contains('7'));
    assert!(lines.iter().any(|line| line == "stderr is captured"));
}

#[test]
fn cli_keeps_redirected_stdout_and_stderr() {
    let executable = env!("CARGO_BIN_EXE_sunrise-launcher");
    let version = process::command(executable)
        .arg("--version")
        .output()
        .unwrap();
    assert!(version.status.success());
    assert!(String::from_utf8_lossy(&version.stdout)
        .starts_with(sunrise_launcher::identity::VERSION_PREFIX));
    let invalid = process::command(executable)
        .arg("--unknown-option")
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("--unknown-option"));
}

#[test]
fn mcp_keeps_its_stdio_transport() {
    let executable = Path::new(env!("CARGO_BIN_EXE_sunrise-launcher"));
    use std::io::Write;
    use std::process::Stdio;
    let mut child = process::command(executable)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let lines = [
        serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "sunrise-launcher-test", "version": "1" } } }),
        serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} }),
    ];
    let mut stdin = child.stdin.take().unwrap();
    for line in &lines {
        writeln!(stdin, "{line}").unwrap();
    }
    // Dropping stdin ends the session: the server answers, then exits.
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let tools = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .find(|r| r["id"] == 2)
        .and_then(|r| r["result"]["tools"].as_array().map(Vec::len));
    assert!(tools.is_some_and(|n| n > 0));
}

#[test]
fn desktop_executable_uses_the_windows_gui_subsystem() {
    const PE_HEADER_POINTER: usize = 0x3c;
    const PE_OPTIONAL_HEADER_OFFSET: usize = 24;
    const SUBSYSTEM_OFFSET: usize = 68;
    const WINDOWS_GUI_SUBSYSTEM: u16 = 2;

    let executable = fs::read(env!("CARGO_BIN_EXE_sunrise-launcher")).unwrap();
    let header = u32::from_le_bytes(
        executable[PE_HEADER_POINTER..PE_HEADER_POINTER + 4]
            .try_into()
            .unwrap(),
    ) as usize;
    assert_eq!(&executable[header..header + 4], b"PE\0\0");
    let subsystem = header + PE_OPTIONAL_HEADER_OFFSET + SUBSYSTEM_OFFSET;
    assert_eq!(
        u16::from_le_bytes(executable[subsystem..subsystem + 2].try_into().unwrap()),
        WINDOWS_GUI_SUBSYSTEM
    );
}
