#!/usr/bin/env python3
"""Prepare story identities and resource requirements from read-only captures.

No gameplay configuration, game resources, account state or keys are written.
Outputs belong in an ignored research directory, not the TDM runtime catalog.
"""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import re

from clientlib import REPO, manifest
from schema_fields import flatten

FIRST_MAP = "coop_training_mp_cargoship"
CLIENT_SHA256 = next(f["sha256"] for f in manifest()["files"]
                     if f["path"] == "CODOL/codoMP_client_shipRetail.exe")


def identity(capture):
    if (capture.get("schema_version") != 1 or capture.get("read_only") is not True
            or capture.get("client_sha256") != CLIENT_SHA256):
        raise ValueError("expected read-only 3.3.3.3 capture")


def rows(capture, name, columns):
    table = capture.get("tables", {}).get(name, {})
    cells = table.get("cells_base64", [])
    count = table.get("rows")
    if (table.get("found") is not True or table.get("columns") != columns
            or type(count) is not int or not 1 <= count <= 512
            or len(cells) != count * columns):
        raise ValueError("incomplete native table: " + name)
    decoded = [base64.b64decode(c, validate=True).decode("ascii") for c in cells]
    return [decoded[i:i + columns] for i in range(0, len(decoded), columns)]


def prepare(frontend):
    identity(frontend)
    if frontend.get("build_version_column") != 2:
        raise ValueError("expected measured native build version column 2")
    result, names, ids = [], set(), set()
    for row in rows(frontend, "mp/coMapList.csv", 37):
        if row[8] != "coop" or int(row[12]) < 1:
            continue
        name, native_id = row[4], int(row[3])
        if (not re.fullmatch(r"coop_[a-z0-9_]{1,60}", name) or not 1 <= native_id <= 512
                or name in names or native_id in ids or row[9] != "coop"
                or row[10:12] != ["1", "1"] or any(v not in ("0", "1") for v in row[:3])):
            raise ValueError("invalid or ambiguous single-player story identity")
        chapter, level, sequence = int(row[12]), int(row[13]), int(row[33])
        if not 1 <= chapter <= 32 or not 1 <= level <= 32 or not 1 <= sequence <= 512:
            raise ValueError("invalid native story order")
        names.add(name)
        ids.add(native_id)
        result.append(dict(name=name, native_id=native_id, mode="coop",
                           native_flags=list(map(int, row[:3])), enabled_by_version=row[2] == "1",
                           minimum_players=1, maximum_players=1, chapter=chapter,
                           level=level, sequence=sequence, unlock_requires=row[22] or None,
                           completion_flag="cf_ch_" + name))
    first = next((m for m in result if m["name"] == FIRST_MAP), None)
    if not first or first["native_id"] != 17 or not first["enabled_by_version"]:
        raise ValueError("first native story map is absent or disabled")
    return dict(schema_version=1, client_sha256=CLIENT_SHA256, build_version_column=2,
                resource_preparation_claimed=False, gameplay_verified=False, maps=result)


def progress(assets, catalog):
    identity(assets)
    definition = assets["schemas"]["mp/playersecuredata.def"]["definitions"][0]
    if definition["version"] != 19:
        raise ValueError("unexpected native private profile version")
    fields = flatten(definition)
    flags = {}
    wanted = {item["completion_flag"] for item in catalog["maps"]}
    for row in rows(assets, "mp/unlockFlags.csv", 3):
        if row[0] not in wanted:
            continue
        if row[0] in flags:
            raise ValueError("ambiguous native completion flag")
        flags[row[0]] = int(row[1])
    for item in catalog["maps"]:
        flag = item["completion_flag"]
        if flag not in flags or not 0 <= flags[flag] < 2048:
            raise ValueError("missing native story completion flag")
        field = fields["unlockFlags[%d]" % flags[flag]]
        if field["type"] != 2 or field["projected"]:
            raise ValueError("unexpected native completion field")
        item.update(completion_flag_id=flags[flag], completion_field=field)


def resource_observations(lists, catalog):
    if (lists.get("schema_version") != 1 or lists.get("read_only") is not True
            or lists.get("version") != "3.3.3.3"
            or lists.get("inventory_sha256") != hashlib.sha256(
                (REPO / "fixtures/resources-3.3.3.3.json").read_bytes()).hexdigest()):
        raise ValueError("expected current read-only IFS inventory observations")
    for item in catalog["maps"]:
        item["observed_fastfiles"] = {}
        for suffix in ("", "_load", "_const"):
            path = "client_shipRetail/chinese/" + item["name"] + suffix + ".ff"
            item["observed_fastfiles"][path] = [p["package"] for p in lists["packages"]
                if any(x["path"] == path for x in p.get("paths", []))]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--frontend", type=Path, required=True)
    parser.add_argument("--assets", type=Path, required=True)
    parser.add_argument("--resource-lists", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    sources = {}
    captures = []
    for path in (args.frontend, args.assets, args.resource_lists):
        raw = path.read_bytes()
        sources[str(path.resolve())] = hashlib.sha256(raw).hexdigest()
        captures.append(json.loads(raw))
    catalog = prepare(captures[0])
    progress(captures[1], catalog)
    resource_observations(captures[2], catalog)
    catalog["sources"] = sources
    first = next(m for m in catalog["maps"] if m["name"] == FIRST_MAP)
    queue = dict(schema_version=1, client_sha256=CLIENT_SHA256, map=FIRST_MAP, mode="coop",
                 minimum_players=1, maximum_players=1, gameplay_verified=False,
                 resource_preparation_claimed=False, observed_fastfiles=first["observed_fastfiles"],
                 required_evidence=["native_allocation_and_server_key_handshake", "native_mmstart",
                                    "initialized_constants_and_crc", "map_entities_and_world_collision",
                                    "native_story_start_origin", "story_script_and_trigger_dependencies",
                                    "npc_models_animations_and_behaviors", "objectives_failure_and_retry",
                                    "authoritative_completion_and_unlock_persistence"])
    args.output.mkdir(parents=True, exist_ok=False)
    for name, value in (("catalog.json", catalog), ("first-map.json", queue)):
        (args.output / name).write_text(json.dumps(value, indent=2) + "\n")
    print(json.dumps(dict(output=str(args.output), story_maps=len(catalog["maps"]),
                         enabled=sum(m["enabled_by_version"] for m in catalog["maps"]),
                         gameplay_verified=False)))


if __name__ == "__main__":
    main()
