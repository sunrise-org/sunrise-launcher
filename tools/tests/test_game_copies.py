"""The launcher's copies of game files (crates/, assets/) stay byte- and value-equal to the originals."""
import ast
import hashlib
import json
import os
import re
import sys
import unittest
from contextlib import contextmanager
from pathlib import Path

LAUNCHER = Path(__file__).resolve().parents[2]

# (the game's file or folder, the launcher's copy)
SAME_BYTES = (
    ("crates/sunrise-release-sign/release-keys.txt", "crates/release-verify/keys/sunrise-org-sunrise.txt"),
    ("crates/sunrise-release-sign/native-build-keys.txt", "crates/release-verify/keys/native-builds.txt"),
    ("crates/sunrise-release-sign/tests/signed", "crates/release-verify/tests/signed"),
    ("crates/sunrise-mod-package/tests/index", "crates/smod/tests/index"),
    ("crates/sunrise-theme/tokens.css", "crates/theme/tokens.css"),
    ("tools/ci/rust-version", "rust-version"),
    ("packaging/icons/hicolor/16x16/apps/sunrise.png", "assets/icons/hicolor/16x16/apps/sunrise.png"),
    ("packaging/icons/hicolor/24x24/apps/sunrise.png", "assets/icons/hicolor/24x24/apps/sunrise.png"),
    ("packaging/icons/hicolor/32x32/apps/sunrise.png", "assets/icons/hicolor/32x32/apps/sunrise.png"),
    ("packaging/icons/hicolor/48x48/apps/sunrise.png", "assets/icons/hicolor/48x48/apps/sunrise.png"),
    ("packaging/icons/hicolor/64x64/apps/sunrise.png", "assets/icons/hicolor/64x64/apps/sunrise.png"),
    ("packaging/icons/hicolor/128x128/apps/sunrise.png", "assets/icons/hicolor/128x128/apps/sunrise.png"),
    ("packaging/icons/hicolor/256x256/apps/sunrise.png", "assets/icons/hicolor/256x256/apps/sunrise.png"),
    ("packaging/icons/hicolor/512x512/apps/sunrise.png", "assets/icons/hicolor/512x512/apps/sunrise.png"),
    ("packaging/icons/hicolor/scalable/apps/sunrise.svg", "assets/icons/hicolor/scalable/apps/sunrise.svg"),
    ("packaging/icons/sunrise.icns", "assets/icons/sunrise.icns"),
    ("packaging/icons/sunrise.ico", "assets/icons/sunrise.ico"),
    ("docs/media/village.webp", "assets/media/village.webp"),
    ("docs/media/underwater.webp", "assets/media/underwater.webp"),
    ("docs/media/nether.webp", "assets/media/nether.webp"),
    ("crates/sunrise-mod-package/src/sdk/fixture.tar.xz", "crates/sdk/src/sdk/fixture.tar.xz"),
    ("crates/sunrise-mod-package/src/sdk/bomb.tar.xz", "crates/sdk/src/sdk/bomb.tar.xz"),
    ("crates/sunrise-mod-package/src/sdk/link.tar.xz", "crates/sdk/src/sdk/link.tar.xz"),
    ("crates/sunrise-mod-package/src/sdk/escape.tar.xz", "crates/sdk/src/sdk/escape.tar.xz"),
    ("crates/sunrise-mod-package/src/sdk/inputs.json", "crates/sdk/src/sdk/inputs.json"),
    ("LICENSE", "assets/LICENSE"),
    ("LICENSES/MIT-Horizon.txt", "assets/LICENSES/MIT-Horizon.txt"),
    ("crates/sunrise-prepare/tests/support/java_probe.rs", "tests/support/java_probe.rs"),
    ("crates/sunrise-port-pack/src/zip_intake/source.rs", "src/zip_intake/source.rs"),
    ("crates/sunrise-port-pack/src/zip_intake/tests.rs", "src/zip_intake/tests.rs"),
)

