# Contributing

## Licence and DCO

The launcher's code is under the [PolyForm Shield licence](LICENSE), and so is what you contribute: it comes in under the
same licence it goes out under, and you keep your copyright. There is no CLA. Instead, sign off every commit
(`git commit -s`) to certify the [Developer Certificate of Origin](https://developercertificate.org/): that you wrote it,
or have the right to submit it under the project's licence. The DCO check fails a pull request with a commit that isn't
signed off; `git rebase --signoff origin/main` fixes a branch.

Code that came from Horizon is also available under its original MIT licence ([LICENSES/MIT-Horizon.txt](LICENSES/MIT-Horizon.txt)),
and the `scenery` crate is MIT-licensed (its `Cargo.toml`).

## History

This repository was split out of [sunrise-org/sunrise](https://github.com/sunrise-org/sunrise), where the launcher lived
in `launcher/`, with its history (`git filter-repo`, from `sunrise-org/sunrise` at `d5813c112f0`). Commit messages are
unchanged, so a bare `#N` in a commit from before the split refers to sunrise-org/sunrise, or to Horizon for the oldest
ones. The crates under `crates/` were copied into the launcher from the game's own crates during the split: their
history before the copy stays in sunrise-org/sunrise.

## Copies of game files

CI compares the launcher's copies against the Sunrise commit in [`tools/ci/sunrise-commit`](tools/ci/sunrise-commit).
When bringing in a game change, update that file to the new full commit SHA and update the affected copies in the same PR.
If the game changes a function in the test's `MIRRORED` table, review and update the launcher implementation before
changing the corresponding hash.

The daily workflow checks the same copies against Sunrise's current `main`. If they differ, it opens or updates
one **Game copies drifted** issue with the affected copies and the Sunrise commits since the pin that touched them.
The next daily that finds the copies match closes the issue.

Run the check locally from the launcher repository root:

```sh
game=$(mktemp -d)
tools/ci/game_at.sh "$(cat tools/ci/sunrise-commit)" "$game"
SUNRISE_GAME_DIR="$game" python3 -m unittest tools/tests/test_game_copies.py
```

## Releasing

See [`docs/releasing.md`](docs/releasing.md) for the release workflow, testing checklist, and publishing steps.
