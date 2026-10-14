#!/usr/bin/env python3
"""Collect loaded native cargo assets with a pinned, read-only Win32 probe.

Does not launch the client, change focus, execute GSC or publish captures.
Run while the native story loader is waiting for the game world.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

from clientlib import matches_client_image
from gamehub_run import engines
from prepare_story import CLIENT_SHA256, FIRST_MAP


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--experiment", type=Path, required=True)
    parser.add_argument("--probe", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--wait-seconds", type=int, default=45, choices=range(1,91))
    args = parser.parse_args()
    state = json.loads(args.experiment.read_bytes())
    if state.get("finished_unix") or state.get("story_capture_map") != FIRST_MAP:
        parser.error("running cargo story experiment required")
    prefix = Path(state["prefix"])
    executable = Path(state["argv"][1])
    actual = subprocess.check_output(["/bin/ps", "-p", str(state["pid"]), "-o", "comm="], text=True).strip()
    if (not matches_client_image(actual, executable, prefix)
            or not (executable.parent/".codol-research-copy.json").is_file()):
        parser.error("experiment does not identify the marked native client")
    probe_hash = hashlib.sha256(args.probe.read_bytes()).hexdigest()
    stamp = json.loads(args.probe.with_suffix(".build.json").read_bytes())
    if stamp["binary_sha256"] != probe_hash:
        parser.error("pinned probe identity mismatch")
    engine = Path(engines()[state["engine_id"]]["install_path"])
    env = dict(os.environ, WINEPREFIX=str(prefix), WINEDEBUG="-all")
    args.output.mkdir(parents=True,exist_ok=False)
    report = dict(schema_version=1, client_sha256=CLIENT_SHA256, map=FIRST_MAP,
                  read_only=True, gameplay_verified=False, native_map_loaded=False,
                  experiment=str(args.experiment.resolve()), probe_sha256=probe_hash,
                  captures=[], errors=[])

    def capture(mode):
        command=[str(engine/"bin/wine"),str(args.probe.resolve()),"--"+mode]
        completed=subprocess.run(command,env=env,capture_output=True,text=True,timeout=30)
        if completed.returncode:
            raise RuntimeError(completed.stderr[-1000:])
        value=json.loads(completed.stdout)
        if value.get("read_only") is not True or value.get("client_sha256") != CLIENT_SHA256:
            raise ValueError("resource capture identity mismatch")
        path=args.output/(mode+"-"+str(time.time_ns())+".json")
        raw=(json.dumps(value,indent=2)+"\n").encode()
        path.write_bytes(raw)
        report["captures"].append(dict(mode=mode,path=path.name,sha256=hashlib.sha256(raw).hexdigest(),
                                       connection_state=value.get("connection_state")))
        return value

    deadline=time.monotonic()+args.wait_seconds
    while time.monotonic()<deadline:
        try:
            bundle=capture("story-bundle")
            entities=bundle["captures"].get("map-entities",{})
            # The measured cargo story start loads this shared native BSP.
            # Keep story identity separate from its base-map asset name.
            if any(m.get("name")=="maps/mp/mp_cargoship.d3dbsp" for m in entities.get("maps",[])):
                report["native_map_loaded"]=True
                report["base_map_asset"]="maps/mp/mp_cargoship.d3dbsp"
                report["final_connection_state"]=bundle["final_connection_state"]
                for mode,value in bundle["captures"].items():
                    if "error" in value:
                        report["errors"].append(dict(mode=mode,error=value["error"]))
                    else:
                        path=args.output/(mode+".json");raw=(json.dumps(value,indent=2)+"\n").encode();path.write_bytes(raw)
                        report["captures"].append(dict(mode=mode,path=path.name,sha256=hashlib.sha256(raw).hexdigest(),connection_state=value.get("connection_state")))
                constants=bundle["captures"].get("net-constants",{})
                if "checksum" in constants:
                    report["constants"]=dict(initialized=constants["initialized"],checksum=constants["checksum"],
                                             assets=len(constants["assets"]),errors=sum("error" in a for a in constants["assets"]))
                scripts=bundle["captures"].get("story-assets",{})
                if "rawfiles" in scripts:
                    report["scripts"]=dict(rawfiles=len(scripts["rawfiles"]),asset_names=len(scripts["asset_names"]))
                break
        except (ValueError,RuntimeError,subprocess.TimeoutExpired) as error:
            report["errors"].append(dict(mode="story-bundle",error=str(error)))
        time.sleep(.5)
    (args.output/"result.json").write_text(json.dumps(report,indent=2)+"\n")
    print(json.dumps(dict(output=str(args.output),native_map_loaded=report["native_map_loaded"],
                         captures=len(report["captures"]),errors=len(report["errors"]))))


if __name__ == "__main__":
    main()
