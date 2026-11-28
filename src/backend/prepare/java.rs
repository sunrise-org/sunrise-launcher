//! The Java a preparation runs with: found on this computer or Minecraft's own, downloaded.
use crate::backend::{data_dir, fetch_bytes_cancellable, read_json};
use crate::mojang;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The Java home for this version: a runtime for precompiled exporters, a JDK for older
/// preparers or changed sources. Forward the selection to external preparers too.
/// Else try the official launcher's Java in `games`, then download the runtime named by
/// `manifest` into Sunrise's game folder (Mojang's runtimes are whole JDKs).
pub(in crate::backend) fn java(
    major: i64,
    compiler: bool,
    games: &[PathBuf],
    manifest: &Path,
    stop: Option<&crate::native_mods::Cancellation>,
    log: &dyn Fn(String),
) -> Result<PathBuf, String> {
    let find = if compiler {
        crate::jdk::find
    } else {
        crate::jdk::find_runtime
    };
    let cancel = stop.map_or(&mojang::CANCEL, |stop| stop.flag());
    java_with(find, major, games, manifest, &|component, missing| {
        if let Some(stop) = stop {
            stop.check()?;
        }
        // Mojang builds its Linux Java for glibc: without glibc's loader (NixOS without nix-ld,
        // musl distributions) it cannot start, so don't download it.
        if cfg!(all(target_os = "linux", target_arch = "x86_64"))
            && !Path::new(GLIBC_LOADER).exists()
        {
            return Err(crate::i18n::trf(
                "{missing}\n  • Minecraft's own Java from Mojang needs glibc ({loader} is missing; on NixOS, enable programs.nix-ld), so install compatible Java instead.",
                &[("missing", missing), ("loader", GLIBC_LOADER)],
            ));
        }
        let fetch = |url: &str| -> Result<Vec<u8>, String> { fetch_bytes_cancellable(url, stop) };
        let runtime = data_dir().join("minecraft").join("runtime");
        mojang::java_runtime(component, &runtime, 8, &fetch, cancel, log).map_err(|e| {
            crate::i18n::trf("{missing}\n  • Sunrise also tried to download Minecraft's own Java from Mojang, but: {error}", &[("missing", missing), ("error", &e.to_string())])
        })
    })
}

/// [`java`] with its Java lookup (`find`) and Mojang download (`download` of a runtime
/// component, given what was missing) passed in. A downloaded runtime's home is looked up
/// through [`crate::java::home_in`], so a macOS `jre.bundle` is entered.
pub(in crate::backend) fn java_with(
    find: impl Fn(i64, &[PathBuf]) -> Result<crate::jdk::Jdk, String>,
    major: i64,
    games: &[PathBuf],
    manifest: &Path,
    download: &dyn Fn(&str, &str) -> Result<PathBuf, String>,
) -> Result<PathBuf, String> {
    if let Ok(jdk) = find(major, &[]) {
        return Ok(jdk.home());
    }
    let mut runtimes: Vec<PathBuf> = Vec::new();
    for home in games
        .iter()
        .flat_map(|game| crate::java::minecraft_runtimes(&game.join("runtime")))
    {
        if !runtimes.contains(&home) {
            runtimes.push(home);
        }
    }
    let missing = match find(major, &runtimes) {
        Ok(jdk) => return Ok(jdk.home()),
        Err(e) => e,
    };
    let component = read_json(manifest)
        .and_then(|json| json["javaVersion"]["component"].as_str().map(String::from))
        .ok_or_else(|| missing.clone())?;
    let home = download(&component, &missing)?;
    let home = crate::java::home_in(&home).unwrap_or(home);
    find(major, &[home]).map(|jdk| jdk.home())
}

/// The dynamic loader glibc programs on x86-64 Linux start through.
pub(crate) const GLIBC_LOADER: &str = "/lib64/ld-linux-x86-64.so.2";

/// Runs `cmd` with Java at `home`: JAVA_HOME names it and its `bin` leads the PATH, as the
/// versions' own preparers look for Java.
pub(in crate::backend) fn with_java(cmd: &mut Command, home: &Path) {
    let mut path = vec![home.join("bin")];
    path.extend(
        std::env::var_os("PATH")
            .iter()
            .flat_map(std::env::split_paths),
    );
    cmd.env("JAVA_HOME", home);
    if let Ok(path) = std::env::join_paths(path) {
        cmd.env("PATH", path);
    }
}
