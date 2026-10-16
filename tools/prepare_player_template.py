#!/usr/bin/env python3
"""Prepare an external starter template from verified 3.3.3.3 client data.

Only developers preparing a local installation need Python; the launcher uses
the Rust backend to create players from this file.
"""
import argparse
import base64
import json
import os
from pathlib import Path

from prepare_inventory import public_snapshot, selected_items
from prepare_lobby_profile import mark_complete
from prepare_default_rewards import seed_empty_rewards
from schema_fields import flatten, value
from unlock_all_items import catalogue


def prepare(assets, public, user_id):
    client_hash = "e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b"
    items = catalogue(assets, client_hash)
    definition = assets["schemas"]["mp/playerdata.def"]["definitions"][0]
    if (definition["version"], definition["formatChecksum"]) != (233, 3811397846):
        raise ValueError("starter public schema mismatch")
    fields = flatten(definition)
    snapshot, tail = public_snapshot(public)
    projected = [f for f in fields.values() if f["projected"]]
    expected = b"".join(b"\x07" + snapshot[f["offset"]:f["offset"] + 4] for f in projected) + b"\0"
    if len(projected) != 125 or expected != tail:
        raise ValueError("starter projections mismatch")
    public, rewards, _ = seed_empty_rewards(public, fields)
    snapshot, _ = public_snapshot(public)
    starter = list(dict.fromkeys(selected_items(fields, snapshot) + rewards))
    if any(item not in items for item in starter):
        raise ValueError("starter equipment is absent from this catalogue")
    if not 0 < user_id <= 0xffffffffffffffff:
        raise ValueError("native user id is outside u64")
    # This verified local template opens the lobby without copying private progress.
    field = fields["uiNotificationFlags[87]"]
    if (field["offset"], field["bit"], field["type"]) != (4390, 0, 2):
        raise ValueError("onboard_complete layout mismatch")
    if not value(snapshot, field):
        public, _ = mark_complete(public, fields, 87)
    return {"schema_version": 1, "client_sha256": client_hash,
            "native_user_id": str(user_id), "public_base64": base64.b64encode(public).decode(),
            "starter_items": starter, "all_items": sorted(items)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--assets", type=Path, required=True)
    parser.add_argument("--public-profile", type=Path, required=True)
    parser.add_argument("--user-id", type=int, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.assets.stat().st_size > 16 * 1024 * 1024 or args.public_profile.stat().st_size > 61440:
        parser.error("template input exceeds limit")
    result = prepare(json.loads(args.assets.read_text()), args.public_profile.read_bytes(), args.user_id)
    data = json.dumps(result, separators=(",", ":")).encode()
    if len(data) > 256 * 1024:
        parser.error("template exceeds runtime budget")
    with os.fdopen(os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "wb") as output:
        output.write(data)
        output.flush()
        os.fsync(output.fileno())
    print(json.dumps({"created": str(args.output), "starter_items": len(result["starter_items"]),
                      "catalogue_items": len(result["all_items"])}))


if __name__ == "__main__":
    main()
