# Releasing the Sunrise Launcher

Real launcher releases run only from the `release` branch (`refs/heads/release`) and require the repository variable `LAUNCHER_RELEASES=on`. Any run on another branch—or with `dry_run=true`—builds and verifies artifacts without creating a tag or draft.

> **Never publish a draft release by hand.** Hand-publishing skips `SHA256SUMS.sig` signing, which breaks the launcher's self-updater verification. Always publish through `launcher-bridge.yml` in `sunrise-org/sunrise`.

## Release steps

### 1. Promote `main` to `release`

Confirm `Cargo.toml` (and `Cargo.lock`) holds an untagged stable `X.Y.Z` version (`tools/release_notes.py decide` refuses versions that are already tagged):

```sh
git fetch origin main release
grep -m1 '^version = ' Cargo.toml
git tag --list 'v*' --sort=-v:refname | head -n 5
```

Open and merge a pull request from `main` into `release`:

```sh
gh pr create -R sunrise-org/sunrise-launcher \
  --base release --head main \
  --title "Release vX.Y.Z" \
  --body "Promote main to release for vX.Y.Z."
```

Wait for CI to pass and merge the PR.

### 2. Build the unsigned draft

Dispatch `release.yml` on `release` with `dry_run=false`:

```sh
gh workflow run release.yml -R sunrise-org/sunrise-launcher \
  --ref release \
  -f dry_run=false
```

Watch the run until it finishes:

```sh
gh run list -R sunrise-org/sunrise-launcher --workflow release.yml --limit 3
gh run watch -R sunrise-org/sunrise-launcher <run-id>
```

When it completes, it creates an unsigned draft GitHub release `vX.Y.Z` with all platform archives/installers, `SHA256SUMS`, build provenance attestations, and generated release notes (including PRs merged into `main` that shipped on `release`).

### 3. Test the draft end to end

Download the draft artifact for your platform (`gh release download vX.Y.Z -R sunrise-org/sunrise-launcher -p '<pattern>'`) and verify a fresh install before publishing:

- The installer or archive launches cleanly on a fresh home/profile.
- Sign-in (or offline startup) completes without unexpected prompts or errors.
- Downloading and starting the game works.
- Settings / About displays version `X.Y.Z`, and the updater check runs without errors.

### 4. Sign and publish via `launcher-bridge.yml`

Run the bridge workflow in `sunrise-org/sunrise`, pointing `source_ref` at `refs/heads/release`:

```sh
gh workflow run launcher-bridge.yml -R sunrise-org/sunrise \
  -f source_ref=refs/heads/release
```

Watch the bridge run (`gh run watch -R sunrise-org/sunrise <run-id>`), which signs `SHA256SUMS` into `SHA256SUMS.sig` and publishes the release. Once it finishes, confirm both the signature attachment and the `Latest` badge:

```sh
gh release view vX.Y.Z -R sunrise-org/sunrise-launcher \
  --json isDraft,isLatest,assets \
  --jq '{isDraft, isLatest, sig: (.assets | map(.name) | index("SHA256SUMS.sig") != null)}'
```

Expect `isDraft: false`, `isLatest: true`, and `sig: true`.

## Build runners (Blacksmith and GitHub fallback)

Platform build targets and their runner labels are defined in [`tools/launcher_build_matrix.py`](../tools/launcher_build_matrix.py). When the repository variable `BLACKSMITH_RELEASE=on` is set, the `build` job in [`.github/workflows/release.yml`](../.github/workflows/release.yml) runs platform builds on Blacksmith runners (using the same OS images and Linux glibc as GitHub's runners). If `BLACKSMITH_RELEASE` is unset or set to anything else, the workflow automatically falls back to standard GitHub-hosted runners. Attestation, draft creation, and publishing always run on GitHub runners.
