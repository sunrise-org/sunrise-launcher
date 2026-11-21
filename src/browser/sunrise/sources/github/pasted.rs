//! Proofs for pasted source links and source advances. Neither proof fetches an archive.
use super::{valid_repo, Failure, Github, GithubRelease, RefKind, State, STATE};
use crate::browser::sunrise::sources::{safe_ref, PastedPin};
use crate::github_mods::{Link, Resolved, SourceTrack, Target};
use serde::Deserialize;
use sunrise_launcher_smod::manifest::revision;

pub(crate) fn pasted_source(link: &Link, found: &Resolved) -> Result<PastedPin, Failure> {
    let mut state = STATE.lock().unwrap_or_else(|error| error.into_inner());
    prove_pasted(&Github::current(), &mut state, link, found)
}

pub(super) fn prove_pasted(
    github: &Github,
    state: &mut State,
    link: &Link,
    found: &Resolved,
) -> Result<PastedPin, Failure> {
    let requested_repo = format!("{}/{}", link.owner, link.repo);
    if !valid_repo(&requested_repo) || !found.name().eq_ignore_ascii_case(&requested_repo) {
        return Err(Failure::RepositoryUnavailable);
    }
    if !found.assets.is_empty() || !revision(&found.commit) {
        return Err(Failure::RefUnavailable);
    }
    let identity = github.repository(state, &requested_repo)?;
    let repo = &identity.full_name;
    let release: Option<GithubRelease> = match &link.target {
        Target::Latest => github.api(state, repo, &["releases", "latest"], None)?,
        Target::Tag(tag) if safe_ref(tag) => {
            github.api(state, repo, &["releases", "tags", tag], None)?
        }
        Target::Tag(_) => return Err(Failure::RefUnavailable),
        _ => None,
    };
    let (sha, track, published_at, tag) = if let Some(release) = release {
        if matches!(link.target, Target::Latest) && release.prerelease {
            return Err(Failure::RefUnavailable);
        }
        let release = release.published()?;
        if found.tag.as_deref() != Some(release.tag.as_str())
            || matches!(&link.target, Target::Tag(requested) if requested != &release.tag)
        {
            return Err(Failure::RefUnavailable);
        }
        let sha = github.ref_commit(state, repo, RefKind::Tag, &release.tag)?;
        (
            sha,
            SourceTrack::Release,
            Some(release.published_at),
            Some(release.tag),
        )
    } else {
        if found.tag.is_some() {
            return Err(Failure::RefUnavailable);
        }
        let (sha, track) = match &link.target {
            Target::Latest | Target::Head => {
                let name = identity
                    .default_branch
                    .filter(|name| safe_branch(name))
                    .ok_or(Failure::RefUnavailable)?;
                let sha = github.ref_commit(state, repo, RefKind::Head, &name)?;
                (sha, SourceTrack::Branch(name))
            }
            Target::Tag(name) => (
                github.ref_commit(state, repo, RefKind::Tag, name)?,
                SourceTrack::Pinned,
            ),
            Target::Ref(reference) => {
                let (kind, name) = if let Some(name) = reference.strip_prefix("refs/heads/") {
                    (Some(RefKind::Head), name)
                } else if let Some(name) = reference.strip_prefix("refs/tags/") {
                    (Some(RefKind::Tag), name)
                } else {
                    (None, reference.as_str())
                };
                if !safe_ref(name) {
                    return Err(Failure::RefUnavailable);
                }
                match kind {
                    Some(RefKind::Head) if safe_branch(name) => (
                        github.ref_commit(state, repo, RefKind::Head, name)?,
                        SourceTrack::Branch(name.into()),
                    ),
                    Some(RefKind::Head) => return Err(Failure::RefUnavailable),
                    Some(RefKind::Tag) => (
                        github.ref_commit(state, repo, RefKind::Tag, name)?,
                        SourceTrack::Pinned,
                    ),
                    None => {
                        let head = github.ref_commit(state, repo, RefKind::Head, name)?;
                        let tag = github.ref_commit(state, repo, RefKind::Tag, name)?;
                        match (head, tag) {
                            (Some(sha), None) if safe_branch(name) => {
                                (Some(sha), SourceTrack::Branch(name.into()))
                            }
                            (None, Some(sha)) => (Some(sha), SourceTrack::Pinned),
                            (None, None) => (
                                prove_commit(github, state, repo, name, &found.commit)?,
                                SourceTrack::Pinned,
                            ),
                            _ => return Err(Failure::RefUnavailable),
                        }
                    }
                }
            }
            Target::Commit(revision) => (
                prove_commit(github, state, repo, revision, &found.commit)?,
                SourceTrack::Pinned,
            ),
        };
        (sha, track, None, None)
    };
    if sha.as_deref() != Some(found.commit.as_str()) {
        return Err(Failure::RefUnavailable);
    }
    Ok(PastedPin {
        repo: identity.full_name,
        repo_id: identity.id,
        track,
        published_at,
        tag,
    })
}

fn safe_branch(name: &str) -> bool {
    safe_ref(name) && !name.starts_with('-')
}

#[derive(Deserialize)]
struct Commit {
    sha: String,
}

fn prove_commit(
    github: &Github,
    state: &mut State,
    repo: &str,
    input: &str,
    expected: &str,
) -> Result<Option<String>, Failure> {
    // Prove the exact selected commit is public, not just a match to the user's abbreviated SHA.
    if !(7..=40).contains(&input.len()) || !expected.starts_with(&input.to_ascii_lowercase()) {
        return Err(Failure::RefUnavailable);
    }
    let receipt: Option<Commit> = github.api(state, repo, &["git", "commits", expected], None)?;
    Ok(receipt.map(|commit| commit.sha))
}

/// Proof that the resolved branch pin is a strict fast-forward from the installed pin.
pub(crate) fn branch_ahead(repo: &str, old: &str, new: &str) -> Result<bool, Failure> {
    let mut state = STATE.lock().unwrap_or_else(|error| error.into_inner());
    compare(&Github::current(), &mut state, repo, old, new)
}

pub(super) fn compare(
    github: &Github,
    state: &mut State,
    repo: &str,
    old: &str,
    new: &str,
) -> Result<bool, Failure> {
    if !valid_repo(repo) {
        return Err(Failure::RepositoryUnavailable);
    }
    if !revision(old) || !revision(new) {
        return Err(Failure::RefUnavailable);
    }
    if old == new {
        return Ok(false);
    }
    #[derive(Deserialize)]
    struct Comparison {
        status: String,
        merge_base_commit: Commit,
    }
    let receipt: Comparison = github
        .api(state, repo, &["compare", &format!("{old}...{new}")], None)?
        .ok_or(Failure::RefUnavailable)?;
    Ok(receipt.status == "ahead" && receipt.merge_base_commit.sha == old)
}
