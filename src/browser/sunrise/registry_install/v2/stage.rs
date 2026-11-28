//! Fetching a release and its attestation, verifying both, staging the change, and recording the publisher pins it accepted.

use super::*;

/// Maximum accepted `.sigstore.json` attestation bundle size (1 MiB).
const MAX_ATTESTATION_BYTES: u64 = 1 << 20;

/// Default production HTTPS byte fetcher for v2 release `.smod` archives and `.sigstore.json`
/// attestation bundles.
pub(crate) fn fetch_release_bytes(url: &str, max_bytes: u64) -> Result<Vec<u8>, String> {
    read_release_bytes(url, max_bytes, None)
}

/// [`fetch_release_bytes`], which gives up within about 50 ms of `stop` being asked: the
/// request runs on a worker the caller stops waiting for, and the body is read in chunks that
/// each check `stop`, so the worker ends soon after and nothing is kept.
pub(crate) fn fetch_release_bytes_cancellable(
    url: &str,
    max_bytes: u64,
    stop: Option<&native_mods::Cancellation>,
) -> Result<Vec<u8>, String> {
    let Some(stop) = stop else {
        return read_release_bytes(url, max_bytes, None);
    };
    stop.check()?;
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let worker_url = url.to_owned();
    let worker_stop = stop.clone();
    std::thread::spawn(move || {
        let _ = tx.send(read_release_bytes(
            &worker_url,
            max_bytes,
            Some(&worker_stop),
        ));
    });
    loop {
        stop.check()?;
        match rx.recv_timeout(std::time::Duration::from_millis(50)) {
            Ok(result) => return result,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                stop.check()?;
                return Err(crate::i18n::tr("Download worker stopped unexpectedly").into());
            }
        }
    }
}

fn read_release_bytes(
    url: &str,
    max_bytes: u64,
    stop: Option<&native_mods::Cancellation>,
) -> Result<Vec<u8>, String> {
    if !download::github_release_asset(url) {
        return Err(trf(
            "{url} is not a GitHub release asset URL allowed by the Sunrise v2 registry.",
            &[("url", url)],
        ));
    }
    let agent = download::agent(url)?;
    let response = agent.get(url).call().map_err(|e| {
        trf(
            "download failed for {url}: {error}",
            &[("url", url), ("error", &e.to_string())],
        )
    })?;
    let limited = response.into_reader().take(max_bytes.saturating_add(1));
    let mut bytes = Vec::new();
    let read = match stop {
        Some(stop) => stop.reader(limited).read_to_end(&mut bytes),
        None => {
            let mut limited = limited;
            limited.read_to_end(&mut bytes)
        }
    };
    read.map_err(|e| {
        if let Some(stop) = stop {
            if let Err(stopped) = stop.check() {
                return stopped;
            }
        }
        trf(
            "reading {url}: {error}",
            &[("url", url), ("error", &e.to_string())],
        )
    })?;
    if bytes.len() as u64 > max_bytes {
        return Err(trf(
            "{url} exceeded maximum allowed size of {max_bytes} bytes",
            &[("url", url), ("max_bytes", &max_bytes.to_string())],
        ));
    }
    Ok(bytes)
}

/// Download and verification callbacks for staging schema-2 registry releases.
#[derive(Clone, Copy)]
pub(crate) struct V2Transfer<'a> {
    pub(crate) fetch_bytes: BytesFetcher<'a>,
    pub(crate) verify_release: ReleaseVerifier<'a>,
}

/// Shared inputs for schema-2 registry install, update, and provider-switch edits.
pub(crate) struct V2EditRequest<'a> {
    pub(crate) target: &'a Target,
    pub(crate) identity: Identity,
    pub(crate) index: Arc<Index>,
    pub(crate) options: V2InstallOptions,
    pub(crate) transfer: V2Transfer<'a>,
}

