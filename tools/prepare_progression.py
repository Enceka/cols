#!/usr/bin/env python3
"""Prepare numeric progression tables and an evidence catalog from a read-only
3.3.3.3 asset capture. Never read or write player accounts or gameplay settings.
"""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import re
import struct

from clientlib import manifest
from schema_fields import flatten

CLIENT_SHA256 = next(f["sha256"] for f in manifest()["files"]
                     if f["path"] == "CODOL/codoMP_client_shipRetail.exe")
MAX_XP = 2**31 - 1


def rows(capture, name, columns):
    table = capture.get("tables", {}).get(name, {})
    count, cells = table.get("rows"), table.get("cells_base64", [])
    if (table.get("found") is not True or table.get("columns") != columns
            or type(count) is not int or not 1 <= count <= 30000
            or len(cells) != count * columns):
        raise ValueError("incomplete target table: " + name)
    decoded = [base64.b64decode(c, validate=True).decode("ascii") for c in cells]
    return [decoded[i:i + columns] for i in range(0, len(decoded), columns)]


def number(value):
    if not re.fullmatch(r"0|[1-9][0-9]*", value):
        raise ValueError("invalid unsigned native integer: " + value)
    result = int(value)
    if result > MAX_XP:
        raise ValueError("integer exceeds native signed range")
    return result


def curve(table, count):
    if len(table) != count:
        raise ValueError("unexpected target curve length")
    thresholds = []
    for index, row in enumerate(table):
        if number(row[0]) != index:
            raise ValueError("missing, duplicate or reordered rank index")
        thresholds.append(number(row[1]))
    if thresholds[0] != 0 or any(a >= b for a, b in zip(thresholds, thresholds[1:])):
        raise ValueError("non-monotonic cumulative XP curve")
    return thresholds


def prepare(capture):
    if (capture.get("schema_version") != 1 or capture.get("read_only") is not True
            or capture.get("client_sha256") != CLIENT_SHA256):
        raise ValueError("expected read-only target 3.3.3.3 capture")
    rank = rows(capture, "mp/rankTable.csv", 18)
    if (len(rank) != 72 or rank[0][:4] != ["maxrank", "69", "version", "4"]
            or rank[1][:2] != ["maxprestige", "3"]):
        raise ValueError("unexpected target player rank metadata")
    player = curve([[r[0], r[2]] for r in rank[2:]], 70)
    for index, row in enumerate(rank[2:]):
        end = number(row[7])
        if (number(row[13]) != index + 1 or number(row[3]) < 1
                or end != player[index] + number(row[3])
                or (index < 69 and end != player[index + 1])):
            raise ValueError("inconsistent player rank span/display level")
    curves = dict(schema_version=1, client_sha256=CLIENT_SHA256, rank_version=4,
                  player=player,
                  primary=curve(rows(capture, "mp/coWeaponPrimaryRankTable.csv", 2), 31),
                  secondary=curve(rows(capture, "mp/coWeaponSecondaryRankTable.csv", 2), 20))

    definitions = capture.get("schemas", {}).get("mp/playersecuredata.def", {}).get("definitions", [])
    if len(definitions) != 1 or definitions[0].get("version") != 19:
        raise ValueError("expected target secure profile v19")
    fields = flatten(definitions[0])
    expected = {"experience": (0, 4844, True), "rank": (0, 4887, True),
                "rankVersion": (0, 4891, True), "prestige": (0, 4883, True),
                "unlockFlagsNew[1]": (9, 8297, False)}
    selected = {}
    for name, (kind, offset, projected) in expected.items():
        field = fields.get(name, {})
        if (field.get("type"), field.get("offset"), field.get("projected")) != (kind, offset, projected):
            raise ValueError("unexpected target progression field: " + name)
        selected[name] = field
    prestige_flags = [r for r in rows(capture, "mp/unlockFlags.csv", 3)
                      if r[0] == "uf_prestige_rank"]
    if len(prestige_flags) != 1 or prestige_flags[0][1] != "1":
        raise ValueError("missing or ambiguous target rank unlock flag")

    weapons, ids = [], set()
    for row in rows(capture, "mp/coMarketplaceItems.csv", 12):
        item = number(row[0])
        if item == 0 or item > 65535 or item in ids:
            raise ValueError("invalid or duplicate marketplace item id")
        ids.add(item)
        if row[2] == "wea":
            if not re.fullmatch(r"[A-Za-z0-9_]{1,100}", row[1]):
                raise ValueError("invalid native weapon identity")
            weapons.append(dict(item_id=item, name=row[1], subclass=row[3]))
    if not weapons:
        raise ValueError("no target marketplace weapon identities")

    observations = {}
    for name in ("mp/splashTable.csv", "mp/rankIconTable.csv"):
        table = capture.get("tables", {}).get(name, {})
        # Presence alone does not verify promotion or weapon splash consumption.
        observations[name] = dict(found=table.get("found") is True,
                                  rows=table.get("rows"), columns=table.get("columns"),
                                  loaded=table.get("loaded"))
    catalog = dict(schema_version=1, client_sha256=CLIENT_SHA256, rank_version=4,
                   secure_profile_version=19, profile_fields=selected,
                   weapon_items=weapons, notification_table_observations=observations,
                   player_experience_unit="within_rank",
                   weapon_experience_unit="cumulative_inventory_xp",
                   runtime_enabled=False, native_notifications_verified=False,
                   attachment_requirements_verified=False,
                   required_evidence=["native_item_primary_secondary_classification",
                                      "per_weapon_attachment_and_slot_requirements",
                                      "promotion_and_weapon_splash_consumers",
                                      "native_summary_player_and_weapon_progress",
                                      "authoritative_award_rates_and_weapon_attribution",
                                      "atomic_match_commit_retry_and_profile_upload_conflicts",
                                      "relogin_and_duplicate_notification_behavior"])
    return curves, catalog


