#!/usr/bin/env python3
"""Prepare or apply complete local ownership from this build's item table.

Default is a read-only plan. --apply is offline administration: stop the local
services and client first. Original account files are backed up before either
inventory or an explicitly required private unlock flag is changed. Public
profile, wallet balances, purchases and unrelated private progress are retained.

Ownership is not a shop purchase. No SKU, price or wallet record is fabricated.
Private flags are never blanket-maximized. An optional read-only target capture
of item requirements can raise only the named unlockFlagsNew minimums.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import struct
import tempfile
import zlib

from clientlib import manifest
from prepare_inventory import public_snapshot
from schema_fields import flatten, value

MAX_ITEMS = 8192  # codol_protocol::task::MAX_INVENTORY_ITEMS
MAX_INVENTORY_BYTES = 256 * 1024
PRIVATE_SCHEMA = (19, 2318499177)
PUBLIC_SCHEMA = (233, 3811397846)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def encoded_json(data):
    return (json.dumps(data, indent=2, ensure_ascii=True) + "\n").encode()


def table_rows(table, columns):
    if not table.get("found") or table.get("columns") != columns:
        raise ValueError("required target table is missing or has a different layout")
    rows = table.get("rows")
    cells = table.get("cells_base64", [])
    if type(rows) is not int or not 0 < rows <= 30000 or len(cells) != rows * columns:
        raise ValueError("incomplete target table")
    decoded = [base64.b64decode(c, validate=True).decode("ascii") for c in cells]
    if any(len(c) > 4096 for c in decoded):
        raise ValueError("target table cell too large")
    return [decoded[i:i + columns] for i in range(0, len(decoded), columns)]


def catalogue(assets, client_hash):
    if assets.get("schema_version") != 1 or assets.get("read_only") is not True or assets.get("client_sha256") != client_hash:
        raise ValueError("read-only extraction of the expected client is required")
    items = {}
    for row in table_rows(assets["tables"]["mp/coMarketplaceItems.csv"], 12):
        item = int(row[0])
        if not 0 < item <= 65535 or item in items or not row[1] or not row[2]:
            raise ValueError("invalid or duplicate marketplace item identity")
        items[item] = {"name": row[1], "category": row[2], "group": row[3]}
    if len(items) > MAX_ITEMS:
        raise ValueError("catalogue exceeds verified inventory transport budget")
    return items


def schema(assets, name, expected):
    source = assets["schemas"][name]
    if not source.get("found") or len(source.get("definitions", [])) != 1:
        raise ValueError("one intact target profile schema required")
    definition = source["definitions"][0]
    if (definition["version"], definition["formatChecksum"]) != expected:
        raise ValueError("profile schema does not match the target build")
    return flatten(definition)


def checked_profile(arguments, fields, count):
    plain, tail = public_snapshot(arguments)
    projected = [f for f in fields.values() if f["projected"]]
    if len(projected) != count or any(f["type"] not in (0, 8) or f["width"] != 4 for f in projected):
        raise ValueError("unsupported profile projection layout")
    expected = b"".join(bytes([13 if f["type"] == 8 else 7]) + plain[f["offset"]:f["offset"] + 4] for f in projected) + b"\0"
    if tail != expected:
        raise ValueError("profile projection does not match its snapshot")
    return plain


def requirement_minimums(capture, assets, items, client_hash):
    """Accept actual per-item flag requirements, not a list of all flag names.

    Capture format: schema_version=1, read_only=true, client_sha256,
    source_sha256, requirements=[{item_id, flag, minimum}]. The source digest
    records the separately reviewed runtime requirement evidence. Flag names
    and indices must independently exist in the captured unlockFlags table.
    """
    if capture is None:
        return {}, None
    if (capture.get("schema_version") != 1 or capture.get("read_only") is not True
            or capture.get("client_sha256") != client_hash
            or not re.fullmatch(r"[0-9a-fA-F]{64}", capture.get("source_sha256", ""))):
        raise ValueError("invalid target item-requirement capture identity")
    flags = {}
    for name, index, _ in table_rows(assets["tables"]["mp/unlockFlags.csv"], 3):
        if not index:  # The target includes a final reserved index marker.
            continue
        index = int(index)
        if not name or name in flags or index < 1:
            raise ValueError("invalid unlock flag registry")
        flags[name] = index
    entries = capture.get("requirements")
    if not isinstance(entries, list) or len(entries) > 30000:
        raise ValueError("item requirement count")
    required = {}
    for entry in entries:
        if set(entry) != {"item_id", "flag", "minimum"}:
            raise ValueError("unsupported item requirement layout")
        item, flag, minimum = entry["item_id"], entry["flag"], entry["minimum"]
        if type(item) is not int or item not in items or flag not in flags or type(minimum) is not int or not 0 <= minimum <= 32767:
            raise ValueError("requirement is not a target item/named Int16 minimum")
        field = "unlockFlagsNew[%d]" % flags[flag]
        previous = required.setdefault(field, {"minimum": 0, "flags": set(), "items": set()})
        previous["minimum"] = max(previous["minimum"], minimum)
        previous["flags"].add(flag)
        previous["items"].add(item)
    return required, capture["source_sha256"].lower()


def raise_private_unlocks(arguments, fields, user_id, requirements):
    plain = checked_profile(arguments, fields, 36)
    high = value(plain, fields["userIDHigh"]) & 0xffffffff
    low = value(plain, fields["userIDLow"]) & 0xffffffff
    if (high << 32) | low != user_id:
        raise ValueError("private profile owner mismatch")
    edited = bytearray(plain)
    changes = []
    for name, requirement in sorted(requirements.items()):
        field = fields.get(name)
        if not field or field["type"] != 9 or field["width"] != 2 or field["projected"]:
            raise ValueError("requirement is not an unprojected target Int16 unlock flag")
        before = value(plain, field)
        after = max(before, requirement["minimum"])
        if before != after:
            struct.pack_into("<h", edited, field["offset"], after)
            changes.append({"field": name, "offset": field["offset"], "before": before,
                            "after": after, "flags": sorted(requirement["flags"]),
                            "item_ids": sorted(requirement["items"])})
    if not changes:
        return arguments, changes
    crc = zlib.crc32(edited[4:])
    struct.pack_into("<I", edited, 0, crc)
    blob = zlib.compress(edited, 1)
    u32 = lambda n: b"\x08" + struct.pack("<I", n)
    tail = b"".join(bytes([13 if f["type"] == 8 else 7]) + edited[f["offset"]:f["offset"] + 4]
                    for f in fields.values() if f["projected"]) + b"\0"
    result = u32(1) + u32(1) + u32(crc) + b"\x13" + u32(len(blob)) + blob + tail
    checked_profile(result, fields, 36)
    return result, changes


def check_inventory(inventory, title_id, user_id, client_hash):
    if (set(inventory) != {"schema_version", "client_sha256", "title_id", "user_id", "items"}
            or inventory["schema_version"] != 1 or inventory["client_sha256"] != client_hash
            or inventory["title_id"] != title_id or inventory["user_id"] != str(user_id)):
        raise ValueError("existing inventory identity/layout mismatch")
    items = inventory["items"]
    if not isinstance(items, list) or len(items) > MAX_ITEMS or any(type(i) is not int or not 0 < i <= 0xffffffff for i in items) or len(set(items)) != len(items):
        raise ValueError("invalid existing ownership")


def prepare(assets, client_hash, title_id, user_id, files, item_requirements=None):
    items = catalogue(assets, client_hash)
    public_fields = schema(assets, "mp/playerdata.def", PUBLIC_SCHEMA)
    private_fields = schema(assets, "mp/playersecuredata.def", PRIVATE_SCHEMA)
    checked_profile(files["public.bin"], public_fields, 125)
    old_inventory = files.get("inventory.json")
    inventory = (json.loads(old_inventory) if old_inventory is not None else
                 {"schema_version": 1, "client_sha256": client_hash, "title_id": title_id,
                  "user_id": str(user_id), "items": []})
    check_inventory(inventory, title_id, user_id, client_hash)
    previous = set(inventory["items"])
    union = previous | items.keys()
    wallet = files.get("wallet.json")
    purchased = set()
    if wallet is not None:
        parsed = json.loads(wallet)
        if (parsed.get("schema_version") != 1 or parsed.get("client_sha256") != client_hash
                or parsed.get("title_id") != title_id or parsed.get("user_id") != str(user_id)):
            raise ValueError("wallet identity mismatch")
        for grant in parsed.get("purchases", []):
            item = grant["item_id"]
            if type(item) is not int or not 0 < item <= 0xffffffff:
                raise ValueError("invalid existing purchased item")
            purchased.add(item)
    if len(union | purchased) > MAX_ITEMS:
        raise ValueError("full ownership and existing purchases exceed transport capacity")
    required, requirement_source = requirement_minimums(item_requirements, assets, items, client_hash)
    private, changes = raise_private_unlocks(files["private.bin"], private_fields, user_id, required)
    inventory["items"] = sorted(union)
    encoded_inventory = encoded_json(inventory)
    if len(encoded_inventory) > MAX_INVENTORY_BYTES:
        raise ValueError("inventory file exceeds runtime read budget")
    replacements = {"inventory.json": encoded_inventory}
    if private != files["private.bin"]:
        replacements["private.bin"] = private
    replacements = {name: data for name, data in replacements.items() if files.get(name) != data}
    summary = {
        "scope": "local_test_account_all_target_items", "title_id": title_id, "user_id": str(user_id),
        "catalogue_items": len(items), "previous_items": len(previous), "inventory_items": len(union),
        "new_items": len(items.keys() - previous), "preserved_existing_extra_ids": sorted(previous - items.keys()),
        "preserved_purchases": len(purchased), "private_changes": changes,
        "private_flag_policy": "only_captured_item_minimums" if item_requirements is not None else "unchanged_ownership_grants_only",
        "requirement_source_sha256": requirement_source,
        "before_sha256": {name: digest(data) if data is not None else None for name, data in files.items()},
        "after_sha256": {name: digest(replacements.get(name, data)) if replacements.get(name, data) is not None else None for name, data in files.items()},
        "replaced_files": sorted(replacements),
    }
    return replacements, summary


def read_account(directory):
    files = {}
    for name, bound in [("public.bin", 61440), ("private.bin", 61440),
                        ("inventory.json", MAX_INVENTORY_BYTES), ("wallet.json", 32768)]:
        path = directory / name
        if path.is_symlink():
            raise ValueError("account files must not be symlinks")
        if not path.exists() and name in ("inventory.json", "wallet.json"):
            files[name] = None
            continue
        if not path.is_file() or path.stat().st_size > bound:
            raise ValueError("missing/oversized account file: " + name)
        files[name] = path.read_bytes()
    return files


def atomic_write(path, data):
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=path.parent, prefix=".unlock-", delete=False) as f:
            temporary = Path(f.name)
            os.chmod(temporary, 0o600)
            f.write(data)
            f.flush()
            os.fsync(f.fileno())
        os.replace(temporary, path)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def apply(directory, original, replacements, summary):
    if read_account(directory) != original:
        raise ValueError("account changed after preparation; stop services and prepare again")
    if not replacements:
        return None
    backup = directory / ("backup-unlock-all-" + secrets.token_hex(8))
    backup.mkdir(mode=0o700)
    for name, data in original.items():
        if data is not None:
            atomic_write(backup / name, data)
    atomic_write(backup / "manifest.json", encoded_json(summary))
    if read_account(directory) != original:
        raise ValueError("account changed while backing up; no replacements applied")
    changed = []
    try:
        for name, data in replacements.items():
            atomic_write(directory / name, data)
            changed.append(name)
    except Exception:
        for name in reversed(changed):
            if original[name] is None:
                (directory / name).unlink(missing_ok=True)
            else:
                atomic_write(directory / name, original[name])
        raise
    return backup


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--assets", type=Path, required=True)
    parser.add_argument("--service-data-dir", type=Path, required=True)
    parser.add_argument("--title-id", type=int, default=1002)
    parser.add_argument("--user-id", type=int, required=True)
    parser.add_argument("--item-requirements", type=Path)
    parser.add_argument("--apply", action="store_true", help="apply after stopping local services/client; default only prints a plan")
    args = parser.parse_args()
    try:
        if not 0 <= args.title_id <= 0xffffffff or not 0 < args.user_id <= 0xffffffffffffffff:
            raise ValueError("account identity out of range")
        if not args.assets.is_file() or args.assets.stat().st_size > 32 * 1024 * 1024:
            raise ValueError("asset extraction exceeds budget")
        raw_assets = args.assets.read_bytes()
        assets = json.loads(raw_assets)
        client_hash = next(f["sha256"] for f in manifest()["files"] if f["path"] == "CODOL/codoMP_client_shipRetail.exe")
        directory = args.service_data_dir / "profiles" / str(args.title_id) / str(args.user_id)
        for path in [args.service_data_dir, directory.parent.parent, directory.parent, directory]:
            if not path.is_dir() or path.is_symlink():
                raise ValueError("existing regular account directories required")
        requirements = None
        if args.item_requirements:
            if args.item_requirements.stat().st_size > 4 * 1024 * 1024:
                raise ValueError("item requirement capture exceeds budget")
            requirements = json.loads(args.item_requirements.read_bytes())
        original = read_account(directory)
        replacements, report = prepare(assets, client_hash, args.title_id, args.user_id, original, requirements)
        report["assets_sha256"] = digest(raw_assets)
        report["applied"] = args.apply
        if args.apply:
            backup = apply(directory, original, replacements, report)
            report["backup"] = str(backup) if backup else None
        print(json.dumps(report, indent=2))
    except (ValueError, KeyError, OSError, zlib.error) as error:
        parser.error(str(error))


if __name__ == "__main__":
    main()
