#!/usr/bin/env python3
"""Build/run a read-only Win32 state probe inside the project's GameHub prefix.

No DebugActiveProcess, memory writes or injection. Output is a small allowlist
of known client state fields; no account/session keys are read.
"""
import argparse
import fcntl
import json
import os
import re
from pathlib import Path
import subprocess
import time

from clientlib import REPO, sha256
from gamehub_run import engines


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--build", action="store_true")
    mode.add_argument("--experiment", type=Path, help="a running experiment.json")
    parser.add_argument("--prefix-name", help="project-owned build prefix; experiment selects its own prefix")
    extract = parser.add_mutually_exclusive_group()
    extract.add_argument("--assets", action="store_true", help="extract the fixed profile-schema and data-table allowlist")
    extract.add_argument("--profiles", action="store_true", help="save this client's two fixed profile buffers to the ignored experiment directory")
    extract.add_argument("--net-constants", action="store_true", help="read loaded type-44 net-constant assets and CRC state")
    args = parser.parse_args()
    if args.prefix_name and (not args.build or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_-]{0,63}", args.prefix_name)):
        parser.error("prefix-name requires build and a simple project directory name")
    if args.build and (args.assets or args.profiles or args.net_constants):
        parser.error("extraction options require a running experiment")
    state = json.loads(args.experiment.read_text()) if args.experiment else None
    engine_id = state["engine_id"] if state else "10000033"
    engine = Path(engines()[engine_id]["install_path"])
    prefix = Path(state["prefix"]) if state else REPO / "client-work" / (args.prefix_name or "wine10-prefix")
    if not prefix.resolve().is_relative_to((REPO / "client-work").resolve()):
        parser.error("prefix must belong to this project")
    marker = prefix / ".codol-prefix.json"
    if not marker.is_file() or json.loads(marker.read_text())["engine_id"] != engine_id:
        parser.error("prefix ownership marker mismatch")
    sources = [REPO / "tools/ReadClientState.cs", REPO / "tools/ClientAssets.cs"]
    source_hashes = {p.name: sha256(p) for p in sources}
    binary = REPO / ".tools/ReadClientState.exe"
    stamp = binary.with_suffix(".build.json")
    env = os.environ.copy()
    env.update(WINEPREFIX=str(prefix.resolve()), WINEDEBUG="-all")
    wine = str(engine / "bin/wine")
    if args.build:
        binary.parent.mkdir(exist_ok=True)
        with (prefix / ".codol-lock").open("a") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            try:
                built = subprocess.run([
                    wine, r"C:\windows\Microsoft.NET\Framework\v4.0.30319\csc.exe",
                    "/nologo", "/platform:anycpu", "/r:System.Web.Extensions.dll", "/out:Z:" + str(binary), *["Z:" + str(p) for p in sources],
                ], env=env, capture_output=True, text=True, timeout=30)
                if built.returncode:
                    raise RuntimeError((built.stdout + built.stderr)[-2000:])
                stamp.write_text(json.dumps({"sources_sha256": source_hashes, "binary_sha256": sha256(binary)}) + "\n")
                print(json.dumps({"built": str(binary), "read_only": True}))
            finally:
                # A standalone compiler can let Wine exit before cleanup.
                # -k then returns 1 with no output; -w confirms it is stopped.
                stopped = subprocess.run([str(engine / "bin/wineserver"), "-k"], env=env,
                                         capture_output=True, text=True, timeout=15)
                if stopped.returncode not in (0, 1) or (stopped.returncode == 1 and stopped.stderr):
                    stopped.check_returncode()
                subprocess.run([str(engine / "bin/wineserver"), "-w"], env=env,
                               capture_output=True, timeout=15, check=True)
    else:
        if "finished_unix" in state:
            parser.error("experiment has already ended")
        actual = subprocess.check_output(["/bin/ps", "-p", str(state["pid"]), "-o", "comm="], text=True).strip()
        if actual != state["argv"][1]:
            parser.error("recorded PID is no longer the research client")
        if not stamp.is_file():
            parser.error("build the probe first with --build")
        hashes = json.loads(stamp.read_text())
        if hashes != {"sources_sha256": source_hashes, "binary_sha256": sha256(binary)}:
            parser.error("probe changed; rebuild it")
        result = subprocess.run([wine, str(binary)] + (["--assets"] if args.assets else ["--profiles"] if args.profiles else ["--net-constants"] if args.net_constants else []), env=env, capture_output=True, text=True, timeout=30)
        if result.returncode:
            raise RuntimeError((result.stdout + result.stderr)[-2000:])
        decoded = json.loads(result.stdout)
        output = args.experiment.parent / (("client-assets-" if args.assets else "client-profiles-" if args.profiles else "net-constants-" if args.net_constants else "client-state-") + str(int(time.time() * 1000)) + ".json")
        output.write_text(json.dumps(decoded, indent=2) + "\n")
        summary = {"schemas": {k: {f: v[f] for f in ("found", "error") if f in v} for k, v in decoded["schemas"].items()},
                   "tables": {k: {f: v[f] for f in ("found", "rows", "columns", "error") if f in v} for k, v in decoded["tables"].items()}} if args.assets else {"read_only": True, "profile_buffers": 2} if args.profiles else decoded
        if args.net_constants:
            summary = {"read_only": True, "connection_state": decoded["connection_state"], "initialized": decoded["initialized"], "checksum": decoded["checksum"], "assets": len(decoded["assets"]), "linked": len(decoded["linked"]), "asset_errors": sum("error" in asset for asset in decoded["assets"])}
        print(json.dumps({"output": str(output), "state": summary}))


if __name__ == "__main__":
    main()
