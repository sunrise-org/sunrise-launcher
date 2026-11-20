use std::process::{Child, Command};

/// The game, with its output in pipes the launcher reads, or in `log` ([`super::spawn_logged`]).
pub(super) fn spawn(cmd: &mut Command, log: Option<&std::fs::File>) -> Result<Child, String> {
    match log {
        Some(log) => crate::run_leases::spawn_logged(cmd, log),
        None => crate::run_leases::spawn(cmd, true),
    }
    .map_err(|error| {
        crate::i18n::trf(
            "Could not complete game launch: {error}",
            &[("error", &error.to_string())],
        )
    })
}
