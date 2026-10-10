import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from prepare_bot_roster import animation_config, rows, variant
from test_prepare_bot_locomotion import inputs


class BotRosterPreparation(unittest.TestCase):
    def test_outfit_variant_matches_gender_faction_and_shared_parts(self):
        import base64
        values = [["coat", "model", "m sc sss", "coat_sc"],
                  ["coat", "model", "m tf sss", "coat_tf"],
                  ["coat", "model", "f ss sss", "coat_f"]]
        table = dict(found=True, rows=3, columns=4,
                     cells_base64=[base64.b64encode(x.encode()).decode() for row in values for x in row])
        self.assertEqual(variant(table, "coat", "m", "tf"), "coat_tf")
        self.assertEqual(variant(table, "coat", "f", "sc"), "coat_f")
        with self.assertRaises(ValueError):
            variant(table, "missing", "m", "tf")
        table["rows"] += 1
        with self.assertRaises(ValueError):
            rows(table)

    def test_each_weapon_uses_its_own_native_animation_branch(self):
        import base64
        import struct
        state, capture, arsenal = inputs()
        definition = bytearray(base64.b64decode(arsenal["weapons"][0]["definition_base64"]))
        rifle = animation_config({}, capture, definition)
        self.assertEqual(rifle["reload_animation"], 13)
        struct.pack_into("<I", definition, 0x480, 2)
        machinegun = animation_config({}, capture, definition)
        self.assertNotEqual(rifle["locomotion"]["run_forward"], machinegun["locomotion"]["run_forward"])
        struct.pack_into("<I", definition, 0x478, 3)
        struct.pack_into("<I", definition, 0x480, 3)
        smg = animation_config({}, capture, definition)
        self.assertEqual(smg["reload_animation"], 14)
        struct.pack_into("<I", definition, 0x480, 12)
        with self.assertRaises(ValueError):
            animation_config({}, capture, definition)

    def test_reload_binding_rejects_missing_or_ambiguous_selected_animation(self):
        import base64
        for missing in (True, False):
            state, capture, arsenal = inputs()
            definition = base64.b64decode(arsenal["weapons"][0]["definition_base64"])
            if missing:
                capture["animations"] = [a for a in capture["animations"] if a["name"] != "reload_rifle"]
            else:
                capture["animations"].append({"source":0x07817b80,"name":"reload_rifle","index":90})
            with self.assertRaises(ValueError):
                animation_config({}, capture, definition)


if __name__ == "__main__":
    unittest.main()
