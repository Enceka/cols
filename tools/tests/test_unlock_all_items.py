import base64
import copy
import json
from pathlib import Path
import struct
import sys
import tempfile
import unittest
from unittest.mock import patch
import zlib

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import unlock_all_items as unlock


CLIENT = "e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b"
UID = 16660678484126794031


def fields(count, private=False):
    result = {"projected[%d]" % i: {"type": 0, "width": 4, "offset": 12 + 4 * i, "projected": True}
              for i in range(count)}
    if private:
        result.update({
            "userIDHigh": {"type": 0, "width": 4, "offset": 4943, "projected": False},
            "userIDLow": {"type": 0, "width": 4, "offset": 4947, "projected": False},
            "unlockFlagsNew[754]": {"type": 9, "width": 2, "offset": 9803, "projected": False},
        })
    return result


def profile(layout, private=False):
    plain = bytearray(32768)
    plain[21000:21008] = b"retained"
    if private:
        struct.pack_into("<II", plain, 4943, UID >> 32, UID & 0xffffffff)
    crc = zlib.crc32(plain[4:])
    struct.pack_into("<I", plain, 0, crc)
    blob = zlib.compress(plain, 1)
    u32 = lambda n: b"\x08" + struct.pack("<I", n)
    tail = b"".join(b"\x07" + plain[f["offset"]:f["offset"] + 4]
                    for f in layout.values() if f["projected"]) + b"\0"
    return u32(1) + u32(1) + u32(crc) + b"\x13" + u32(len(blob)) + blob + tail


class UnlockAllItems(unittest.TestCase):
    def setUp(self):
        self.public_fields, self.private_fields = fields(125), fields(36, True)
        rows = [[str(i), "item_%d" % i, "wat"] + [""] * 9 for i in range(1, 6773)]
        self.assets = {"schema_version": 1, "read_only": True, "client_sha256": CLIENT,
                       "tables": {"mp/coMarketplaceItems.csv": {
                           "found": True, "columns": 12, "rows": len(rows),
                           "cells_base64": [base64.b64encode(c.encode()).decode() for r in rows for c in r]},
                           "mp/unlockFlags.csv": {"found": True, "columns": 3, "rows": 1,
                               "cells_base64": [base64.b64encode(c.encode()).decode() for c in ["uf_m4_mp_rank", "754", "1"]]}}}
        self.inventory = {"schema_version": 1, "client_sha256": CLIENT, "title_id": 1002,
                          "user_id": str(UID), "items": [5, 8000]}
        self.wallet = {"schema_version": 1, "client_sha256": CLIENT, "title_id": 1002,
                       "user_id": str(UID), "cp": 123, "gp": 456,
                       "purchases": [{"sku_id": 999, "item_id": 6000, "payment": "gp", "price": 42}]}
        self.files = {"public.bin": profile(self.public_fields),
                      "private.bin": profile(self.private_fields, True),
                      "inventory.json": unlock.encoded_json(self.inventory),
                      "wallet.json": unlock.encoded_json(self.wallet)}
        self.schema = patch.object(unlock, "schema", side_effect=lambda _, name, __:
                                   self.private_fields if "secure" in name else self.public_fields)
        self.schema.start()
        self.addCleanup(self.schema.stop)

    def prepare(self, requirements=None):
        return unlock.prepare(self.assets, CLIENT, 1002, UID, self.files, requirements)

    def test_complete_ownership_preserves_existing_progress_wallet_and_purchases(self):
        replacements, report = self.prepare()
        self.assertEqual(set(replacements), {"inventory.json"})
        inventory = json.loads(replacements["inventory.json"])
        self.assertEqual(inventory["items"], list(range(1, 6773)) + [8000])
        self.assertGreater(len(replacements["inventory.json"]), 32768)
        self.assertEqual(report["catalogue_items"], 6772)
        self.assertEqual(report["preserved_purchases"], 1)
        self.assertEqual(report["private_changes"], [])
        for name in ["private.bin", "public.bin", "wallet.json"]:
            self.assertEqual(report["before_sha256"][name], report["after_sha256"][name])
        self.files.update(replacements)
        self.assertEqual(self.prepare()[0], {})

    def test_offline_apply_backs_up_every_original_before_replacing_only_inventory(self):
        replacements, report = self.prepare()
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            for name, data in self.files.items():
                (directory / name).write_bytes(data)
            backup = unlock.apply(directory, self.files, replacements, report)
            for name, original in self.files.items():
                self.assertEqual((backup / name).read_bytes(), original)
                self.assertEqual((directory / name).read_bytes(), replacements.get(name, original))
            self.assertTrue((backup / "manifest.json").is_file())

    def test_account_changes_after_preparation_are_rejected_without_replacing_any_file(self):
        replacements, report = self.prepare()
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            for name, data in self.files.items():
                (directory / name).write_bytes(data)
            (directory / "wallet.json").write_bytes(b"changed")
            with self.assertRaises(ValueError):
                unlock.apply(directory, self.files, replacements, report)
            self.assertEqual((directory / "inventory.json").read_bytes(), self.files["inventory.json"])
            self.assertEqual(list(directory.glob("backup-*")), [])

    def test_optional_requirements_raise_only_verified_named_minimums(self):
        capture = {"schema_version": 1, "read_only": True, "client_sha256": CLIENT,
                   "source_sha256": "a" * 64,
                   "requirements": [{"item_id": 1, "flag": "uf_m4_mp_rank", "minimum": 7},
                                    {"item_id": 2, "flag": "uf_m4_mp_rank", "minimum": 5}]}
        replacements, report = self.prepare(capture)
        old = unlock.checked_profile(self.files["private.bin"], self.private_fields, 36)
        new = unlock.checked_profile(replacements["private.bin"], self.private_fields, 36)
        self.assertEqual([i for i in range(4, 32768) if old[i] != new[i]], [9803])
        self.assertEqual(struct.unpack_from("<h", new, 9803)[0], 7)
        self.assertEqual(report["private_changes"][0]["item_ids"], [1, 2])
        for key, bad in [("flag", "uf_not_captured"), ("item_id", 9000), ("minimum", 32768)]:
            invalid = copy.deepcopy(capture)
            invalid["requirements"][0][key] = bad
            with self.assertRaises(ValueError):
                self.prepare(invalid)

    def test_wrong_client_account_and_corrupt_profile_are_rejected(self):
        self.assets["client_sha256"] = "b" * 64
        with self.assertRaises(ValueError):
            self.prepare()
        self.assets["client_sha256"] = CLIENT
        invalid = dict(self.inventory, user_id="1")
        self.files["inventory.json"] = unlock.encoded_json(invalid)
        with self.assertRaises(ValueError):
            self.prepare()
        self.files["inventory.json"] = unlock.encoded_json(self.inventory)
        bad = bytearray(self.files["private.bin"])
        bad[11] ^= 1
        self.files["private.bin"] = bytes(bad)
        with self.assertRaises(ValueError):
            self.prepare()


if __name__ == "__main__":
    unittest.main()
