#!/usr/bin/env python3
"""Prepare killstreak resource/layout evidence, without enabling gameplay."""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import struct

from clientlib import REPO, manifest
from schema_fields import flatten

STATS = "mp/coKillstreakStatsTable.csv"
MARKET = "mp/coMarketplaceItems.csv"


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def identity(capture, expected):
    if (capture.get("schema_version") != 1 or capture.get("read_only") is not True
            or capture.get("client_sha256") != expected):
        raise ValueError("expected read-only target-client capture")


def rows(capture, name, columns):
    table = capture.get("tables", {}).get(name, {})
    cells = table.get("cells_base64", [])
    count = table.get("rows")
    if (table.get("found") is not True or table.get("columns") != columns
            or type(count) is not int or not 1 <= count <= 16384
            or len(cells) != count * columns):
        raise ValueError("incomplete or unexpected table: " + name)
    decoded = [base64.b64decode(cell, validate=True).decode("ascii") for cell in cells]
    return [decoded[i:i + columns] for i in range(0, len(decoded), columns)]


def unsigned(text):
    if not text or not text.isascii() or not text.isdecimal():
        raise ValueError("expected unsigned decimal table cell")
    return int(text)


def catalog(capture):
    rewards = []
    ids, names = set(), set()
    for physical_row, row in enumerate(rows(capture, STATS, 33)):
        native_id = unsigned(row[0])
        name = row[1]
        if (not 0 <= native_id <= 255 or native_id in ids or name in names
                or not name or not all(c.isascii() and (c.isalnum() or c == "_") for c in name)):
            raise ValueError("invalid or duplicate native killstreak identity")
        ids.add(native_id)
        names.add(name)
        # Export only needed functional identifiers and values. The complete
        # resource table stays in the ignored source capture.
        cost = unsigned(row[4]) if row[4] else None
        if (native_id == 0 and (name != "none" or cost is not None)
                or native_id != 0 and (cost is None or cost > 31)):
            raise ValueError("unexpected native killstreak cost")
        rewards.append(dict(physical_row=physical_row, native_id=native_id,
                            name=name, cost_c4=cost, column5_raw=row[5] or None,
                            weapon_token_c13=row[13], hud_icon_c15=row[15],
                            hint_icon_c17=row[17]))
    if 0 not in ids:
        raise ValueError("missing native empty reward")
    by_name = {r["name"]: r for r in rewards}
    items = []
    item_ids, item_names = set(), set()
    for row in rows(capture, MARKET, 12):
        if row[2] != "kis":
            continue
        item_id = unsigned(row[0])
        if not 0 < item_id <= 0xffffffff or item_id in item_ids or row[1] in item_names:
            raise ValueError("invalid or duplicate marketplace killstreak identity")
        item_ids.add(item_id)
        item_names.add(row[1])
        reward = by_name.get(row[1])
        items.append(dict(item_id=item_id, name=row[1],
                          native_id=reward["native_id"] if reward else None))
    if not items:
        raise ValueError("missing marketplace killstreak items")
    return rewards, items


def class_layout(fields):
    result = []
    occupied = set()
    for index in range(60):
        offsets = []
        for slot in range(4):
            name = "weaponClasses[%d].killstreaks[%d]" % (index, slot)
            field = fields.get(name)
            if (not field or field["type"] != 0 or field["width"] != 4
                    or field["projected"]):
                raise ValueError("unexpected killstreak profile field: " + name)
            offset = field["offset"]
            occupied_bytes = set(range(offset, offset + 4))
            if offset < 12 or offset + 4 > 32768 or occupied.intersection(occupied_bytes):
                raise ValueError("overlapping/out-of-bounds killstreak profile fields")
            occupied.update(occupied_bytes)
            offsets.append(offset)
        result.append(offsets)
    return result