# (the game's source, the launcher's copy, the edits that turn one into the other): compared as code, ignoring
# layout, module docs and `use` items, which the launcher's edition formats and orders its own way.
SAME_CODE = (
    ("crates/sunrise-port-pack/src/net.rs", "src/net.rs", (
        ("pub(crate) fn get_from_hosts", "pub fn get_from_hosts"),
        # Nightly sources reuse the same proxy-aware, no-redirect request for their own fetches.
        ("""let agent = keep(
            "port-pack-no-redirect",
            spec_for(address).as_deref(),
            |proxy| {
                let builder = ureq::AgentBuilder::new().redirects(0);
                match proxy {
                    Some(proxy) => builder.proxy(proxy).build(),
                    None => builder.build(),
                }
            },
        );
        match agent
            .get(address)
            .set("User-Agent", crate::modrinth::AGENT)
            .call()""", "match anonymous_get_no_redirect(address).call()"),
        ("/// GET from allowed HTTPS hosts, validating every redirect before contacting it.",
         """/// Anonymous GET with the configured proxy, leaving redirects and status to the caller.
pub(crate) fn anonymous_get_no_redirect(address: &str) -> ureq::Request {
    let agent = keep(
        "port-pack-no-redirect",
        spec_for(address).as_deref(),
        |proxy| {
            let builder = ureq::AgentBuilder::new().redirects(0);
            match proxy {
                Some(proxy) => builder.proxy(proxy).build(),
                None => builder.build(),
            }
        },
    );
    agent.get(address).set("User-Agent", crate::modrinth::AGENT)
}

/// GET from allowed HTTPS hosts, validating every redirect before contacting it."""),
        ('.set("User-Agent", crate::modrinth::AGENT)',
         '.set("User-Agent", concat!("sunrise-launcher/", env!("CARGO_PKG_VERSION")))'),
    )),
    ("crates/sunrise-port-pack/src/zip_intake.rs", "src/zip_intake.rs", (
        ("""if let Ok(directory) = zip64
            && (directory.entries, directory.size, directory.offset)
                == (classic.entries, classic.size, classic.offset)
        {
            return Ok((directory, Some(at)));
        }""", """if let Ok(directory) = zip64 {
            if (directory.entries, directory.size, directory.offset)
                == (classic.entries, classic.size, classic.offset)
            {
                return Ok((directory, Some(at)));
            }
        }"""),
    )),
    ("crates/sunrise-mod-package/src/modpack.rs", "crates/smod/src/modpack.rs", (
        ('#[cfg(feature = "index")] impl PackMod', "impl PackMod"),
    )),
)


# (the game's file, a function in it, the SHA-256 of its code, the launcher's function that does the same for show):
# the launcher reads these few things itself, so a change to the game's function is a change to make there too.
MIRRORED = (
    ("crates/sunrise-port-pack/src/import.rs", "prism_archive_layout",
     "9c5580515b41ec63c768d9de42054348e3dec65cc7c04705b0f48cb721929b67", "src/import.rs prism_instance"),
    ("crates/sunrise-port-pack/src/import.rs", "curseforge_instance_target",
     "59251c22e6e81e67985e2436edfbc7d92f1c5a4b8302f65bae4f901c7d67e2da",
     "src/other_launchers.rs curseforge_instance_target"),
    ("crates/sunrise-port-pack/src/import.rs", "curseforge",
     "85ff06bf7e97dd51c6d2410587acd3c3d5519fec93e6734b87cef0309003d29c",
     "src/other_launchers.rs curseforge_instance_target"),
    ("crates/sunrise-port-pack/src/jar.rs", "read_inner",
     "a1fdeb9b3daaf5fbd74cd4e18185beb452759b24e3e2cedabd17595ec5d5850d", "src/browser/java.rs jar_title"),
    ("crates/sunrise-port-pack/src/jar.rs", "fabric",
     "052665a32cf9980ffcd1d1cb51f72a918e6613d89fb2097562afb306ca4f4fe2", "src/browser/java.rs jar_title"),
    ("crates/sunrise-port-pack/src/jar.rs", "manifest_value",
     "8eba4a800351645479e1f2fc79c727bf2e3aee7bdcc97b27a3fa249791a2b33e", "src/browser/java.rs jar_title"),
    ("crates/sunrise-native-build/src/lib.rs", "bundles",
     "8929a560dc4a8fb4bf49243cd415b7f6bdad037278d5eaaf4c6e64f366222356", "src/native_mods/bundle.rs bundles"),
    ("crates/sunrise-native-build/src/lib.rs", "scan_bundles",
     "0186b6a99c78d6c3296063bc8d4c31134c035e787d25a146d7dc1839f20ae974",
     "src/native_mods/bundle.rs scan_bundles"),
    ("crates/sunrise-native-build/src/lib.rs", "failure",
     "ce65c921695767d21d8c3047d36b48b58d843a78ac4b56d2f77117c68a016430", "src/native_mods/build_log.rs failure"),
    ("crates/sunrise-native-build/src/lib.rs", "unpack_archive",
     "08b67ddd8289e49ed1ce5c81ff77ee25bfca94a9d6cbf0b85df68a28e203eb0c", "src/github_mods.rs unpack_archive"),
    # The launcher's unpacker is stricter (size, duplicate and Windows-reserved paths, cancellation).
    ("crates/sunrise-native-build/src/lib.rs", "unpack_source",
     "356c9c82b888214518bbf5c6949f4ebfd4963e24993dda7bc11db877a33b2187", "src/github_mods.rs unpack_bounded"),
    ("crates/sunrise-native-build/src/package.rs", "read",
     "94d760c8ea6f5452f20a6a816d88207675ed454e44cc0973240cff8069aaecf0",
     "src/native_mods/bundle/package.rs read"),
    ("crates/sunrise-native-build/src/package.rs", "copy_files",
     "ea02d6cc0ec18a4fe33804e4ee67bd2fe49e6e6d410b30fd56872f9e0c2e317a",
     "src/native_mods/bundle/package.rs copy_files"),
)


