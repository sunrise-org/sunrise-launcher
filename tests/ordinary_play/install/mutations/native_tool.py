#!/usr/bin/env python3
"""Local developer SDK fixture: compile the staged source and return the exact bytes."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tomllib

root = Path(os.environ["FIXTURE_NATIVE_ROOT"])
plan = json.loads(sys.stdin.readline())
command = sys.argv[1]
if command == "native-layout":
    print(json.dumps({"package": {part: str(root / "sdk" / part)
                                  for part in ("runtime", "api", "engine")}}))
elif command == "native-check":
    mods = [tomllib.loads(path.read_text()) for path in
            Path(plan["mods"]).glob("*/sunrise-mod.toml")]
    assert all(mod["native"]["sunrise"] == plan["build"] for mod in mods)
    (root / "checked-mods.json").write_text(json.dumps([mod["id"] for mod in mods]))
    print(json.dumps({"contract": 2, "route": "prebuilt-api",
                      "mods": [{"id": mod["id"], "verdict": {"kind": "made-for"}}
                               for mod in mods]}))
elif command == "native-build":
    source, = Path(plan["mods"]).iterdir()
    manifest = tomllib.loads((source / "sunrise-mod.toml").read_text())
    mod_id, version = manifest["id"], manifest["version"]
    with (root / "compiled").open("a") as seen:
        print(version, file=seen)
    library = Path(plan["work"]) / f"lib{mod_id}.so"
    library.parent.mkdir(parents=True, exist_ok=True)
    cfg = ["--cfg", "fixture_switch_target"] if (root / "fixture-switch-target").exists() else []
    result = subprocess.run(["rustc", "--crate-type=cdylib", f"--crate-name={mod_id}", *cfg,
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
                                   "identity": f"{manifest['native']['sunrise']} {mod_id} {version} {rust_version} unwind"}]}))
else:
    raise SystemExit(f"unexpected SDK command: {command}")
