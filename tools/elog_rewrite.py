#!/usr/bin/env python3
"""Rewrites every `eprintln!(` in the launcher's sources into `elog!(`, so what goes to stderr
also goes to the launcher log (src/launcher_log.rs).

Files of the lib crate (the modules reached from src/lib.rs) get `crate::elog!(`; files of the
bin crate (reached from src/main.rs) get `sunrise_launcher::elog!(`. Which crate a file belongs
to is decided by walking the `mod name;` declarations from both roots. Test code is left alone:
src/launcher_log.rs (the macro's own eprintln!), files named tests.rs or *_tests.rs, anything under a tests/ folder, and every line after a
`#[cfg(test)]` line.

Run once from the repository root: python3 tools/elog_rewrite.py [--dry-run]
"""
import re
import sys
from pathlib import Path

SRC = Path("src")
MOD = re.compile(r"^(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;")
PATH_ATTR = re.compile(r'^\s*#\[path\s*=\s*"([^"]+)"\s*\]')
# The macro's own eprintln! must stay.
LOGGER = SRC / "launcher_log.rs"
# `eprintln!()` (a bare newline, as after a progress line) has nothing to log and stays.
CALL = re.compile(r"(?<![\w:])eprintln!\((?!\))")


def children(file: Path):
    """The files `file` declares with `mod name;` (or `#[path]`)."""
    root = file.name in ("lib.rs", "main.rs", "mod.rs")
    base = file.parent if root else file.parent / file.stem
    path = None
    for line in file.read_text().splitlines():
        if line.lstrip().startswith("//"):
            continue
        attr = PATH_ATTR.match(line)
        if attr:
            path = attr.group(1)
            continue
        decl = MOD.match(line)
        if decl:
            if path:
                found = file.parent / path
            else:
                name = decl.group(1)
                found = next(
                    (c for c in (base / f"{name}.rs", base / name / "mod.rs") if c.exists()),
                    None,
                )
            if found and found.exists():
                yield found
            else:
                print(f"warning: {file}: mod {decl.group(1)} not found", file=sys.stderr)
        if not line.lstrip().startswith("#["):
            path = None


def walk(root: Path):
    seen = set()
    todo = [root]
    while todo:
        file = todo.pop()
        if file in seen:
            continue
        seen.add(file)
        todo.extend(children(file))
    return seen


def is_test_file(file: Path) -> bool:
    return (
        file == LOGGER
        or file.name == "tests.rs"
        or file.name.endswith("_tests.rs")
        or "tests" in file.parts[:-1]
    )


def rewrite(file: Path, macro: str, dry: bool) -> int:
    out = []
    count = 0
    in_tests = False
    for line in file.read_text().splitlines(keepends=True):
        if line.lstrip().startswith("#[cfg(test)]"):
            in_tests = True
        if not in_tests and not line.lstrip().startswith("//"):
            line, n = CALL.subn(f"{macro}(", line)
            count += n
        out.append(line)
    if count and not dry:
        file.write_text("".join(out))
    return count


def main():
    dry = "--dry-run" in sys.argv
    lib = walk(SRC / "lib.rs")
    bin_ = walk(SRC / "main.rs")
    for file in sorted(lib & bin_):
        print(f"reached from both crates (left alone): {file}")
    total = 0
    for crate, files, macro in (
        ("lib", lib - bin_, "crate::elog!"),
        ("bin", bin_ - lib, "sunrise_launcher::elog!"),
    ):
        for file in sorted(files):
            if is_test_file(file):
                continue
            n = rewrite(file, macro, dry)
            if n:
                print(f"{file} ({crate}): {n}")
                total += n
    reached = lib | bin_
    for file in sorted(SRC.rglob("*.rs")):
        if file not in reached and not is_test_file(file) and CALL.search(file.read_text()):
            print(f"not reached from either crate (left alone): {file}")
    print(f"{total} eprintln! calls {'would be ' if dry else ''}rewritten")


if __name__ == "__main__":
    main()