def function(path: Path, name: str) -> str:
    """The text of `fn name` in the Rust file at `path`, from `fn` to its closing brace."""
    source = path.read_text(encoding="utf-8")
    found = re.search(rf"\bfn {name}\b", source)
    if not found:
        raise AssertionError(f"{path} has no fn {name}")
    depth, at = 0, source.index("{", found.end())
    while at < len(source):
        rest = source[at:]
        raw = re.match(r'r(#*)"', rest)
        char = re.match(r"'(\\.|[^\\'])'", rest)
        if rest.startswith("//"):
            at = source.find("\n", at)
        elif raw and (at == 0 or not (source[at - 1].isalnum() or source[at - 1] == "_")):
            at = source.index('"' + raw.group(1), at + len(raw.group(0))) + len(raw.group(1))
        elif rest.startswith('"'):
            at = re.compile(r'"(\\.|[^"\\])*"', re.S).match(source, at).end() - 1
        elif char:
            at += len(char.group(0)) - 1
        elif rest[0] in "{}":
            depth += 1 if rest[0] == "{" else -1
            if depth == 0:
                return source[found.start():at + 1]
        at += 1
    raise AssertionError(f"{path}: fn {name} never closes")


def code(text: str) -> str:
    """Rust source as this test compares it: no module docs, `use` items, whitespace or trailing commas."""
    text = re.sub(r"^\s*//!.*$", "", text, flags=re.M)
    text = re.sub(r"^\s*(pub(\(crate\))? )?use [^;]*;", "", text, flags=re.M | re.S)
    return re.sub(r",([)\]}])", r"\1", re.sub(r"\s+", "", text))


# (the game's file, the launcher's copy, the functions copied whole): compared as code, whole-line comments aside.
SAME_FUNCTIONS = (
    ("crates/sunrise-prepare/src/java.rs", "src/jdk.rs", (
        "command", "executable", "real_path", "without_verbatim", "path_homes", "tool_major", "jdk_parents",
        "installed_jdks", "find", "find_runtime", "find_tools", "home",
    )),
    ("crates/sunrise-mod-package/src/manifest.rs", "crates/sdk/src/lib.rs", ("revision",)),
    ("crates/sunrise-mod-package/src/sdk.rs", "crates/sdk/src/lib.rs", (
        "route", "to_json", "build_of", "schema", "asset", "route_assets", "verify", "unpack", "lock",
        "no_locks", "rename_retrying", "busy", "extract", "untar", "entry_target",
    )),
    ("crates/sunrise-native-build/src/lib.rs", "src/native_mods/bundle.rs", (
        "bundle_in", "bundle_files", "digest", "source_digest", "copy_bundle",
    )),
    ("crates/sunrise-native-build/src/lib.rs", "src/native_mods/build_log.rs", (
        "log_path", "clear_log", "append_log", "mod_id_from_file", "diagnostic",
    )),
    ("crates/sunrise-native-build/src/prebuilt.rs", "src/native_mods/build_log.rs", ("compiling",)),
    ("crates/sunrise-native-build/src/prebuilt.rs", "src/native_mods/sdk_cache.rs", ("at",)),
    ("crates/sunrise-native-build/src/rust.rs", "src/rust_toolchain.rs", ("scrubbed", "low_priority")),
    ("crates/sunrise-mod-surface/src/uses.rs", "src/native_mods/uses.rs", ("item",)),
    ("crates/sunrise-mod-surface/src/lib.rs", "src/native_mods/uses.rs", ("leaf",)),
)

