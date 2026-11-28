#!/usr/bin/env python3
"""The launcher release's tag and notes, for release.yml: `decide` names the release, `notes` writes its title and body."""
import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
REPOSITORY = "sunrise-org/sunrise-launcher"
# The last version released from the game repository (launcher@1.0.1): installed copies take only a newer one.
SHIPPED_FROM_GAME_REPOSITORY = (1, 0, 1)
# A release is exactly X.Y.Z, as launchers and the game repository's bridge read it: no suffix, no leading zeros.
STABLE = re.compile(r"(0|[1-9][0-9]{0,8})\.(0|[1-9][0-9]{0,8})\.(0|[1-9][0-9]{0,8})")


def version() -> str:
    """The version in Cargo.toml's [package] section."""
    text = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    package = text.split("[package]", 1)[1].split("\n[", 1)[0]
    found = re.search(r'^version\s*=\s*"([^"]+)"', package, re.M)
    if not found:
        raise SystemExit("Cargo.toml has no [package] version")
    return found.group(1)


def tags(merged: str | None = None) -> list[str]:
    """This repository's release tags (`vX.Y.Z`), newest first."""
    cmd = ["git", "-C", str(ROOT), "tag", "--list", "v*", "--sort=-v:refname"]
    if merged:
        cmd.insert(4, f"--merged={merged}")
    out = subprocess.run(cmd, capture_output=True, text=True, check=True).stdout
    return [t for t in out.split() if re.fullmatch(r"v\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?", t)]


def published_tags() -> set[str]:
    """Tags with a published (non-draft) GitHub release."""
    out = subprocess.run(
        ["gh", "api", "--paginate", f"repos/{REPOSITORY}/releases", "--jq", ".[] | select(.draft | not) | .tag_name"],
        capture_output=True, text=True, check=True,
    ).stdout
    return {line.strip() for line in out.splitlines() if line.strip()}


def previous_release(all_tags: list[str], tag: str, published: set[str]) -> tuple[str | None, list[str]]:
    """The newest published tag before `tag`, and any unpublished tags skipped since it."""
    skipped = []
    for earlier in all_tags:
        if earlier == tag:
            continue
        if earlier in published:
            return earlier, skipped
        skipped.append(earlier)
    return None, []


def decide(head: str) -> dict:
    """The release Cargo.toml's version makes: refused unless it is a stable X.Y.Z newer than launcher@1.0.1, not tagged yet."""
    current = version()
    found = STABLE.fullmatch(current)
    if not found:
        raise SystemExit(f"Cargo.toml's version {current!r} is not a stable X.Y.Z")
    tag = f"v{current}"
    if tag in tags():
        raise SystemExit(f"{tag} is already released: raise Cargo.toml's version first")
    if tuple(int(n) for n in found.groups()) <= SHIPPED_FROM_GAME_REPOSITORY:
        raise SystemExit(f"{tag} isn't newer than launcher@1.0.1 from the game repository: raise Cargo.toml's version")
    if not re.fullmatch(r"[0-9a-f]{40}", head):
        raise SystemExit(f"{head!r} is not a commit")
    return {"version": current, "tag": tag, "head": head}


def merged_since(tag: str | None, head: str, base: str = "main") -> list[dict]:
    """The pull requests merged into `base` after `tag` (all, before the first release), from GitHub, newest first."""
    since = ""
    if tag:
        since = subprocess.run(["git", "-C", str(ROOT), "log", "-1", "--format=%cI", tag],
                               capture_output=True, text=True, check=True).stdout.strip()
    search = f"is:merged base:{base}" + (f" merged:>{since}" if since else "")
    out = subprocess.run(["gh", "pr", "list", "-R", REPOSITORY, "--state", "merged", "--search", search,
                          "--limit", "200", "--json", "number,title,labels,mergeCommit"],
                         capture_output=True, text=True, check=True).stdout
    rev_range = f"{tag}..{head}" if tag else head
    reachable = subprocess.run(["git", "-C", str(ROOT), "rev-list", rev_range], capture_output=True, text=True,
                               check=True).stdout.split()
    return [p for p in json.loads(out) if (p.get("mergeCommit") or {}).get("oid") in reachable]


def notes(tag: str, pulls: list[dict], skipped: list[str] = ()) -> tuple[str, str]:
    """The release's title and Markdown body: features first, then fixes; `skip-release` pull requests left out."""
    shown = [p for p in pulls if "skip-release" not in {l["name"] for l in p["labels"]}]
    features = [p for p in shown if "feature" in {l["name"] for l in p["labels"]}]
    fixes = [p for p in shown if p not in features]
    lines = []
    if skipped:
        lines += [f"{t[1:]} was not released." for t in reversed(skipped)] + [""]
    for heading, group in (("New", features), ("Fixed and changed", fixes)):
        if group:
            lines += [f"### {heading}", ""] + [f"- {p['title']} (#{p['number']})" for p in group] + [""]
    if not features and not fixes:
        lines += ["Maintenance release.", ""]
    lines.append("Each file is listed in SHA256SUMS, signed in SHA256SUMS.sig; the launcher checks both before it updates.")
    return f"Sunrise Launcher {tag[1:]}", "\n".join(lines) + "\n"


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    d = sub.add_parser("decide")
    d.add_argument("--head", required=True)
    n = sub.add_parser("notes")
    n.add_argument("--tag", required=True)
    n.add_argument("--head", required=True)
    # The branch released from: main, or a patch line whose pull requests target it.
    n.add_argument("--base", default="main")
    n.add_argument("--out", type=Path, required=True)
    args = parser.parse_args(argv)
    if args.command == "decide":
        print(json.dumps(decide(args.head)))
        return 0
    previous, skipped = previous_release(tags(args.head), args.tag, published_tags())
    title, body = notes(args.tag, merged_since(previous, args.head, args.base), skipped)
    args.out.mkdir(parents=True, exist_ok=True)
    (args.out / "title.txt").write_text(title + "\n")
    (args.out / "body.md").write_text(body)
    print(title)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
