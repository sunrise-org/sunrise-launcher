# Launcher commands

<!-- Generated from launcher/src/cli.rs by cli_tools::markdown(); a test fails when this file is out of date. Update it with: SUNRISE_WRITE_DOCS=1 cargo test --manifest-path launcher/Cargo.toml the_launcher_reference -->

Every command of `sunrise-launcher`, and the tool `sunrise-launcher mcp` offers for it. A tool's arguments are the command's, in snake_case. A tool runs the command with `--json` and returns what it printed. The tools that change the player's things for good need `confirm: true`. The tools that send something or reach other people say "only when the player asks". The game's own tools, `game_*`, are in [sunrise-ctl-commands.md](https://github.com/sunrise-org/sunrise/blob/main/docs/sunrise-ctl-commands.md). For unsigned developer installations only, `SUNRISE_LAUNCHER_DEVELOPER_OFFLINE=1` enables account-free Play. Only the exact value `1` enables it. The installed base must have persisted developer provenance, and the selected game must advertise `--developer-single-player` through `--launch-api`; older games must be rebuilt and reinstalled. Signed releases, Nightlies and their native descendants keep the account check. This mode supplies no session, blocks normal external connections and LAN hosting, and preserves managed launch leases, instance settings and unsigned labels. It is not a sandbox for native mods. If direct execution reports unregistered cache directories, use Sunrise Launcher Play from the same launcher data directory with the intended instance and installed version (`sunrise-launcher play --instance <instance> --version <tag>`).

With No AI on, `mcp` is hidden and refuses; every other command stays. See [agent-control.md](https://github.com/sunrise-org/sunrise/blob/main/docs/agent-control.md#no-ai).

## `sunrise-launcher completions`

Write a shell completion script to stdout.

    sunrise-launcher completions <SHELL>

- `SHELL` (bash, elvish, fish, powershell, zsh)

MCP: none: the player's switch.

## `sunrise-launcher versions`

List Sunrise releases, or the installed versions.

    sunrise-launcher versions

- `--installed`: Only the versions installed here (no network)

MCP: `versions`.

## `sunrise-launcher install`

Download and install a release (a tag such as v2026.10.13, or "latest").

    sunrise-launcher install <TAG>

- `TAG`

MCP: `install`.

## `sunrise-launcher developer-install`

Install a local unsigned developer build from a trusted checkout.

    sunrise-launcher developer-install --program <PROGRAM> --trust-unsigned <CHECKOUT>

- `CHECKOUT`: The trusted clean Sunrise checkout to package
- `--program`: The real sunrise-client executable built from that committed checkout
- `--trust-unsigned`: Allow executing this checkout's build code and unsigned game with your rights

Example: sunrise-launcher developer-install CHECKOUT --program CHECKOUT/target/debug/sunrise-client --trust-unsigned
This executes the checkout's build code and an unsigned game with your rights. Trust both before continuing.

MCP: none: the player's switch.

## `sunrise-launcher remove`

Remove an installed version.

    sunrise-launcher remove <TAG>

- `TAG`

MCP: `remove`, needs `confirm: true`.

## `sunrise-launcher import-build`

Install a game program you have, such as a sunrise-client built on your fork's Actions, as its own version on an installed base's game files, and select it. A local unsigned developer base is allowed only when explicitly selected or named with --version. Nothing signed it: it runs with your rights, and updates never replace it.

    sunrise-launcher import-build <PATH>

- `PATH`: The program (sunrise-client, sunrise-client.exe on Windows), or a zip holding it
- `--version`: The installed base whose game files it plays with (default: the selected usable base, else the newest release or Nightly; developer bases need explicit selection)
- `--trust-unsigned`: You picked this build yourself and trust it with full access to this computer

The program must match this computer's executable format and carry the exact `sunrise-game-executable;`
identity signature embedded by ordinary and launcher-generated games. It carries no mod API level or build id,
and is not a signature of authenticity. Unmarked programs are refused.

MCP: none: the player's switch.

## `sunrise-launcher prepare`

Prepare a version's resources from your own Minecraft install, or from Mojang.

    sunrise-launcher prepare

- `--minecraft`: The game folder of the official launcher, Prism Launcher, MultiMC, the Modrinth App, ATLauncher or GDLauncher (default: the launcher's, then every launcher's default)
- `--download`: Download Minecraft from Mojang for the signed-in account that owns it, instead of using an install
- `--version`: The installed version (default: the selected one)

MCP: `prepare`.

## `sunrise-launcher play`

Start the client (needs a signed-in Microsoft account that owns Minecraft: Java Edition).

    sunrise-launcher play

- `--allow-jar-downloads`: Allow missing original Java asset/data jars to be downloaded from Modrinth
- `--version`: Installed version to play (default: profile version)
- `--instance`: The profile to play (its name or id; default: the selected one)
- `--world`: Open or create this world (a folder under saves) at once
- `--creative`: Creative mode for a new world
- `--join`: Join a friend's world with its invite code
- `--server`: Join this server at once: an address, or a server of the list by name or position
- `--arg`: An extra argument for the game, passed through as-is (repeatable), for example --server-thread
- `--account`: Play this launch as this signed-in account (its name or UUID), without changing the one `account select` picked: several games can play as several accounts at once

MCP: `play`, then the `game_*` tools.

## `sunrise-launcher server`

Run a dedicated server here; type commands into its console.

    sunrise-launcher server

- `--allow-jar-downloads`: Allow missing original Java asset/data jars to be downloaded from Modrinth
- `--dir`: The server's folder (default: the one the launcher uses)
- `--version`
- `--accept-eula`: Agree to the Minecraft EULA (https://aka.ms/MinecraftEULA)

MCP: `server`, `server_command`, `server_log`, `server_stop`.

## `sunrise-launcher storage`

Show disposable storage by category as JSON. Builds must be idle.

    sunrise-launcher storage

- `--clear`: Request cleanup of build-files, prebuilt-games, spare-game-builds, rust-compiler, other-caches or all rows; game-accessible caches refuse cleanup without enforceable lifetime exclusion

MCP: `storage`.

## `sunrise-launcher native-lock`

Generate the registry-only Cargo.lock for one native mod, without executing mod code.

    sunrise-launcher native-lock <FOLDER>

- `FOLDER`
- `--update-lock`: Explicitly replace an existing lock with newly resolved registry pins

MCP: `native_lock`.

## `sunrise-launcher native-build`

Native mods (docs/modding/src/how-to/native-mods.md): compile flat Rust mod projects against the profile's exact Sunrise source revision with the launcher's own Rust, as the version the profile then plays. Not sandboxed: the mods' code runs with your rights. If there are no native mods to build, it exits successfully; a server export still fails, as no engine is made.

    sunrise-launcher native-build [FOLDER]

- `--sync-internals`: Discover internal uses through compiler diagnostics. Prints resolved declarations as JSON for cargo sunrise-mod internals sync; never installs or selects a game
- `--catalog`
- `--catalog-output`: Copy discovery catalogs into this caller-owned empty directory
- `FOLDER`: A mod folder (sunrise-mod.toml with [native], Cargo.toml and src/), or a folder of them (default: the native mods installed and turned on in the profile)
- `--server-output`: Export mode: build a standalone dedicated-server engine into this folder instead of a profile's build. Needs a mod FOLDER; no profile is read or changed, and no launcher config is written
- `--allow-jar-downloads`: Allow exact-hash jar downloads into the local cache for the server export; jar bytes are never included in the exported folder
- `--instance`: The profile whose installed native mods to build, and which then plays the build (default: the selected one)
- `--version`: The installed version whose game files the build reuses, and whose commit it builds on when no mod pins one (default: the one the profile plays, so a rebuild stays on its Sunrise)
- `--update`: Move to the newest installed release or Nightly instead of the Sunrise the profile plays (rebuilds the mods for that pinned SDK)
- `--trust-native-code`: Allow native code for this install (remembered; revoke in Settings, Storage)
- `--profile` (optimized): Native mods use the pinned SDK's optimized compilation settings. The game is never rebuilt by a mod build
- `--leave-out-mods`: When a mod this build can't use was saved into the profile's worlds, build without it anyway: what the mod added to those worlds is lost when they are next opened (back them up in the game first). Without this the build stops and names the worlds, leaving the profile as it is

MCP: none: the player's switch.

## `sunrise-launcher notes`

Show a release's patch notes (default: the newest).

    sunrise-launcher notes [TAG]

- `TAG`

MCP: `notes`.

## `sunrise-launcher links`

Show the website, roadmap and documentation links.

    sunrise-launcher links

MCP: `links`.

## `sunrise-launcher account list`

The signed-in accounts and which one plays.

    sunrise-launcher account list

MCP: `account_list`.

## `sunrise-launcher account sign-in`

Sign in with a Microsoft account in your browser (device code). Play needs one that owns Minecraft: Java Edition.

    sunrise-launcher account sign-in

- `--for`: Sign this account (name or UUID) in again: another Microsoft account is refused and who plays stays the same

MCP: `account_sign_in`.

## `sunrise-launcher account refresh`

Renew an account's session now, even when its access token is still fresh.

    sunrise-launcher account refresh <WHO>

- `WHO`

MCP: `account_refresh`.

## `sunrise-launcher account session`

Renew an account's session for a game already running: prints the read-once session file it hands over (`--json`: {"session_file"} or {"authentication"} when it can't).

    sunrise-launcher account session <WHO>

- `WHO`

MCP: none: the game's own session handoff.

## `sunrise-launcher account sign-out`

Sign out: the account's tokens leave the keychain (and the secrets file, where it kept them).

    sunrise-launcher account sign-out <WHO>

- `WHO`: Its name or UUID

MCP: `account_sign_out`, needs `confirm: true`.

## `sunrise-launcher account select`

Play as this account.

    sunrise-launcher account select <WHO>

- `WHO`

MCP: `account_select`.

## `sunrise-launcher settings`

Show the launcher's settings, or set one (the Settings page): selected-version TAG, game-architecture auto|x64|arm64, minecraft-dir PATH, server-dir PATH ("default" resets a folder), prune-old-versions on|off (delete old versions after an install, keeping the selected one, the ones profiles use and the newest few), keep-versions N (how many of the newest to keep; "default" is 2), opening-animation on|off, cat on|off (the corner cat), proxy http://host:port|socks5://host:port ("default" removes it), while-playing stay|minimize|hide (what the window does while the game runs), language system|CODE (the window's language: en, fr, de, ja, pt-br... any language the launcher has; system follows the computer's; a wrong code lists them all), theme default|ID|local:SLUG (a built-in theme, or one of yours from the themes folder; "default" is Dawn; a wrong id lists them all), accent violet|blue|teal|green|amber|red|pink|#rrggbb|DEGREES (a named accent, any colour, or any hue; "theme" goes back to the theme's own), background PATH (your picture behind Home; "default" goes back to the theme's), scenery on|off (whether anything is behind the pages), panel-opacity PERCENT|default (glass panels over Home's background), reduced-graphics on|off (no glass blur or decorative motion), cache-limit-gib N (managed cache limit in GiB: 10, 25 (default), 50, 100; 0 disables automatic cleanup).

    sunrise-launcher settings [KEY] [VALUE]

- `KEY` (selected-version, minecraft-dir, server-dir, prune-old-versions, keep-versions, opening-animation, cat, game-architecture, proxy, while-playing, worlds, language, theme, accent, background, scenery, panel-opacity, reduced-graphics, cache-limit-gib)
- `VALUE`: The new value (with KEY)

worlds rows|tiles: how Home lists the worlds under Continue playing.

MCP: `settings`.

## `sunrise-launcher game-args`

Show or set the extra arguments the game starts with (for example --server-thread); quotes group a word with spaces in it.

    sunrise-launcher game-args [ARGS]

- `ARGS`: Omit to show the current arguments

MCP: `game_args`.

## `sunrise-launcher share-log`

Upload a profile's latest.log to mclo.gs (a paste service for Minecraft logs) and print its link, for a bug report: only when the player asks. The home folder, the user name and the account names are taken out first; the link expires after 90 days unread.

    sunrise-launcher share-log

- `--instance`: The profile (default: the selected one)

MCP: `share_log`, only when the player asks.

## `sunrise-launcher unshare-log`

Delete a log shared with share-log: the "<id>:<token>" it printed.

    sunrise-launcher unshare-log <SHARED>

- `SHARED`

MCP: `unshare_log`, needs `confirm: true`.

## `sunrise-launcher logs`

Print the end of the game's log (logs/latest.log in the game folder), or its newest crash report.

    sunrise-launcher logs

- `--lines`: How many lines of latest.log
- `--crash`: The newest crash report instead

MCP: `logs`.

## `sunrise-launcher update`

Look for a newer Sunrise release and launcher, and install them.

    sunrise-launcher update

- `--check`: Only report what is available

MCP: `update`.

## `sunrise-launcher desktop-entry`

Linux: put Sunrise in the applications menu (an entry and the icon, under $XDG_DATA_HOME), or take it out with --remove.

    sunrise-launcher desktop-entry

- `--remove`: Remove the entry and the icon, and stop adding them back

MCP: `desktop_entry`.

## `sunrise-launcher bug-report`

Write a bug report to a file you attach to the issue by hand; nothing is sent (docs/telemetry.md).

    sunrise-launcher bug-report <TEXT>

- `TEXT`
- `--log`: Add the end of the game's log (names and worlds hidden, without chat)

MCP: `bug_report`.

## `sunrise-launcher data status`

What is allowed, the install ID and how many items wait (the default).

    sunrise-launcher data status

MCP: `data_status`.

## `sunrise-launcher data allow`

Turn kinds on: performance, crashes, bugs, benchmarks, or all.

    sunrise-launcher data allow <KINDS>...

- `KINDS`

MCP: `data_allow`, only when the player asks.

## `sunrise-launcher data deny`

Turn kinds off: performance, crashes, bugs, benchmarks, or all.

    sunrise-launcher data deny <KINDS>...

- `KINDS`

MCP: `data_deny`.

## `sunrise-launcher data show`

Print exactly what would be sent.

    sunrise-launcher data show

MCP: `data_show`.

## `sunrise-launcher data delete`

Delete everything sent from here, and what waits, and take a new install ID.

    sunrise-launcher data delete

MCP: `data_delete`, needs `confirm: true`.

## `sunrise-launcher data send`

Send what is waiting now.

    sunrise-launcher data send

MCP: `data_send`, only when the player asks.

## `sunrise-launcher data bug`

Send a bug report you write; it is printed first, as it would leave.

    sunrise-launcher data bug <TEXT>

- `TEXT`
- `--log`: Add the end of the game's log (scrubbed, without chat)
- `--yes`: Send without asking

MCP: `data_bug`, only when the player asks.

## `sunrise-launcher data benchmark`

Upload a benchmark report (docs/benchmark-report.md); it is printed first.

    sunrise-launcher data benchmark <FILE>

- `FILE`
- `--yes`

MCP: `data_benchmark`, only when the player asks.

## `sunrise-launcher instance list`

The profiles, each with its game folder (* = the one Play starts).

    sunrise-launcher instance list

MCP: `instance_list`.

## `sunrise-launcher instance create`

Make a profile: empty but for your options and name, or a copy of another one's worlds, options and packs.

    sunrise-launcher instance create <NAME>

- `NAME`
- `--copy`: The profile to copy (its name or id)

MCP: `instance_create`.

## `sunrise-launcher instance rename`

Rename a profile (its folder stays).

    sunrise-launcher instance rename <INSTANCE> <NAME>

- `INSTANCE`
- `NAME`

MCP: `instance_rename`.

## `sunrise-launcher instance delete`

Delete a profile: its folder (worlds, options, packs) goes to the Trash, where you can get it back. Sunrise's own profile stays.

    sunrise-launcher instance delete <INSTANCE>

- `INSTANCE`

MCP: `instance_delete`.

## `sunrise-launcher instance icon`

Set a profile's icon: a block or item name from your prepared files (grass_block), a colour (#3fa7d6), or none.

    sunrise-launcher instance icon <INSTANCE> <ICON>

- `INSTANCE`
- `ICON`

MCP: `instance_icon`.

## `sunrise-launcher instance group`

Put a profile in a sidebar group, or take it out with no group.

    sunrise-launcher instance group <INSTANCE> [GROUP]

- `INSTANCE`
- `GROUP`

MCP: `instance_group`.

## `sunrise-launcher instance select`

Make Play start this profile.

    sunrise-launcher instance select <INSTANCE>

- `INSTANCE`

MCP: `instance_select`.

## `sunrise-launcher instance version`

Keep a profile on one installed Sunrise version, or "latest" to follow updates again.

    sunrise-launcher instance version <INSTANCE> <VERSION>

- `INSTANCE`
- `VERSION`

MCP: `instance_version`.

## `sunrise-launcher instance export`

Export a profile to share as a zip: its settings, packs, Sunrise mods and the worlds named. Never logs, screenshots or account data; worlds go without their per-player files.

    sunrise-launcher instance export [INSTANCE]

- `INSTANCE`: The profile (default: the selected one)
- `--world`: A world to put in it (its name or folder); repeat for more
- `--out`: Where to write it (default: "<name>.zip" in the current folder)

MCP: `instance_export`.

## `sunrise-launcher instance launch`

Show or set what runs around a profile's game: a shell command before it (the game starts only if it succeeds), a wrapper it runs through (`nice -n 5`), environment variables ("NAME=value NAME2=value") and a shell command after it exits. An empty value clears one. The game and the commands also get INST_NAME, INST_ID, INST_DIR, INST_MC_DIR. On Linux, also GameMode, MangoHud and the dedicated graphics card, when the computer has them.

    sunrise-launcher instance launch <INSTANCE>

- `INSTANCE`
- `--pre-launch`
- `--wrapper`
- `--post-exit`
- `--env`
- `--gamemode`: Linux: run it through GameMode (gamemoderun), on or off
- `--mangohud`: Linux: MangoHud's overlay, on or off
- `--dedicated-gpu`: Linux: the dedicated graphics card (DRI_PRIME, or NVIDIA's offload), on or off

MCP: `instance_launch`, needs `confirm: true`.

## `sunrise-launcher instance shortcut`

Put a shortcut on the desktop that plays a profile (`play --instance`): a .desktop file (also in the applications menu) on Linux, an app on macOS, a .lnk on Windows.

    sunrise-launcher instance shortcut [INSTANCE]

- `INSTANCE`: The profile (default: the selected one)
- `--remove`: Move the profile's shortcuts to the trash instead

MCP: `instance_shortcut`.

## `sunrise-launcher instance import`

Import what a file is: a shared profile (a game folder zip, such as `instance export` writes) becomes a new profile; a resource or shader pack goes into the profile (default: the selected one).

    sunrise-launcher instance import <FILE>

- `FILE`
- `--instance`: The profile a pack goes into

MCP: `instance_import`.

## `sunrise-launcher worlds list`

The worlds: name, mode, version, last played and size (the default).

    sunrise-launcher worlds list

- `--instance`: The profile (default: the selected one)

MCP: `worlds_list`.

## `sunrise-launcher worlds backup`

Zip a world into the profile's backups/ folder, as the game's Make Backup does.

    sunrise-launcher worlds backup <WORLD>

- `WORLD`: Its folder or name
- `--instance`

MCP: `worlds_backup`.

## `sunrise-launcher worlds copy`

Copy a world beside it, as "<name> (1)".

    sunrise-launcher worlds copy <WORLD>

- `WORLD`
- `--instance`

MCP: `worlds_copy`.

## `sunrise-launcher worlds delete`

Move a world to the system's trash.

    sunrise-launcher worlds delete <WORLD>

- `WORLD`
- `--instance`

MCP: `worlds_delete`, needs `confirm: true`.

## `sunrise-launcher worlds datapack-list`

A world's data packs, and which are on.

    sunrise-launcher worlds datapack-list <WORLD>

- `WORLD`
- `--instance`

MCP: `worlds_datapack_list`.

## `sunrise-launcher worlds datapack-add`

Copy a data pack zip into a world (the game turns it on when the world opens).

    sunrise-launcher worlds datapack-add <WORLD> <FILE>

- `WORLD`
- `FILE`
- `--instance`

MCP: `worlds_datapack_add`.

## `sunrise-launcher worlds datapack-remove`

Move a world's data pack to the system's trash.

    sunrise-launcher worlds datapack-remove <WORLD> <PACK>

- `WORLD`
- `PACK`: Its file name, with or without .zip
- `--instance`

MCP: `worlds_datapack_remove`, needs `confirm: true`.

## `sunrise-launcher packs list`

The resource and shader packs, and which are on (the default).

    sunrise-launcher packs list

- `--kind` (resource, shader): Only this kind
- `--instance`: The profile (default: the selected one)

MCP: `packs_list`.

## `sunrise-launcher packs add`

Copy a pack file in: a resource pack .zip, a shader pack .zip or a .sshader.

    sunrise-launcher packs add <FILE>

- `FILE`
- `--instance`

MCP: `packs_add`.

## `sunrise-launcher packs on`

Turn a pack on (a resource pack goes on top; a shader pack replaces the one that was on).

    sunrise-launcher packs on <PACK>

- `PACK`: Its file name or title
- `--kind` (resource, shader)
- `--instance`

MCP: `packs_on`.

## `sunrise-launcher packs off`

Turn a pack off.

    sunrise-launcher packs off <PACK>

- `PACK`
- `--kind` (resource, shader)
- `--instance`

MCP: `packs_off`.

## `sunrise-launcher packs remove`

Turn a pack off and move it to the system's trash.

    sunrise-launcher packs remove <PACK>

- `PACK`
- `--kind` (resource, shader)
- `--instance`

MCP: `packs_remove`, needs `confirm: true`.

## `sunrise-launcher java list`

The Java runtimes Sunrise downloaded from Mojang and the ones installed on this computer.

    sunrise-launcher java list

MCP: `java_list`.

## `sunrise-launcher java install`

Download Mojang's Java runtime: a major (8, 16, 17, 21, 25) or a component name from a version's JSON (java-runtime-delta).

    sunrise-launcher java install <JAVA>

- `JAVA`

MCP: `java_install`.

## `sunrise-launcher screenshots list`

The screenshots, the newest first, with their files (the default).

    sunrise-launcher screenshots list

- `--instance`: The profile (default: the selected one)

MCP: `screenshots_list`.

## `sunrise-launcher screenshots delete`

Move a screenshot to the system's trash.

    sunrise-launcher screenshots delete <SCREENSHOT>

- `SCREENSHOT`: Its file name, with or without .png
- `--instance`

MCP: `screenshots_delete`, needs `confirm: true`.

## `sunrise-launcher multiplayer list`

The servers of the game's Multiplayer screen (the default).

    sunrise-launcher multiplayer list

- `--instance`: The profile (default: the selected one)

MCP: `multiplayer_list`.

## `sunrise-launcher multiplayer add`

Add a server to the list.

    sunrise-launcher multiplayer add <ADDRESS>

- `ADDRESS`: Its address, as in the game's Add Server (host or host:port)
- `--name`: Its name in the list (default: Minecraft Server)
- `--instance`

MCP: `multiplayer_add`.

## `sunrise-launcher multiplayer remove`

Remove a server from the list: its position from 1, name or address.

    sunrise-launcher multiplayer remove <SERVER>

- `SERVER`
- `--instance`

MCP: `multiplayer_remove`, needs `confirm: true`.

## `sunrise-launcher modpack list`

The installed Sunrise modpacks (the default).

    sunrise-launcher modpack list

MCP: `modpack_list`.

## `sunrise-launcher modpack play`

Play a Sunrise modpack as its own profile.

    sunrise-launcher modpack play <PACK>

- `PACK`
- `--allow-jar-downloads`: Allow missing original Java asset/data jars to be downloaded from Modrinth
- `--version`

MCP: `modpack_play`.

## `sunrise-launcher modpack remove`

Remove a Sunrise modpack and the profile's saves.

    sunrise-launcher modpack remove <PACK>

- `PACK`

MCP: `modpack_remove`, needs `confirm: true`.

## `sunrise-launcher mods list`

What the browser installed in a profile (the default).

    sunrise-launcher mods list

- `--instance`: The profile (default: the selected one)

MCP: `mods_list`.

## `sunrise-launcher mods search`

Search for packs Sunrise loads, or the registry's mods and modpacks.

    sunrise-launcher mods search [TEXT]...

- `TEXT`
- `--kind` (mod, resource, modpack, shader, datapack)
- `--everything`: Also what was made for other versions or runs only partly, each marked
- `--source` (modrinth, sunrise)
- `--instance`: The profile (default: the selected one)

MCP: `mods_search`.

## `sunrise-launcher mods install`

Install a pack or a Sunrise mod and what it needs into a profile, by its id or slug (checksums checked); a Sunrise modpack is installed as a profile of its own; a data pack goes into the world `--world` names.

    sunrise-launcher mods install <PROJECT>

- `PROJECT`
- `--world`: Install the project's data pack into this world of the profile (its name or folder)
- `--instance`: The profile (default: the selected one)
- `--everything`: Allow a version made for another Minecraft version
- `--source` (modrinth, sunrise)

MCP: `mods_install`.

## `sunrise-launcher mods update`

Update what the browser installed (or one pack).

    sunrise-launcher mods update [PROJECT]

- `PROJECT`
- `--check`: Only say what is newer
- `--instance`: The profile (default: the selected one)

MCP: `mods_update`.

## `sunrise-launcher mods add-github`

Install a native mod from a GitHub link (docs/modding/src/how-to/native-mods.md, "Install from GitHub"): https://github.com/OWNER/REPO (its latest release, else its default branch), .../releases/tag/TAG, .../tree/REF or .../commit/SHA. Resolved to an exact release file or commit, checked like a registry mod. NOT reviewed by the Sunrise registry.

    sunrise-launcher mods add-github <URL>

- `URL`
- `--mod`: The mod to install when the link holds several
- `--all`: Install every mod the link holds
- `--instance`: The profile (default: the selected one)
- `--trust-native-code`: Allow native code for this install (remembered; revoke in Settings, Storage)

MCP: none: the player's switch.

## `sunrise-launcher mods update-github`

Look for newer versions of the native mods installed from GitHub (the latest release, or the default branch's head for a mod installed from a commit), and install them.

    sunrise-launcher mods update-github [ID]

- `ID`: One mod (default: all of them)
- `--check`: Only say what is newer
- `--instance`: The profile (default: the selected one)
- `--trust-native-code`: Allow native code for this install (remembered; revoke in Settings, Storage)

MCP: none: the player's switch.

## `sunrise-launcher mods remove`

Remove a pack the browser installed.

    sunrise-launcher mods remove <PROJECT>

- `PROJECT`
- `--instance`: The profile (default: the selected one)

MCP: `mods_remove`, needs `confirm: true`.

## `sunrise-launcher mods enable`

Turn on a mod the browser installed (sunrise-mods.txt; the game's Mods screen has no switch).

    sunrise-launcher mods enable <PROJECT>

- `PROJECT`
- `--instance`: The profile (default: the selected one)

MCP: `mods_enable`.

## `sunrise-launcher mods disable`

Turn off a mod the browser installed; it stays installed.

    sunrise-launcher mods disable <PROJECT>

- `PROJECT`
- `--instance`: The profile (default: the selected one)

MCP: `mods_disable`.

## `sunrise-launcher rust status`

Whether it is installed, its version and whether this computer can link with it.

    sunrise-launcher rust status

MCP: `rust_status`.

## `sunrise-launcher rust install`

Download the pinned toolchain from static.rust-lang.org (once), checked by SHA-256.

    sunrise-launcher rust install

MCP: `rust_install`.

## `sunrise-launcher rust remove`

Delete the toolchain and the native builds' Cargo home.

    sunrise-launcher rust remove

MCP: `rust_remove`.

## `sunrise-launcher servers list`

The servers, their folder and whether they run.

    sunrise-launcher servers list

MCP: `servers_list`.

## `sunrise-launcher servers create`

Create a server; with no options it is a new world, survival, normal, 10 players, friends only.

    sunrise-launcher servers create

- `--name`
- `--seed`: The new world's seed (default: random)
- `--copy-world`: Start from a copy of this world folder (one of your saves)
- `--mode` (survival, creative, adventure, spectator)
- `--difficulty` (peaceful, easy, normal, hard)
- `--max-players`
- `--open`: Let anyone in (no whitelist)
- `--offline`: Do not check players' Minecraft accounts (offline mode: anyone can use any name)
- `--player`: Who may join (repeatable); the first becomes an operator
- `--version`: The installed version whose programs and game files servers use, if none are set up yet
- `--accept-eula`: Agree to the Minecraft EULA (https://aka.ms/MinecraftEULA)

MCP: `servers_create`, only when the player asks.

## `sunrise-launcher servers set-up`

Copy an installed, prepared version's programs and game files for the servers (again).

    sunrise-launcher servers set-up

- `--version`

MCP: `servers_set_up`.

## `sunrise-launcher servers run`

Run a sunrise-server command on a server, for example: servers run "My server" -- status.

    sunrise-launcher servers run <SERVER> -- <ARGS>...

- `SERVER`: The server's name or folder
- `ARGS`

MCP: `servers_run`.

## `sunrise-launcher servers forget`

Take a server off the list (its folder stays).

    sunrise-launcher servers forget <SERVER>

- `SERVER`

MCP: `servers_forget`, needs `confirm: true`.

## `sunrise-launcher mcp`

Serve the Model Context Protocol on stdin and stdout, for AI agents (refused while No AI is on).

    sunrise-launcher mcp

MCP: none: this command is the MCP server.

## `sunrise-launcher no-ai`

Show or set No AI: on hides and turns off every AI feature (the companion, agent control, sunrise-ctl and both MCP servers); off, the default, makes them available.

    sunrise-launcher no-ai [STATE]

- `STATE` (on, off): Omit to show the current state

MCP: none: the player's switch.

## `sunrise-launcher benchmark`

Measure the launcher: starts (the first is the cold one), page switching, idle CPU and memory, the release list fetch; writes a benchmark report to benchmarks/ in its data folder.

    sunrise-launcher benchmark

- `--repeats`: How many times to start it

MCP: `benchmark`.