# (the game's file, the launcher's copy, its edits, the types, impls and constants copied whole, by their head).
SAME_ITEMS = (
    ("crates/sunrise-mod-package/src/sdk.rs", "crates/sdk/src/lib.rs", (
        ('#[cfg(feature = "prebuilt")]', ""),
        ("crate::manifest::revision", "revision"),
        # The direct reader is the launcher rather than cargo sunrise-mod.
        (r""", so it is older than this SDK. Update it (`cargo sunrise-mod`: run the install block of the book's \"Install the mod tool\" page), or use the tool in this SDK's own release""",
         ". Update it to use this SDK"),
    ), (
        "pub const INDEX", "pub const SCHEMA", "pub struct Index", "pub struct Build", "pub struct Toolchain",
        "pub struct Surface", "pub struct ChangeNote", "pub enum Verdict", "pub struct Mixins", "pub struct Asset",
        "pub enum Kind", "impl Kind", "pub enum Codec", "pub enum Side", "impl Asset", "pub struct Inputs",
        "pub struct Route", "pub enum Refusal", "impl fmt::Display for Refusal", "impl std::error::Error for Refusal",
        "struct Written", "struct WrittenAsset", "pub const DICTIONARY_LIMIT", "pub const EXTENSION_LIMIT",
        "pub const LOCK_SUFFIX", "pub const PARTIAL_SUFFIX", "const PAX_KEYS", "struct Partial", "impl Partial",
        "impl Drop for Partial", "pub fn read",
    )),
    ("crates/sunrise-native-build/src/lib.rs", "src/native_mods/bundle.rs", (
        ("sunrise_mod_package::", "sunrise_launcher_smod::"),
        ("""pub fn needs_mixins(&self) -> bool {
        self.manifest.native.as_ref().is_some_and(|native| {
            !native.mixin.is_empty()
                || !native.module.is_empty()
                || !native.bridge.is_empty()
                || !native.state.is_empty()
        })
    }""", ""),
    ), ("pub enum Target", "impl Target", "pub const NATIVE_MANIFEST", "pub struct Bundle", "impl Bundle")),
    ("crates/sunrise-native-build/src/lib.rs", "src/native_mods/build_log.rs", (), (
        "pub const CANCELLED",  # Cancellation state belongs to each launcher attempt, not a process global.
    )),
    ("crates/sunrise-native-build/src/prebuilt.rs", "src/native_mods/build_log.rs", (), (
        "pub enum Refusal", "impl std::fmt::Display for Refusal", "impl Refusal",
    )),
    ("crates/sunrise-native-build/src/package.rs", "src/native_mods/bundle/package.rs", (
        ("sunrise_mod_package::", "sunrise_launcher_smod::"), ("crate::host::trf", "trf"),
    ), ("fn checked_files", "pub fn files", "pub fn copy")),
    ("crates/sunrise-native-build/src/registry_inventory.rs", "src/native_mods/tool.rs", (
        ("crate::host::", ""),
    ), ("pub fn label", "pub fn detail")),
    ("crates/sunrise-native-build/src/rust.rs", "src/rust_toolchain.rs", (), (
        "pub const WINDOWS_X64_TARGET", "pub const SCRUBBED", "const IOPRIO_WHO_PROCESS", "const LOW_IO",
    )),
    ("crates/sunrise-mod-surface/src/uses.rs", "src/native_mods/uses.rs", (), ("pub enum ItemKind",)),
)


def item(path: Path, head: str) -> str:
    """The text of the item starting `head` in the Rust file at `path`, from its attributes to its end."""
    source = path.read_text(encoding="utf-8")
    found = re.search(rf"^[ \t]*{re.escape(head)}\b", source, re.M)
    if not found:
        raise AssertionError(f"{path} has no {head}")
    start = found.start()
    while (above := re.search(r"^[ \t]*(#\[.*\]|//.*)\n\Z", source[:start], re.M)) is not None:
        start = above.start()
    depth, at = 0, found.end()
    while at < len(source):
        rest = source[at:]
        if rest.startswith("//"):
            at = source.find("\n", at)
        elif rest.startswith('"'):
            at = re.compile(r'"(\\.|[^"\\])*"', re.S).match(source, at).end() - 1
        elif rest[0] in "{[(":
            depth += 1
        elif rest[0] in "}])":
            depth -= 1
            if depth == 0 and rest[0] == "}":
                return source[start:at + 1]
        elif rest[0] == ";" and depth == 0:
            return source[start:at + 1]
        at += 1
    raise AssertionError(f"{path}: {head} never closes")