def runtime_rules(capture, catalog):
    """Use the native item-record class byte consumed by 0x6b7190.

    No attachment threshold is inferred from the display metadata strings.
    """
    exported = capture.get("progression_items", {})
    items = exported.get("items", [])
    if exported.get("found") is not True or not 1 <= len(items) <= 30000:
        raise ValueError("native progression item records missing")
    by_id = {}
    for item in items:
        item_id = item.get("item_id")
        if type(item_id) is not int or not 1 <= item_id <= 65535 or item_id in by_id:
            raise ValueError("invalid or duplicate native progression item")
        by_id[item_id] = item
    weapons = []
    for weapon in catalog["weapon_items"]:
        record = by_id.get(weapon["item_id"], {})
        kind, classification = record.get("kind"), record.get("weapon_class")
        if (record.get("name") != weapon["name"] or type(kind) is not int or kind != 0
                or type(classification) is not int or not 0 <= classification <= 15):
            raise ValueError("weapon classification identity/layout mismatch")
        weapon.update(native_weapon_class=classification, is_primary=classification < 6)
        weapons.append(dict(item_id=weapon["item_id"], is_primary=classification < 6,
                            attachments=[]))
    catalog["native_weapon_classification_verified"] = True
    products = {}
    for product in capture.get("runtime_marketplace", {}).get("products", []):
        header = base64.b64decode(product["header_base64"], validate=True)
        if len(header) != 24:
            raise ValueError("unexpected native product header size")
        product_id, level = struct.unpack_from("<H", header)[0], struct.unpack_from("<H", header, 4)[0]
        if product_id and product_id in products:
            raise ValueError("duplicate native product id")
        if product_id:
            products[product_id] = level
    if products:
        observations = []
        for weapon in catalog["weapon_items"]:
            compatible = by_id[weapon["item_id"]].get("compatible_items", [])
            if len(compatible) > 1000:
                raise ValueError("weapon compatibility list exceeds native limit")
            for item_id in compatible:
                item = by_id.get(item_id)
                if item is None:
                    # Some default parts belong to another resource category.
                    continue
                product_ids = item.get("product_ids", [])
                if len(product_ids) > 128 or any(p not in products for p in product_ids):
                    raise ValueError("attachment product link missing")
                levels = sorted({products[p] + 1 for p in product_ids if products[p] > 0})
                if levels:
                    observations.append(dict(weapon_item_id=weapon["item_id"], item_id=item_id,
                                             name=item["name"], displayed_levels=levels,
                                             product_ids=product_ids))
        catalog["attachment_level_observations"] = observations
        # Active SKU/time/other eligibility conditions and the native slot
        # selection still need verification; observations do not enable reminders.
    return dict(schema_version=1, client_sha256=CLIENT_SHA256, source_sha256="",
                weapons=weapons)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--assets", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--with-runtime-rules", action="store_true",
                        help="require native item records and emit weapon classification rules; attachment gates remain unverified")
    args = parser.parse_args()
    raw = args.assets.read_bytes()
    capture = json.loads(raw)
    curves, catalog = prepare(capture)
    rules = runtime_rules(capture, catalog) if args.with_runtime_rules else None
    if rules is not None:
        rules["source_sha256"] = hashlib.sha256(raw).hexdigest()
    catalog["sources"] = {str(args.assets.resolve()): hashlib.sha256(raw).hexdigest()}
    args.output.mkdir(parents=True, exist_ok=False)
    for name, content in (("curves.json", curves), ("catalog.json", catalog)):
        (args.output / name).write_text(json.dumps(content, indent=2) + "\n")
    if rules is not None:
        (args.output / "progression-rules.json").write_text(json.dumps(rules, indent=2) + "\n")
    print(json.dumps(dict(output=str(args.output), player_max_level=len(curves["player"]),
                         primary_max_level=len(curves["primary"]),
                         secondary_max_level=len(curves["secondary"]),
                         weapon_items=len(catalog["weapon_items"]), runtime_enabled=False)))


if __name__ == "__main__":
    main()