def native(client_raw, expected):
    if digest(client_raw) != expected:
        raise ValueError("expected unmodified target client")
    import pefile

    pe = pefile.PE(data=client_raw)
    image = pe.get_memory_mapped_image()
    base = pe.OPTIONAL_HEADER.ImageBase
    fixture_raw = (REPO / "fixtures/protocol/player-state-fields-3.3.3.3.json").read_bytes()
    layout = json.loads(fixture_raw)
    at = layout["table_va"] - base
    table = image[at:at + len(layout["fields"]) * 12]
    if (layout["client_sha256"] != expected or digest(table) != layout["table_sha256"]
            or [list(row) for row in struct.iter_unpack("<Iii", table)] != layout["fields"]):
        raise ValueError("playerState netfield table differs")

    def field(offset, bits):
        matches = [(i, r) for i, r in enumerate(layout["fields"]) if r[0] == offset]
        if len(matches) != 1 or matches[0][1][1:] != [bits, 0]:
            raise ValueError("unexpected killstreak netfield")
        return dict(offset=offset, field=matches[0][0], bits=bits, hint=0)

    specs = [
        ("count_getter", 0x519820, 0x37, ["8b4064"]),
        ("next_target_getter", 0x519860, 0xde, ["8b517c", "397164"]),
        ("id_getter", 0x519940, 0x38, ["8b449068"]),
        ("target_getter", 0x519980, 0x38, ["8b44907c"]),
        ("action_state_getter", 0x4327d0, 0x58, ["8b8c9060050000", "3b8c9074050000"]),
        ("action_dispatch", 0x55d510, 0x10d, ["8b848e60050000", "8bbc8e74050000"]),
        ("profile_item_decode", 0x593040, 0x135, ["85c9", "8d41ff"]),
        ("cost_c4_lookup", 0x6c1270, 0x68, ["8b4cc820"]),
    ]
    functions = []
    for name, va, size, signatures in specs:
        block = image[va - base:va - base + size]
        if len(block) != size or any(bytes.fromhex(s) not in block for s in signatures):
            raise ValueError("native consumer differs: " + name)
        functions.append(dict(name=name, va=va, bytes=size, sha256=digest(block)))
    return dict(address_kind="preferred-image VA; playerState-relative offsets",
                field_fixture_sha256=digest(fixture_raw), field_table_sha256=digest(table),
                consumers=functions, count=field(0x64, 8),
                ids=[field(0x68 + i * 4, 8) for i in range(5)],
                targets=[field(0x7c + i * 4, 5) for i in range(5)],
                action_types=[field(0x560 + i * 4, 2) for i in range(5)],
                action_weapons=[field(0x574 + i * 4, 11) for i in range(5)],
                input_bindings=["+actionslot %d" % i for i in range(1, 6)])


def prepare(assets_raw, client_raw, hud_raw=None):
    expected = next(f["sha256"] for f in manifest()["files"]
                    if f["path"] == "CODOL/codoMP_client_shipRetail.exe")
    assets = json.loads(assets_raw)
    identity(assets, expected)
    rewards, items = catalog(assets)
    definition = assets["schemas"]["mp/playerdata.def"]["definitions"][0]
    if definition["version"] != 233:
        raise ValueError("unexpected playerdata definition version")
    fields = flatten(definition)
    result = dict(schema_version=1, client_sha256=expected,
                  evidence_only=True, gameplay_verified=False,
                  capture_sha256=digest(assets_raw), profile_version=233,
                  class_offsets=class_layout(fields), rewards=rewards, marketplace=items,
                  unmatched_marketplace=[i["name"] for i in items if i["native_id"] is None],
                  rewards_without_marketplace=[r["name"] for r in rewards
                                               if r["native_id"] != 0 and r["name"] not in {i["name"] for i in items}],
                  native=native(client_raw, expected))
    if hud_raw is not None:
        hud = json.loads(hud_raw)
        identity(hud, expected)
        names = {"killstreaks_hd2", "killstreak", "killstreak_pickup", "killstreak_hint"}
        menus = [m for m in hud["menus"] if m["name"] in names and m["type"] == 26]
        lookups, strings = set(), set()

        def walk(value, menu):
            if isinstance(value, dict):
                if isinstance(value.get("string"), str):
                    strings.add(value["string"])
                entries = value.get("entries", [])
                # Captured tableLookup(table, search column, key, result column).
                if (len(entries) >= 10 and entries[1].get("type") == 0
                        and entries[1].get("kind") == 80 and entries[2].get("string") == STATS
                        and entries[4].get("type") == 1 and entries[4].get("kind") == 0
                        and entries[-2].get("type") == 1 and entries[-2].get("kind") == 0):
                    lookups.add((menu, entries[4]["value"], entries[-2]["value"]))
                for child in value.values():
                    walk(child, menu)
            elif isinstance(value, list):
                for child in value:
                    walk(child, menu)

        for menu in menus:
            walk(menu, menu["name"])
        result["hud"] = dict(capture_sha256=digest(hud_raw),
                             connection_state=hud["connection_state"],
                             menus=[dict(name=m["name"], expanded=m.get("items") is not None,
                                         item_count=len(m.get("items") or [])) for m in menus],
                             table_lookups=[dict(menu=m, search_column=s, result_column=r)
                                            for m, s, r in sorted(lookups)],
                             streak_queries=sorted(s for s in strings if s.startswith("killstreak")),
                             input_bindings=sorted(s for s in strings if s.startswith("+actionslot ")))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--assets", type=Path, required=True)
    parser.add_argument("--client", type=Path, required=True)
    parser.add_argument("--hud-assets", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = prepare(args.assets.read_bytes(), args.client.read_bytes(),
                     args.hud_assets.read_bytes() if args.hud_assets else None)
    with args.output.open("x", encoding="utf-8") as output:
        json.dump(result, output, indent=2, allow_nan=False)
        output.write("\n")
    print(json.dumps(dict(output=str(args.output), rewards=len(result["rewards"]),
                          marketplace=len(result["marketplace"]),
                          matched=len(result["marketplace"]) - len(result["unmatched_marketplace"]),
                          classes=len(result["class_offsets"]), gameplay_verified=False)))


if __name__ == "__main__":
    main()