def uncommented(text: str) -> str:
    """`text` without its whole-line comments: a copy may say less than the game's comments."""
    return re.sub(r"^\s*//.*$", "", text, flags=re.M)


def files(path: Path) -> dict:
    if path.is_file():
        return {"": path.read_bytes()}
    return {str(f.relative_to(path)): f.read_bytes() for f in sorted(path.rglob("*")) if f.is_file()}


def value(expression: str):
    """A Rust constant's value: a string literal, or integers joined by <<, *, + and -."""
    expression = expression.strip()
    if expression.startswith('"'):
        return ast.literal_eval(expression)
    try:
        tree = ast.parse(re.sub(r"(\d)_(?=\d)", r"\1", expression), mode="eval").body
    except SyntaxError as error:
        raise ValueError(f"not a constant this test reads: {expression}") from error

    def evaluate(node):
        if isinstance(node, ast.Constant) and isinstance(node.value, int):
            return node.value
        if isinstance(node, ast.BinOp):
            ops = {ast.LShift: lambda a, b: a << b, ast.Mult: lambda a, b: a * b,
                   ast.Add: lambda a, b: a + b, ast.Sub: lambda a, b: a - b}
            return ops[type(node.op)](evaluate(node.left), evaluate(node.right))
        raise ValueError(f"not a constant this test reads: {expression}")

    return evaluate(tree)


def constant(path: Path, name: str):
    source = path.read_text(encoding="utf-8")
    found = re.search(rf"\bconst {name}\s*:\s*[^=]+=\s*(.+?);", source, re.S)
    if not found:
        raise AssertionError(f"{path} has no const {name}")
    return value(found.group(1))


PACKAGE_FORMAT = "crates/sunrise-mod-package/src/sdk/package-format"

# (the game's file and constant, the launcher's file and constant)
FORMAT_CONSTANTS = (
    ("crates/sunrise-mod-surface/src/uses.rs", "USES_FILE", "crates/smod/src/manifest.rs", "USES_FILE"),
    ("crates/sunrise-mod-surface/src/uses.rs", "MAX_USES_BYTES", "crates/smod/src/manifest.rs", "MAX_USES"),
    ("crates/sunrise-core/src/prepared_files.rs", "FILE_NAME", "src/preparer.rs", "DATA_FILE"),
    ("crates/sunrise-mod-package/src/mixins_tool.rs", "REV", "src/native_mods/bundle.rs", "MIXINS_REV"),
    ("crates/sunrise-core/src/version.rs", "VERSION_ID", "src/backend.rs", "PINNED_GAME"),
)


