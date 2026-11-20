use crate::process::child::Child;
use std::process::Command;

pub(super) fn spawn(
    cmd: &mut Command,
    profile: Option<&crate::browser::sunrise::txn::Lease>,
) -> Result<Child, String> {
    crate::run_leases::spawn(cmd, true, profile).map_err(|error| {
        crate::i18n::trf(
            "Could not complete game launch: {error}",
            &[("error", &error.to_string())],
        )
    })
}
