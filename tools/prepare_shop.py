#!/usr/bin/env python3
"""Prepare the local permanent-item shop from target runtime SKU/product tables.

Only enabled, untimed, single-item offers with no product requirements are used.
Does not synthesize prices, overwrite a shop, or grant anything to an account.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import struct
import tempfile

from clientlib import manifest


def offers_from_assets(assets):
    data = assets["runtime_marketplace"]
    products = {}
    for product in data["products"]:
        header = base64.b64decode(product["header_base64"], validate=True)
        if len(header) != 24:
            raise ValueError("unexpected product stride")
        fields = struct.unpack("<5HBB3I", header)
        product_id = fields[0]
        if product_id in products:
            raise ValueError("duplicate product ID")
        items = product["item_ids"]
        if fields[5] != len(items):
            raise ValueError("product item count mismatch")
        products[product_id] = (fields, items)
    table = assets["tables"]["mp/coMarketplaceItems.csv"]
    if not table.get("found") or table["columns"] != 12 or len(table["cells_base64"]) != table["rows"] * 12:
        raise ValueError("target item table missing or incomplete")
    item_types = {int(base64.b64decode(table["cells_base64"][i * 12])):
                  base64.b64decode(table["cells_base64"][i * 12 + 2]).decode("ascii")
                  for i in range(table["rows"])}
    offers, seen = [], set()
    for row in data["skus"]:
        fields = row["fields"]
        if len(fields) != 9 or any(type(v) is not int or v < 0 for v in fields):
            raise ValueError("invalid decoded SKU record")
        sku, flags, product_id, cp, gp, duration, extra, starts, ends = fields
        if sku in seen:
            raise ValueError("duplicate SKU ID")
        seen.add(sku)
        if not sku or flags != 5 or any([duration, extra, starts, ends, row["byte14"], row["byte15"]]):
            continue
        if not 1 <= sku <= 65535 or product_id not in products:
            raise ValueError("SKU references invalid product")
        header, items = products[product_id]
        if any(header[1:5]) or len(items) != 1 or header[6]:
            continue
        item = items[0]
        if item not in item_types or not 1 <= item <= 65535:
            raise ValueError("product item absent from target table")
        if item_types[item] not in {"wea", "wat", "cat", "cha", "pcb", "pct", "pci", "bdg"}:
            continue  # Consumables, perks, streaks and equipment need their own grant semantics.
        if cp and gp:  # Unobserved multi-currency offer; do not choose for the client.
            continue
        price, payment = (cp, "cp") if cp else (gp, "gp")
        if 0 < price <= 65535:
            offers.append({"sku_id": sku, "item_id": item, "payment": payment, "price": price})
    if not offers or len(offers) > 30000:
        raise ValueError("local shop count outside supported limit")
    return sorted(offers, key=lambda row: row["sku_id"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--assets", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--title-id", type=int, default=1002)
    args = parser.parse_args()
    if args.assets.stat().st_size > 32 * 1024 * 1024 or not 0 <= args.title_id <= 2**32 - 1:
        parser.error("preparation input outside budget")
    raw = args.assets.read_bytes()
    assets = json.loads(raw)
    client = next(f["sha256"] for f in manifest()["files"] if f["path"] == "CODOL/codoMP_client_shipRetail.exe")
    if assets.get("client_sha256") != client or assets.get("read_only") is not True:
        parser.error("expected target read-only runtime extraction")
    offers = offers_from_assets(assets)
    payload = {"schema_version": 1, "client_sha256": client, "title_id": args.title_id,
               "source_sha256": hashlib.sha256(raw).hexdigest(), "offers": offers}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=args.output.parent, prefix=".shop-", delete=False) as file:
            temporary = Path(file.name)
            file.write((json.dumps(payload, indent=2) + "\n").encode())
            file.flush()
            os.fsync(file.fileno())
        os.link(temporary, args.output)
    finally:
        if temporary:
            temporary.unlink(missing_ok=True)
    print(json.dumps({"created": str(args.output), "offers": len(offers), "scope": "local_permanent_single_item_purchases"}))


if __name__ == "__main__":
    main()
