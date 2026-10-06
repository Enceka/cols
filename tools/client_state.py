#!/usr/bin/env python3
"""Build/run a read-only Win32 state probe inside the project's GameHub prefix.

No DebugActiveProcess, memory writes or injection. Output is a small allowlist
of known client state fields; no account/session keys are read.
"""
import argparse
import fcntl
import json
import os
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
    args = parser.parse_args()
    state = json.loads(args.experiment.read_text()) if args.experiment else None
    engine_id = state["engine_id"] if state else "10000033"
    engine = Path(engines()[engine_id]["install_path"])
    prefix = Path(state["prefix"]) if state else REPO / "client-work/wine10-prefix"
    if not prefix.resolve().is_relative_to((REPO / "client-work").resolve()):
        parser.error("prefix must belong to this project")
    marker = prefix / ".codol-prefix.json"
    if not marker.is_file() or json.loads(marker.read_text())["engine_id"] != engine_id:
        parser.error("prefix ownership marker mismatch")
    source = REPO / "tools/ReadClientState.cs"
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
                    "/nologo", "/platform:anycpu", "/out:Z:" + str(binary), "Z:" + str(source),
                ], env=env, capture_output=True, text=True, timeout=30)
                if built.returncode:
                    raise RuntimeError((built.stdout + built.stderr)[-2000:])
                stamp.write_text(json.dumps({"source_sha256": sha256(source), "binary_sha256": sha256(binary)}) + "\n")
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
        if hashes != {"source_sha256": sha256(source), "binary_sha256": sha256(binary)}:
            parser.error("probe changed; rebuild it")
        result = subprocess.run([wine, str(binary)], env=env, capture_output=True, text=True, timeout=20)
        if result.returncode:
            raise RuntimeError((result.stdout + result.stderr)[-2000:])
        decoded = json.loads(result.stdout)
        output = args.experiment.parent / ("client-state-" + str(int(time.time() * 1000)) + ".json")
        output.write_text(json.dumps(decoded, indent=2) + "\n")
        print(json.dumps({"output": str(output), "state": decoded}))


if __name__ == "__main__":
    main()
