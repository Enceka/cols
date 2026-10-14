#!/usr/bin/env python3
"""Grant native story unlocks only in an offline, marked test account copy.

The client reads unlockFlagsNew Int16 values via0x4a4ca0. These grants are
diagnostic completion markers, not authoritative wins or rewards.
"""
import argparse
import json
from pathlib import Path
import re

from prepare_story import CLIENT_SHA256, identity
from unlock_all_items import (PRIVATE_SCHEMA, apply, checked_profile, digest,
                              raise_private_unlocks, read_account, schema, table_rows)


def prepare(assets, user_id, original):
    identity(assets)
    fields = schema(assets, "mp/playersecuredata.def", PRIVATE_SCHEMA)
    flags, requirements = [], {}
    for name, number, _ in table_rows(assets["tables"]["mp/unlockFlags.csv"], 3):
        if not name.startswith("cf_ch_coop_"):
            continue
        if not re.fullmatch(r"cf_ch_coop_[a-z0-9_]+", name) or not number.isdecimal():
            raise ValueError("invalid native story completion flag")
        index = int(number)
        field = "unlockFlagsNew[%d]" % index
        if not 0 <= index < 2048 or field in requirements:
            raise ValueError("ambiguous native story flag")
        flags.append(name)
        requirements[field] = dict(minimum=1, flags={name}, items=set())
    if "cf_ch_coop_training_mp_cargoship" not in flags:
        raise ValueError("first story completion flag missing")
    replacement, changes = raise_private_unlocks(original["private.bin"], fields, user_id, requirements)
    before = checked_profile(original["private.bin"], fields, 36)
    after = checked_profile(replacement, fields, 36)
    allowed = {i for field in requirements for i in range(fields[field]["offset"], fields[field]["offset"]+2)}
    if any(before[i] != after[i] and i not in allowed for i in range(4, len(before))):
        raise ValueError("story grant modified unrelated progress")
    replacements = {"private.bin": replacement} if replacement != original["private.bin"] else {}
    report = dict(schema_version=1, client_sha256=CLIENT_SHA256, scope="isolated_test_story_unlocks",
                  authoritative_completion=False, rewards_granted=False,
                  native_flags=len(flags), private_changes=changes,
                  before_sha256={n: digest(b) if b is not None else None for n,b in original.items()},
                  after_sha256={n: digest(replacements.get(n,b)) if b is not None else None for n,b in original.items()})
    return replacements, report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--assets", type=Path, required=True)
    parser.add_argument("--service-data-dir", type=Path, required=True)
    parser.add_argument("--user-id", type=int, required=True)
    parser.add_argument("--title-id", type=int, default=1002)
    parser.add_argument("--apply", action="store_true")
    args = parser.parse_args()
    root = args.service_data_dir.resolve()
    marker = root / ".codol-story-capture.json"
    if not marker.is_file() or marker.is_symlink():
        parser.error("isolated story service-data marker required")
    mark = json.loads(marker.read_bytes())
    if mark.get("isolated") is not True or mark.get("client_sha256") != CLIENT_SHA256:
        parser.error("story service-data identity mismatch")
    directory = root / "profiles" / str(args.title_id) / str(args.user_id)
    if not directory.is_dir() or directory.is_symlink() or args.assets.stat().st_size > 32*1024*1024:
        parser.error("existing regular test account and bounded schema capture required")
    raw = args.assets.read_bytes()
    original = read_account(directory)
    replacements, report = prepare(json.loads(raw), args.user_id, original)
    report.update(assets_sha256=digest(raw), applied=args.apply)
    if args.apply:
        backup = apply(directory, original, replacements, report)
        report["backup"] = str(backup) if backup else None
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
