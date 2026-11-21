#!/usr/bin/env python3
"""Local developer SDK fixture: compile the staged source and return the exact bytes."""
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys

root = Path(os.environ["FIXTURE_NATIVE_ROOT"])
plan = json.loads(sys.stdin.readline())
command = sys.argv[1]
if command == "native-layout":
    print(json.dumps({"package": {part: str(root / "sdk" / part)
                                  for part in ("runtime", "api", "engine")}}))
elif command == "native-build":
    mod_id = "native_lantern"
    source = Path(plan["mods"]) / mod_id
    version = re.search(r"^version='([^']+)'", (source / "sunrise-mod.toml").read_text(), re.M)[1]
    with (root / "compiled").open("a") as seen:
        print(version, file=seen)
    library = Path(plan["work"]) / "libnative_lantern.so"
    library.parent.mkdir(parents=True, exist_ok=True)
    result = subprocess.run(["rustc", "--crate-type=cdylib", "--crate-name=native_lantern",
                             str(source / "src/lib.rs"), "-o", str(library)], check=False)
    if result.returncode:
        print(json.dumps({"event": "failed", "error": "Could not compile Native Lantern.",
                          "blame": {"mod": mod_id, "kind": "compile"}}))
        sys.exit(1)
    rust_version = (root / "rust-version").read_text().strip()
    print(json.dumps({"event": "built", "route": "prebuilt-api",
                      "binary": str(Path(plan["package"]["runtime"]) / "bin/sunrise-client"),
                      "mods": [mod_id], "libraries": [str(library)], "internals": {},
                      "records": [{"file": library.name, "kind": "mod", "mod": mod_id,
                                   "sha256": hashlib.sha256(library.read_bytes()).hexdigest(),
                                   "identity": f"{'a' * 40} {mod_id} {version} {rust_version} unwind"}]}))
else:
    raise SystemExit(f"unexpected SDK command: {command}")
