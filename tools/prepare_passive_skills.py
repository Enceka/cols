#!/usr/bin/env python3
"""Derive passive skill identities and all class offsets from a read-only capture."""
import argparse
import base64
import hashlib
import json
from pathlib import Path

from clientlib import manifest
from schema_fields import flatten


def prepare(path):
    raw = path.read_bytes()
    assets = json.loads(raw)
    client = next(f["sha256"] for f in manifest()["files"] if f["path"] == "CODOL/codoMP_client_shipRetail.exe")
    if assets.get("client_sha256") != client or assets.get("read_only") is not True:
        raise ValueError("expected unmodified target capture")

    def rows(name, columns):
        table = assets["tables"][name]
        cells = table["cells_base64"]
        if not table.get("found") or table["columns"] != columns or len(cells) != table["rows"] * columns:
            raise ValueError("incomplete table: " + name)
        return [[base64.b64decode(c, validate=True).decode("ascii") for c in cells[i:i + columns]]
                for i in range(0, len(cells), columns)]

    perk_rows = rows("mp/coPerkStatsTable.csv", 24)
    ability_rows = rows("mp/coAbilityModifiersTable.csv", 7)
    market = rows("mp/coMarketplaceItems.csv", 12)
    fields = flatten(assets["schemas"]["mp/playerdata.def"]["definitions"][0])
    layout = []
    for index in range(60):
        prefix = "weaponClasses[%d]." % index
        names = ["perks[%d]" % i for i in range(6)] + ["lethal", "tactical"]
        leaves = [fields[prefix + name] for name in names]
        if any(f["width"] != 4 or f["type"] != 0 or f["projected"] for f in leaves):
            raise ValueError("unexpected class item representation")
        layout.append([f["offset"] for f in leaves])
    by_name = {r[1]: r for r in perk_rows}
    skills = []
    for row in market:
        if row[2] != "per":
            continue
        native = by_name[row[1]]
        if native[5] not in ("perk1", "perk2", "perk3") or row[3] != "pk" + native[5][-1]:
            raise ValueError("perk color does not match marketplace")
        skills.append({"item_id": int(row[0]), "native_id": int(native[0]), "slot": int(native[5][-1]) - 1,
                       "name": row[1], "modifiers": [[native[c], native[c + 1]] for c in range(15, 23, 2) if native[c]],
                       "flag": native[23]})
    return {"schema_version": 1, "client_sha256": client, "capture_sha256": hashlib.sha256(raw).hexdigest(),
            "class_offsets": layout, "skills": skills, "abilities": ability_rows}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--assets", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = prepare(args.assets)
    with args.output.open("x") as output:
        json.dump(result, output, indent=2)
        output.write("\n")
    print(json.dumps({"output": str(args.output), "skills": len(result["skills"]), "classes": len(result["class_offsets"])}))


if __name__ == "__main__":
    main()
