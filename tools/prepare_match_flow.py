#!/usr/bin/env python3
"""Bind existing war preparations to their own captured shared respawn pools.

Outputs a new catalog; source files, accounts and running clients are untouched.
The native server's prepare-map-starts checks the gameplay capsule for every
opening and respawn point and records exclusions before accepting the output.
"""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import subprocess

from prepare_game_state import CLIENT_SHA256, compute
from prepare_tdm_map import respawn_points, preparation_text


def read(path):
    return json.loads(path.read_text())


def write(path, data):
    with path.open("x") as out:
        json.dump(data, out, separators=(",", ":"), allow_nan=False)
        out.write("\n")


def bind(state, entities, constants):
    if state["client_sha256"] != CLIENT_SHA256 or state["mode"] != "war":
        raise ValueError("match flow requires this client's tactical war identity")
    summary = compute(constants)
    if not summary["observed_crc_verified"] or summary["ncs_crc"] != state["ncs_crc"]:
        raise ValueError("preparation HUD constant bank does not match this map CRC")
    state = copy.deepcopy(state)
    state["observed_crc_verified"] = True
    points = respawn_points(entities, state["map"])
    if not points:
        raise ValueError("map has no captured shared mp_tdm_spawn pool")
    state["match_config"].update(respawn_points=points, preparation_msec=5000,
                                 grace_period_msec=15000,
                                 preparation_text=preparation_text(constants))
    return state


def prepare(catalog_path, capture_root, output, server):
    catalog = read(catalog_path)
    if catalog.get("schema_version") != 1:
        raise ValueError("unsupported preparation catalog")
    index = {}
    initialized = {}
    for kind, pattern in (("entities", "map-entities-*.json"), ("constants", "net-constants-*.json")):
        index[kind] = {hashlib.sha256(p.read_bytes()).hexdigest(): p
                       for p in capture_root.glob("client-*/" + pattern)}
    for path in index["constants"].values():
        capture = read(path)
        if not capture.get("initialized"):
            continue
        try:
            summary = compute(capture)
        except (ValueError, KeyError):
            continue
        initialized.setdefault(summary["ncs_crc"], path)
    output.mkdir(parents=True, exist_ok=False)
    entries, evidence = [], []
    for entry in catalog["entries"]:
        source = (catalog_path.parent / entry["file"]).resolve()
        original = read(source)
        if (original["map"], original["mode"]) != (entry["map"], entry["mode"]):
            raise ValueError("catalog and prepared identity differ")
        entities = index["entities"][original["map_entities_sha256"]]
        constants = index["constants"][original["capture_sha256"]]
        if not read(constants).get("initialized"):
            constants = initialized[original["ncs_crc"]]
        state = bind(original, read(entities), read(constants))
        state["capture_sha256"] = hashlib.sha256(constants.read_bytes()).hexdigest()
        for key, value in list(state.items()):
            if key.endswith("_file") and isinstance(value, str):
                state[key] = str((source.parent / value).resolve(strict=True))
        name = state["map"]
        candidate = output / (name + "-candidate.json")
        ready = output / (name + ".json")
        report = output / (name + "-spawns.json")
        write(candidate, state)
        with (output / (name + "-validation.log")).open("x") as log:
            subprocess.run([str(server), "prepare-map-starts", "--game-state", str(candidate),
                            "--output", str(ready), "--report", str(report)],
                           stdout=log, stderr=subprocess.STDOUT, check=True)
        final = read(ready)
        entries.append(dict(map=name, mode=state["mode"], file=str(ready.resolve())))
        evidence.append(dict(map=name, source=str(source), source_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),
                             entities=str(entities), constants=str(constants),
                             respawn_count=len(final["match_config"]["respawn_points"]),
                             preparation_msec=final["match_config"]["preparation_msec"],
                             preparation_text=final["match_config"]["preparation_text"],
                             report=str(report.resolve())))
    result = copy.deepcopy(catalog)
    result["entries"] = entries
    write(output / "catalog.json", result)
    write(output / "evidence.json", dict(schema_version=1, native_client_used=False, maps=evidence))
    return evidence


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for arg in ("catalog", "capture-root", "output", "server"):
        parser.add_argument("--" + arg, required=True, type=Path)
    args = parser.parse_args()
    maps = prepare(args.catalog.resolve(), args.capture_root.resolve(), args.output.resolve(), args.server.resolve())
    print(json.dumps(dict(maps=len(maps), respawn_points=sum(m["respawn_count"] for m in maps))))


if __name__ == "__main__":
    main()
