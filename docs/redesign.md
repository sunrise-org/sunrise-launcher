# Launcher redesign

The redesign of the launcher's window, built on the `redesign` branch and merged into `main` in
one go when every page is done. The design is the Figma file
[Sunrise Launcher — Redesign](https://www.figma.com/design/YQTiuNXtq74x1uvXjTRUxe) (dark theme,
round 1): its Screens and Popups pages are the target, its Components page the parts, its Notes
page what still needs backend work.

This file is the brief for everyone working on the branch, people and agents. Read
[AGENTS.md](../AGENTS.md) first; every rule there applies here.

## What does not change

- The core (`src/backend*`, the CLI, the MCP server) and everything a page *does*. The redesign
  changes how the window looks and how you move around it, never what a button does.
- Every string goes through `tr`/`trf`; labels reached later are `tr_key("…")` where defined.
  New keys go into `lang/en.json` with `tools/launcher_add_lang_keys.py`.
- Reduced motion: nothing animates under it (`motion::still()`, `motion::eased`).
- **The cat** (`src/cat/`): it peeks out, walks the empty part of the play bar, sleeps on Play
  and can be petted. It moves with Play into the profile page's banner (walking the space beside
  Play and Profile settings) and keeps its switch (Settings › Appearance › Show the cat) and its
  reduced-motion behaviour.
- Keyboard: every control stays reachable with Tab, Esc still closes the top layer, Ctrl K
  searches, Ctrl B toggles the sidebar.

## UX first

The design is a starting point, not a contract. When building a screen, if a flow feels clumsy,
a label is unclear, a state is missing or two things that belong together are far apart, improve
it and say what you changed and why in the commit message. Don't copy an awkward layout because
the mockup shows it, and don't invent features the backend can't do (see the Figma Notes page).

## Look

### Colour

The look is the built-in **Sunrise** theme (id `dawn`, `DAWN_DARK` in `src/themes.rs`), tuned to
the Figma palette. Screens never name a colour: they ask [`paint::palette`](../src/paint.rs)
for a role. Figma variable → palette role:

| Figma `color/…` | Role | Value (Sunrise) |
|---|---|---|
| bg/window, bg/rail | `chrome` | `#1E1B1A` |
| bg/panel | `background` | `#2B2324` |
| bg/surface | `card_color` | `#322829` |
| bg/surface-hover, bg/selected | `hover`, `selected` | `#3A2F30` |
| bg/sunken | `field` | `#241F1F` |
| border/subtle, border/default | `divider`, `line` | `#3A2F30`, `#4A3E3E` |
| text/primary, secondary, tertiary | `text`, `muted`, (muted, dimmer) | `#F3EAE6`, `#B9ABA6`, `#8E807B` |
| accent/default | `accent` | `#C2552D` |
| accent/soft, text/link | `accent_ink` | `#F2BC96` |
| status/* | `success`, `warning`, `danger`, `info` | `#7DC98C`, `#EFC56E`, `#EC8D8D`, `#8DB8EA` |

Sunrise in the light look (`DAWN_LIGHT` in `src/themes.rs`, the same roles on a cream room with
white cards, sky from pale peach to cream) is the light version of the same palette; the dark
look keeps `DAWN_DARK`. Other themes keep working: they feed the same roles, and stay dark in both
looks.

### Shape and type

- Corners ([`tokens.rs`](../src/tokens.rs)): bars 2, small 4, **controls and chips 6**, cards 8,
  panels and sheets 12. Chips are not pills. Switches and radios stay round.
- Type: Geist (`FONT`), Geist Mono for logs and codes. Sizes as in `tokens.rs` (caption 11, small
  12, label 13, body 14, lead 16, heading 18/20, title 24/28, the profile's name 40).

### Selection

The selected page or profile in the sidebar (and a Settings section) gets a raised fill, its
label in full text colour, and the **pixel sun** (the Sunrise mark's sun, 14×10, 2 px pixels) at
the end of the row. The sun is shown only when the label at full length plus the sun fits the
row at the current width; otherwise it is left out (never truncate the label for it). No accent
bar at the edge.

### Motion

Small and smooth, only where something changes: hover and press fades on controls, the chevron
turning when a menu or section opens, a copy button's tick, the sidebar easing between open and
collapsed, toasts sliding in, a page rising in. Use `motion::eased`, `motion::Fade`,
`motion::knob` and the existing `RAIL`/`PAGE` motions; no new endless animations.

## Navigation (the shell)

- **Title bar:** back, sidebar toggle, brand · the search field (Ctrl K) · Update (only when an
  update exists), the account button, window controls.
- **Sidebar:** Discover mods & packs, Servers, Skins · Profiles (with +) · Help & community,
  Settings. Collapses to icons with Ctrl B.
- **Profile page** replaces Home: the banner with the profile's mark, name, build line and
  "Change build"; Play and Profile settings; tabs Mods, Worlds, Packs, Saved servers, Logs.
  Installed mods and installed packs live in these tabs.
- **Discover** holds Mods, Modpacks, Resource packs, Shader packs and Data packs, with the
  "Adding to" profile picker.
- Servers (Join, Host, a server's pages), Skins and Settings keep their content in the new
  layout.

## Code layout

- Shared controls live in **`src/widgets/`** (one file per control: `button.rs`, `chip.rs`,
  `toggle.rs`, `field.rs`, `tabs.rs`, `row.rs`, `menu.rs`, `sun.rs`, …), declared from
  `src/widgets.rs`. A page builds from these; it doesn't draw its own button.
- When a control moves into `widgets/`, its old copies go in the same commit, and their callers
  move to it. No old and new side by side.
- **Clean up the old design as you go.** Whatever a step replaces is deleted in that step: old
  button and toggle variants (`glass_ui`, the blocky `widgets` look, `frame/controls` copies),
  the header tabs, the rail's accent edge, styles and tokens nothing uses any more, and their
  now-unused strings (`launcher_add_lang_keys.py --remove`) and tests. Before the branch merges,
  a final sweep removes anything left: `cargo clippy` dead-code warnings, unused `tr` keys
  (the i18n coverage test reports stale ones) and the `glass_`/`blocks` names AGENTS.md forbids.
- Pages stay in their domain folders (`home/`, `mods/`, `servers_page/`, `settings/`, …).
- File < 1,000 lines, function < 80. Split first if a change would cross either.
- Commits: a move/rename commit has no behaviour or look change; a look commit says what changed.

## Work order and status

| # | Area | Status |
|---|---|---|
| 1 | Theme tune and corner tokens | done |
| 2 | `widgets/`: button, chip, check, field/search, tabs, select, copy button, progress, empty state, banner, toast | done |
| 3 | Shell: title bar, sidebar nav, selection sun, collapse | done |
| 4 | Profile page (Home) and its tabs | done |
| 5 | Discover and detail pages | done |
| 6 | Servers, Skins, Settings | done |
| 7 | Sheets and popups | in progress |
| 8 | Compact window (800×600), light theme | light theme in progress |
| 9 | Final sweep: remove every leftover of the old design (the panel opacity, glass and blur settings, `HomePhoto::tone` and `Plates` are gone; the resolve sheet's own button colours are left) | to do |

Not built because the backend has no data for them yet (see the Figma Notes page): a
Modpacks kind in Discover, ping on server rows, the mods count and loader in the
profile's build line, a source filter in Discover.

## Checks before each merge into the branch

    cargo fmt --all --check
    cargo clippy --all-targets --locked -- -D warnings
    cargo test --locked --workspace --lib --bins
    python3 tools/ci/structure_check.py --base origin/main

and a look at the changed screens in a release build with scratch homes (see AGENTS.md,
"Running the launcher").
