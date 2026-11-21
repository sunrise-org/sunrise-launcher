# Agents working on the Sunrise launcher

For AI coding agents (and anyone new). [CONTRIBUTING.md](CONTRIBUTING.md) and the
[README](README.md) are the source of truth; this file is the short version, plus what agents
tend to get wrong here.

## Workflow

- Start from a fresh base: `git fetch origin` and branch from `origin/main`. Don't build on a
  stale `main`.
- Sign off every commit (`git commit -s`). The DCO check fails a PR without it;
  `git rebase --signoff origin/main` fixes a branch.
- Never push to `main`, force-push it, or move or delete release tags. `main` only changes
  through PRs, and each one needs a code owner's approval.
- One concern per PR. A move or rename goes in its own PR, with no behaviour change, so a
  reviewer can check that nothing else changed.
- Open a PR as a draft until it's ready for review. Don't merge PRs; the person you work for
  decides that.
- Don't rebase, push to or close someone else's branch or PR stack.

## Before you open a PR

Run what CI runs (`.github/workflows/ci.yml`; the Rust version is in `tools/ci/rust-version`):

    cargo fmt --all --check
    cargo clippy --all-targets --locked -- -D warnings
    cargo test --locked --workspace --lib --bins
    cargo test --locked --test developer_install --test storage_consent
    cargo test --locked --workspace --doc

- The full `cargo test` takes a while, and the `storage` tests alone take about 90 s. A slow test
  isn't a hung one; wait for it to finish before deciding.
- Warnings that are already on `main` aren't yours to fix in an unrelated PR. Say which ones
  you saw.
- Ignored tests are listed in `tools/ci/manual-tests.txt`, with what each one needs.

## The PR description

Follow [the PR template](.github/pull_request_template.md):

- **First line:** say who you are, e.g.
  `> 🤖 Opened by <name>'s AI agent (<tool, model>), not by <name> personally.`
- **What changed** and **Why**. For a bug, include the cause and how you found it, not only the
  fix.
- **Self-check:** the exact head SHA, the commands you ran and on which OS, what you tried in
  the running launcher, and **Not checked**. Be honest in Not checked: an untested platform or
  path is fine to admit, and claiming it was tested when it wasn't is not.
- **Numbers over adjectives:** "frames every 48 ms before, 16.7 ms after" rather than
  "smoother".
- **UI or design changes:** a before (taken on `main`) and an after of each screen, at 1280×800
  and 800×600, in the same window size and state.
- **Stacked or split work:** "Based on #N" or "Extracted from #N (`sha`)", and what has to merge
  first.

## Running the launcher

- Point test runs at a scratch home: `SUNRISE_LAUNCHER_HOME=<temp dir>`, plus
  `SUNRISE_LAUNCHER_NO_SELF_UPDATE=1`. Never write into the player's real data folder.
  `tools/lab.py` (`game_env`) sets up a full scratch environment for scripts.
- Judge performance only in a release build (`cargo build --release`). A debug build stutters on
  its own.
- To measure frames: `cargo build --release --features frame-stats`, then run with
  `SUNRISE_LAUNCHER_FRAME_OVERLAY=1`. `sunrise-launcher benchmark` and the session log in
  `benchmarks/` are covered in the README's "Performance log".
- GPUI caps a window to one frame per 33 ms while it's unfocused and under the system's
  power-saver profile (`WindowOptions` in `src/startup.rs`). A 60 Hz display rounds that up to
  ~21 fps. If an animation looks choppy, rule these out before tuning curves.
- Under reduced motion nothing animates: use `motion::eased` and `motion::still()` (`src/motion.rs`)
  rather than raw GPUI animations.

## Project specifics

- **User-facing text** goes through `tr("…")` / `trf("…", &[…])`, never a bare string. Add new
  keys to every catalog in `lang/` with `tools/launcher_add_lang_keys.py "Your string"`.
- **The window, CLI and MCP server share one core**, `src/backend.rs`. A behaviour change there
  affects all three, so check the CLI (`--json`) too.
- **Copies of game files:** when a change brings in a game change, update
  `tools/ci/sunrise-commit` in the same PR (CONTRIBUTING.md, "Copies of game files").
- **Licences:** contributions are under PolyForm Shield. The `scenery` crate is MIT. Don't paste
  in code you can't license that way.
- **Comments and commit messages:** plain prose explaining why, in the voice of the code around
  them.

## Where things are

- `src/frame.rs`: builds every frame (`Launcher::frame`): rail, header, page and the layers
  above them.
- `src/backend.rs`: installs, launching and the data folder, shared by window, CLI and MCP.
- `src/cli.rs`, `src/cli_tools.rs`, `src/mcp.rs`: the command line and the MCP tools generated
  from it.
- `src/mods/`, `src/browser/`: the Mods page and the mod browser.
- `src/servers_page/`: Servers and hosting.
- `src/motion.rs`, `crates/theme`: motion, curves and design tokens.
- `scenery/`: the backdrop (sky, rain, blur); MIT.
- `crates/`: shared crates split out of the game repository (see CONTRIBUTING.md, "History").
- `tools/`: release, CI and test helpers. `tools/ci/` is what the workflows call.
