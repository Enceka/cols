import base64
import struct
import sys
import unittest
import zlib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from prepare_bot_locomotion import prepare


def asset(name, text):
    data = text.encode("ascii")
    compressed = zlib.compress(data)
    return {"found": True, "name": name, "length": len(data), "compressed_length": len(compressed),
            "buffer_base64": base64.b64encode(compressed).decode()}


def inputs():
    script = """
ANIMATIONS
STATE COMBAT {
walk { default { both walk } }
walkbk { default { both back } }
run {
 playerAnimType autorifle, weaponclass mg { both lmg_run }
 playerAnimType autorifle { both rifle_run }
 strafing left { both side_run }
 default { both generic_run }
}
runbk { default { both run_back } }
sprint { playerAnimType autorifle { both rifle_sprint } default { both generic_sprint } }
}
EVENTS
fireweapon {
 movetype moving { // Keep the body pose when moving.
 }
 weapon_position ads { torso fire_ads }
 default { torso fire_hip }
}
reload {
 playerAnimType smg { torso reload_smg }
 playerAnimType mg { torso reload_mg }
 default { torso reload_rifle }
}
"""
    names = ["walk", "back", "lmg_run", "rifle_run", "side_run", "generic_run", "run_back",
             "rifle_sprint", "generic_sprint", "fire_ads", "fire_hip", "reload_mg", "reload_rifle", "reload_smg"]
    capture = {"schema_version": 1, "client_sha256": "target", "read_only": True, "ncs_crc": 123,
               "animation_rules": {"mp/playeranim.script": asset("mp/playeranim.script", script),
                                   "mp/playeranimtypes.txt": asset("mp/playeranimtypes.txt", "none\nother\npistol\nsmg\nautorifle\n")},
               "animations": [{"source": 0x07817b80, "name": name, "index": i + 1} for i, name in enumerate(names)]}
    definition = bytearray(0xe28)
    struct.pack_into("<I", definition, 0x478, 4)
    arsenal = {"client_sha256": "target", "read_only": True, "ncs_crc": 123,
               "weapons": [{"name": "m4_mp", "definition_base64": base64.b64encode(definition).decode()}]}
    state = {"client_sha256": "target", "ncs_crc": 123, "training_enemy": {}}
    return state, capture, arsenal


class NativeBotAnimationPreparation(unittest.TestCase):
    def test_native_rule_order_wins_over_later_strafe_rules_and_suppresses_moving_overlay(self):
        state, capture, arsenal = inputs()
        # Native metadata contains many duplicated unused placeholders.
        capture["animations"] += [{"source": 0x07817b80, "name": "unused", "index": n} for n in (90, 91)]
        bindings = prepare(state, capture, arsenal, "target")
        self.assertEqual(bindings["run_left"][1], "rifle_run")
        self.assertEqual(bindings["run_forward"], bindings["run_left"])
        self.assertEqual(state["training_enemy"]["locomotion"]["sprint"], 8)
        self.assertEqual(state["training_enemy"]["locomotion"]["fire_standing"], 11)
        self.assertEqual(len(state["training_enemy"]["locomotion"]["script_sha256"]), 64)

    def test_rejects_wrong_map_missing_rules_ambiguous_selected_ids_and_invalid_compression(self):
        for change in ("map", "ambiguous", "compression", "length", "class"):
            state, capture, arsenal = inputs()
            if change == "map":
                capture["ncs_crc"] += 1
            elif change == "ambiguous":
                capture["animations"].append({"source": 0x07817b80, "name": "rifle_run", "index": 99})
            elif change == "compression":
                capture["animation_rules"]["mp/playeranim.script"]["buffer_base64"] = base64.b64encode(b"bad").decode()
            elif change == "length":
                capture["animation_rules"]["mp/playeranim.script"]["length"] = 4
            else:
                definition = bytearray(base64.b64decode(arsenal["weapons"][0]["definition_base64"]))
                struct.pack_into("<I", definition, 0x480, 4)
                arsenal["weapons"][0]["definition_base64"] = base64.b64encode(definition).decode()
            with self.subTest(change=change), self.assertRaises(ValueError):
                prepare(state, capture, arsenal, "target")


if __name__ == "__main__":
    unittest.main()