pub(crate) fn stage_and_verify_v2_release(
    index: &Index,
    home: &Path,
    step: &PlannedV2Release,
    pin_dir: &Path,
    accepted_publisher_changes: &BTreeSet<String>,
    confirm_publisher_change: Option<ConfirmPublisherChange<'_>>,
    transfer: V2Transfer<'_>,
) -> Result<(installed::Entry, PendingPin), InstallError> {
    let V2Transfer {
        fetch_bytes,
        verify_release,
    } = transfer;
    let project = index.project(&step.project_id).ok_or_else(|| {
        InstallError::Refused(trf(
            "Project {project} is not in the Sunrise registry.",
            &[("project", &step.project_id)],
        ))
    })?;
    let release = project
        .releases
        .iter()
        .find(|r| r.version == step.version && r.file.sha256.eq_ignore_ascii_case(&step.sha256))
        .ok_or_else(|| {
            InstallError::Refused(trf(
                "Release {version} of {mod} is no longer in the Sunrise registry.",
                &[("version", &step.version), ("mod", &step.name)],
            ))
        })?;
    if let Some(revoked) = index.revoked(&project.id, Some(&release.file.sha256)) {
        return Err(InstallError::Refused(trf(
            "{mod} was taken down: {reason}",
            &[("mod", &project.name), ("reason", &revoked.reason)],
        )));
    }

    // 1. Download .smod and check exact byte length + SHA-256.
    let archive_bytes = fetch_bytes(&release.file.url, MAX_FILE).map_err(InstallError::Download)?;
    if archive_bytes.len() as u64 != release.file.size {
        return Err(InstallError::Download(trf(
            "{mod} download size did not match the registry ({got} bytes, expected {expected}).",
            &[
                ("mod", &project.name),
                ("got", &archive_bytes.len().to_string()),
                ("expected", &release.file.size.to_string()),
            ],
        )));
    }
    let actual_sha256 = index::sha256(&archive_bytes);
    if !actual_sha256.eq_ignore_ascii_case(&release.file.sha256) {
        return Err(InstallError::Download(trf(
            "{mod} failed its SHA-256 check.",
            &[("mod", &project.name)],
        )));
    }
    if let Some(revoked) = index.revoked_by_sha256(&actual_sha256) {
        return Err(InstallError::Refused(trf(
            "{mod} was taken down: {reason}",
            &[("mod", &project.name), ("reason", &revoked.reason)],
        )));
    }
    let digest_bytes = attestation::parse_sha256_hex(&actual_sha256).ok_or_else(|| {
        InstallError::Download(trf(
            "{mod} failed its SHA-256 check.",
            &[("mod", &project.name)],
        ))
    })?;

    // 2. Download .sigstore.json bundle and verify offline attestation + Sunrise release policy.
    let bundle_bytes = fetch_bytes(&release.file.attestation, MAX_ATTESTATION_BYTES)
        .map_err(InstallError::Download)?;
    let expected = ExpectedRelease {
        repository_id: project.release_repository(release),
        workflow: &release.workflow,
    };
    let verified = verify_release(&bundle_bytes, digest_bytes, &expected).map_err(|error| {
        InstallError::Refused(trf(
            "Could not verify the release signature for {mod}: {detail}",
            &[("mod", &project.name), ("detail", &error.to_string())],
        ))
    })?;

    // 3. Check per-computer publisher pin (without mutating disk until publication commits).
    let pin = verified.pin();
    let mut accepted_change = None;
    match attestation::check_pin_in(pin_dir, &project.id, &pin) {
        Ok(attestation::PinStatus::FirstInstall | attestation::PinStatus::Matched) => {}
        Err(attestation::Error::PublisherChanged(change)) => {
            let confirmed = accepted_publisher_changes.contains(&project.id)
                || confirm_publisher_change.is_some_and(|ask| {
                    ask(&PublisherChangeQuestion {
                        mod_name: project.name.clone(),
                        current_version: Document::read(home).ok().and_then(|doc| {
                            doc.row(&step.mod_id)
                                .ok()
                                .flatten()
                                .and_then(|row| row["version"].as_str().map(str::to_owned))
                        }),
                        incoming_version: release.version.clone(),
                        change: (*change).clone(),
                    })
                });
            if !confirmed {
                return Err(InstallError::PublisherChanged(change));
            }
            accepted_change = Some(*change);
        }
        Err(error) => return Err(InstallError::Unproved(error.to_string())),
    }

    // 4. Write archive into the candidate's target directory and inspect its manifest & layout.
    let folder = match project.kind {
        V2Kind::NativeMod => crate::native_mods::INSTALLED,
        V2Kind::Mod => mod_list::MODS_DIR,
        _ => {
            return Err(InstallError::Refused(trf(
                "{mod} is not a mod package.",
                &[("mod", &project.name)],
            )));
        }
    };
    let other_folder = if folder == crate::native_mods::INSTALLED {
        mod_list::MODS_DIR
    } else {
        crate::native_mods::INSTALLED
    };
    // If a mod changed kind (content <-> native), remove any old file in the sibling folder so
    // `scan_packages` never sees two archives for the same slot.
    let sibling = home
        .join(other_folder)
        .join(format!("{}.smod", project.mod_id));
    if sibling.exists() {
        fs::remove_file(&sibling).map_err(|e| InstallError::Unproved(at(&sibling, e)))?;
    }

    let relative = format!("{folder}/{}.smod", project.mod_id);
    let destination = home.join(&relative);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|e| InstallError::Unproved(at(parent, e)))?;
    }
    fs::write(&destination, &archive_bytes)
        .map_err(|e| InstallError::Unproved(at(&destination, e)))?;

    let smod_kind = match project.kind {
        V2Kind::NativeMod => index::Kind::NativeMod,
        V2Kind::Mod => index::Kind::Mod,
        _ => unreachable!(),
    };
    let facts = index::inspect(&smod_kind, &destination).map_err(|error| {
        let _ = fs::remove_file(&destination);
        InstallError::Download(trf(
            "{mod} is not a valid package: {detail}",
            &[("mod", &project.name), ("detail", &error)],
        ))
    })?;
    if facts.id.as_deref() != Some(project.mod_id.as_str())
        || facts.version.as_deref() != Some(release.version.as_str())
    {
        let _ = fs::remove_file(&destination);
        return Err(InstallError::Download(trf(
            "{mod} archive manifest did not match the registry entry.",
            &[("mod", &project.name)],
        )));
    }
    // Refuse packages that smuggle required dependencies not declared in the signed v2 release.
    for (dep_id, dep) in &facts.dependencies {
        if !dep.details().optional
            && !release
                .dependencies
                .iter()
                .any(|listed| listed.required && listed.mod_id == *dep_id)
        {
            let _ = fs::remove_file(&destination);
            return Err(InstallError::Download(trf(
                "{mod} archive declares an unlisted required dependency ({dep}).",
                &[("mod", &project.name), ("dep", dep_id)],
            )));
        }
    }

    // The signed index listed these bytes and their attestation verified: a build takes them
    // without the unreviewed-code warning.
    if matches!(project.kind, V2Kind::NativeMod) {
        backend::allow_native_files_in(pin_dir, [actual_sha256.clone()])
            .map_err(InstallError::Unproved)?;
    }
    let entry = installed::Entry {
        source: SourceId::Sunrise,
        project: project.id.clone(),
        mod_id: Some(project.mod_id.clone()),
        slug: project.slug.clone(),
        title: project.name.clone(),
        kind: Kind::Mod,
        version: release.version.clone(),
        number: release.version.clone(),
        file: relative,
        checksum: format!("SHA-256:{actual_sha256}"),
        page: format!("{SITE}/{}", project.slug),
        dependency: step.dependency,
        verdict: Some(tr("Verified").into()),
        install_hold: None,
        repository: Some(installed::VerifiedRepository {
            id: verified.attested.repository_id,
            name: verified.attested.repository_name.clone(),
            path: project.repository.path.clone(),
            workflow: release.workflow.clone(),
        }),
    };
    Ok((
        entry,
        PendingPin {
            project_id: project.id.clone(),
            pin,
            accepted_change,
        },
    ))
}

pub(crate) fn commit_pending_pins(pin_dir: &Path, pins: &[PendingPin]) -> Result<(), String> {
    for pending in pins {
        if let Some(change) = &pending.accepted_change {
            attestation::accept_pin_in(pin_dir, change).map_err(|e| e.to_string())?;
        } else {
            attestation::pin_or_check_in(pin_dir, &pending.project_id, &pending.pin)
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
