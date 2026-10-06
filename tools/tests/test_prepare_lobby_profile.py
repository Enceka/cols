import struct
import sys
import unittest
import zlib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from prepare_inventory import public_snapshot
from prepare_lobby_profile import mark_complete


class LobbyProfilePreparation(unittest.TestCase):
    def sample(self):
        plain = bytearray(32768); crc = zlib.crc32(plain[4:]); plain[:4] = struct.pack("<I", crc)
        blob = zlib.compress(plain, 1)
        u32 = lambda n: b"\x08" + struct.pack("<I", n)
        args = u32(1) + u32(1) + u32(crc) + b"\x13" + u32(len(blob)) + blob + b"\x07\0\0\0\0" * 125 + b"\0"
        fields = {"synthetic[%d]" % i: {"type": 0, "offset": 16 + i * 4, "projected": True} for i in range(125)}
        fields["uiNotificationFlags[87]"] = {"type": 2, "offset": 4390, "bit": 0, "projected": False}
        return args, fields

    def test_changes_only_one_boolean_plus_crc_and_preserves_projection(self):
        args, fields = self.sample()
        result, info = mark_complete(args, fields, 87)
        before, old_tail = public_snapshot(args); after, new_tail = public_snapshot(result)
        self.assertEqual([i for i in range(4, 32768) if before[i] != after[i]], [4390])
        self.assertEqual(after[4390] ^ before[4390], 1)
        self.assertEqual(old_tail, new_tail)
        self.assertEqual(info["field"], "uiNotificationFlags[87]")
        with self.assertRaises(ValueError): mark_complete(result, fields, 87)

    def test_refuses_wrong_field_type_and_inconsistent_projection(self):
        args, fields = self.sample(); fields["uiNotificationFlags[87]"]["type"] = 0
        with self.assertRaises(ValueError): mark_complete(args, fields, 87)
        args, fields = self.sample(); bad = bytearray(args); bad[-2] = 1
        with self.assertRaises(ValueError): mark_complete(bytes(bad), fields, 87)
