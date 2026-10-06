import struct
import sys
import unittest
import zlib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from prepare_inventory import public_snapshot, selected_items


class InventoryPreparation(unittest.TestCase):
    def test_profile_crc_and_expansion_validation(self):
        args = (Path(__file__).resolve().parents[2] / "fixtures/protocol/public-profile-synthetic.bin").read_bytes()
        snapshot, tail = public_snapshot(args)
        self.assertEqual(len(snapshot), 32768)
        self.assertEqual(len(tail), 626)
        for offset in (0, 5, 11, 21):
            bad = bytearray(args); bad[offset] ^= 1
            with self.assertRaises((ValueError, zlib.error)): public_snapshot(bytes(bad))

    def test_only_core_equipment_is_selected_not_perk_or_wildcard_scalars(self):
        fields = {}; data = bytearray(2048); at = 12
        def field(name, v):
            nonlocal at
            fields[name] = {"offset": at, "type": 0, "width": 4}; struct.pack_into("<i", data, at, v); at += 4
        field("currentCharacterIndex", 0)
        for i in range(9): field("characterClasses[0].slots[%d]" % i, 792 if i == 0 else 0)
        for name in ("primaryBase", "secondaryBase", "meleeWpn", "lethal", "tactical"): field("weaponClasses[0]." + name, 10001 if name == "primaryBase" else 0)
        for group in ("primaryAttach", "secondaryAttach"):
            for i in range(6): field("weaponClasses[0].%s[%d]" % (group, i), 0)
        field("weaponClasses[0].perks[3]", 1); field("weaponClasses[0].wildcards[0]", 1)
        self.assertEqual(selected_items(fields, data), [792, 10001])
        struct.pack_into("<i", data, fields["weaponClasses[0].primaryBase"]["offset"], -1)
        with self.assertRaises(ValueError): selected_items(fields, data)
