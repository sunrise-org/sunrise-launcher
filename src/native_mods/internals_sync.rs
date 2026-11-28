//! A mod author's discovery and registry lock through the mod's SDK tool (`native-discover`, `native-lock`), with the launcher's managed Rust and native cache.
use super::{root, rust_toolchain, tool, Cancellation};
use crate::rust_toolchain::Toolchain;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Execution context for a one-line SDK command (`native-discover`, `native-lock`).
/// When `workspace` is present, Cargo's home and temp directories are isolated to it.
pub(super) struct Context<'a> {
    pub(super) program: &'a Path,
    pub(super) rust: &'a Toolchain,
    pub(super) work: &'a Path,
    pub(super) workspace: Option<&'a super::workspace::Workspace>,
}

/// Runs the tool for the mod in `folder` on `command` with `plan` plus the work folder, the toolchain and the mod.
fn ask(folder: &Path, command: &str, plan: Value) -> Result<Value, String> {
    let rust = rust_toolchain::install(|_| {})?;
    let program = super::tool_for(folder)?;
    let work = root();
    let isolated = if super::workspace::cargo_home_unproved() {
        std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
        Some(super::workspace::Workspace::new(&work).map_err(|e| e.to_string())?)
    } else {
        None
    };
    let ctx = Context {
        program: &program,
        rust: &rust,
        work: &work,
        workspace: isolated.as_ref(),
    };
    run(&ctx, folder, command, plan, &Cancellation::default())
}

/// Runs `ctx.program` on `command` for the mod in `folder`, with `plan` plus `work` (the tool's own folder), the toolchain and the mod.
pub(super) fn run(
    ctx: &Context<'_>,
    folder: &Path,
    command: &str,
    mut plan: Value,
    cancellation: &Cancellation,
) -> Result<Value, String> {
    let mut cmd = tool::command(ctx.rust, ctx.program);
    let mut tools = tool::toolchain(ctx.rust);
    if let Some(workspace) = ctx.workspace {
        workspace.isolate(&mut cmd).map_err(|e| e.to_string())?;
        tools["cargo_home"] = json!(workspace.cargo_home());
    }
    plan["work"] = json!(ctx.work);
    plan["toolchain"] = tools;
    plan["mods"] = json!(folder);
    tool::ask(cmd, command, &plan, cancellation)
}

/// The internals the mod in `folder` names, as the pinned game's catalog lists them (the tool's JSON).
pub(crate) fn check(folder: &Path) -> Result<Value, String> {
    Ok(ask(folder, "native-discover", json!({}))?["internals"].take())
}

/// The pinned game's build data with only this mod's modules applied, copied into `destination` while no native build can replace it.
pub fn catalog(folder: &Path, destination: &Path) -> Result<PathBuf, String> {
    let _lease = crate::native_cache::Cache::acquire(&crate::backend::build_dir())
        .map_err(super::cache_error)?;
    super::clear_log()?;
    let reply = ask(folder, "native-discover", json!({"catalog": destination}))?;
    Ok(reply["catalog"]
        .as_str()
        .map_or_else(|| destination.to_path_buf(), PathBuf::from))
}

/// Pins the mod's registry dependencies in its `Cargo.lock`; never compiles source or changes the manifest.
pub(crate) fn lock(folder: &Path, update: bool) -> Result<(), String> {
    ask(folder, "native-lock", json!({"update": update})).map(drop)
}