class LauncherCopies(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        game_dir = os.environ.get("SUNRISE_GAME_DIR")
        if not game_dir:
            raise RuntimeError("Set SUNRISE_GAME_DIR to the Sunrise checkout to compare against")
        cls.game = Path(game_dir)
        if not cls.game.is_dir():
            raise RuntimeError(f"SUNRISE_GAME_DIR is not a directory: {cls.game}")

    @contextmanager
    def comparing(self, table, game, launcher, **labels):
        self.copy_row = (table, game, launcher)
        with self.subTest(**labels):
            yield

    def test_copied_files_are_the_games_byte_for_byte(self):
        for game, copy in SAME_BYTES:
            with self.comparing("SAME_BYTES", game, copy, copy=copy):
                original, copied = files(self.game / game), files(LAUNCHER / copy)
                self.assertTrue(original, f"{game} is empty or missing")
                self.assertEqual(sorted(copied), sorted(original), f"{copy} lists other files than {game}")
                for name in original:
                    where = f"{name} in " if name else ""
                    self.assertEqual(copied[name], original[name], f"{where}{copy} differs from {game}")

    def test_rewritten_copies_are_the_games_code_with_their_listed_edits(self):
        for game, copy, edits in SAME_CODE:
            with self.comparing("SAME_CODE", game, copy, copy=copy):
                original = code((self.game / game).read_text(encoding="utf-8"))
                for old, new in edits:
                    self.assertIn(code(old), original, f"{game} no longer has the code {copy} edits: update both")
                    original = original.replace(code(old), code(new))
                self.assertEqual(code((LAUNCHER / copy).read_text(encoding="utf-8")), original,
                                 f"{copy} differs from {game} beyond its listed edits: copy the change, or list it")

    def test_mirrored_game_functions_are_unchanged(self):
        for path, name, pinned, mirror in MIRRORED:
            with self.comparing("MIRRORED", path, mirror.split(" ", 1)[0], function=name):
                digest = hashlib.sha256(code(function(self.game / path, name)).encode()).hexdigest()
                self.assertEqual(digest, pinned, f"{path}'s {name} changed: change {mirror} to match, then pin {digest}")

    def test_functions_copied_whole_are_the_games(self):
        for game, copy, names in SAME_FUNCTIONS:
            for name in names:
                with self.comparing("SAME_FUNCTIONS", game, copy, function=name):
                    original = code(uncommented(function(self.game / game, name)))
                    self.assertEqual(code(uncommented(function(LAUNCHER / copy, name))), original,
                                     f"{copy}'s {name} differs from {game}'s: copy the change")

    def test_items_copied_whole_are_the_games(self):
        for game, copy, edits, heads in SAME_ITEMS:
            for head in heads:
                with self.comparing("SAME_ITEMS", game, copy, item=head):
                    original = code(uncommented(item(self.game / game, head)))
                    for old, new in edits:
                        original = original.replace(code(old), code(new))
                    self.assertEqual(code(uncommented(item(LAUNCHER / copy, head))), original,
                                     f"{copy}'s {head} differs from {game}'s: copy the change")

    def test_format_constants_match_the_games(self):
        for copy in ("crates/smod/src/index/native.rs", "crates/sdk/src/lib.rs"):
            with self.comparing("FORMAT_CONSTANTS", PACKAGE_FORMAT, copy, copy=copy, constant="PACKAGE_FORMAT"):
                package_format = int((self.game / PACKAGE_FORMAT).read_text().strip())
                self.assertEqual(constant(LAUNCHER / copy, "PACKAGE_FORMAT"), package_format,
                                 f"{copy}'s PACKAGE_FORMAT differs from {PACKAGE_FORMAT}: copy the change")
        for game, name, copy, copied_name in FORMAT_CONSTANTS:
            with self.comparing("FORMAT_CONSTANTS", game, copy, copy=copy, constant=copied_name):
                self.assertEqual(constant(LAUNCHER / copy, copied_name), constant(self.game / game, name),
                                 f"{copy}'s {copied_name} differs from {game}'s {name}: copy the change")

    def test_a_drifted_copy_is_caught(self):
        self.assertEqual(code("//! Doc.\nuse a::{b,\n c};\nfn f() { g( 1 ) }\n"), "fnf(){g(1)}")
        self.assertNotEqual(code("fn f() { g(1) }"), code("fn f() { g(2) }"))
        self.assertEqual(value("4 << 20"), 4 * 1024 * 1024)
        self.assertEqual(value("1_000"), 1000)
        self.assertEqual(value('"uses.json"'), "uses.json")
        with self.assertRaises(ValueError):
            value("mod_surface::uses::MAX_USES_BYTES as u64")
        self.assertEqual(code(uncommented("fn f() {\n    // Why.\n    g(1)\n}")), "fn f(){g(1)}".replace(" ", ""))


def load_tests(loader, tests, pattern):
    # Release-tool discovery has no game checkout; the format job runs this file explicitly with the pin.
    if pattern is not None and not os.environ.get("SUNRISE_GAME_DIR"):
        return unittest.TestSuite()
    return tests


class CopyReport(unittest.TextTestResult):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.failed_rows = []

    def addSubTest(self, test, subtest, err):
        super().addSubTest(test, subtest, err)
        if err is not None:
            self.failed_rows.append(test.copy_row)


def write_report(path, suite):
    """Return 1 for drift and 2 for a check that could not run to completion."""
    result = unittest.TextTestRunner(resultclass=CopyReport).run(suite)
    if result.skipped or len(result.failed_rows) != len(result.failures) + len(result.errors):
        return 2
    path.write_text(json.dumps(list(dict.fromkeys(result.failed_rows))) + "\n", encoding="utf-8")
    return int(bool(result.failed_rows))


if __name__ == "__main__":
    if sys.argv[1:] == ["--paths"]:
        tables = (SAME_BYTES, SAME_CODE, MIRRORED, SAME_FUNCTIONS, SAME_ITEMS, FORMAT_CONSTANTS)
        paths = {row[0] for table in tables for row in table} | {PACKAGE_FORMAT}
        print("\n".join(sorted(paths)))
    elif len(sys.argv) == 3 and sys.argv[1] == "--report":
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(LauncherCopies)
        sys.exit(write_report(Path(sys.argv[2]), suite))
    else:
        unittest.main()
