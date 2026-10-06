import copy
import struct
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from schema_fields import flatten, value


class SchemaFields(unittest.TestCase):
    def definition(self):
        prop = lambda name, kind, index, leaf, flags=0: {"name": name, "type": kind, "index": index, "leaf": leaf, "extra4": flags}
        return {"rootType": 5, "rootIndex": 0, "size": 32, "leafCount": 5,
                "structs": [{"leafCount": 5, "properties": [prop("items", 6, 0, 0), prop("enabled", 2, 0, 4)]},
                            {"leafCount": 2, "properties": [prop("number", 0, 0, 0, 32), prop("flag", 2, 0, 1)]}],
                "indexedArrays": [{"count": 2, "type": 5, "index": 1}], "enumedArrays": [], "enums": [],
                "offsets": [8, 160, 12, 161, 162]}

    def test_nested_leaf_stride_and_packed_boolean_units(self):
        fields = flatten(self.definition())
        data = bytearray(36); struct.pack_into("<i", data, 12, -9); struct.pack_into("<i", data, 16, 42); data[24] = 0b101
        self.assertEqual(value(data, fields["items[0].number"]), -9)
        self.assertEqual(value(data, fields["items[1].number"]), 42)
        self.assertTrue(value(data, fields["items[0].flag"]))
        self.assertFalse(value(data, fields["items[1].flag"]))
        self.assertTrue(value(data, fields["enabled"]))
        self.assertEqual([n for n, f in fields.items() if f["projected"]], ["items[0].number", "items[1].number"])

    def test_rejects_duplicate_leaves_overlap_and_buffer_escape(self):
        d = self.definition(); d["offsets"][2] = d["offsets"][0]
        with self.assertRaises(ValueError): flatten(d, reject_overlaps=True)
        d = self.definition(); d["offsets"][0] = 32768
        with self.assertRaises(ValueError): flatten(d)
        d = self.definition(); d["structs"][0]["properties"][1]["leaf"] = 3
        with self.assertRaises(ValueError): flatten(d)
        d = self.definition(); d["structs"][1]["properties"][0] = copy.deepcopy(d["structs"][0]["properties"][0])
        with self.assertRaises(ValueError): flatten(d)
