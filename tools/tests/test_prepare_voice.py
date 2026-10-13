import base64
import copy
import json
from pathlib import Path
import struct
import sys
import subprocess
import tempfile
import unittest
import zlib

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from prepare_game_state import CLIENT_SHA256, compute
from prepare_voice import prepare


def resource(name, category, source, values):
    return dict(name=name, category=category, source=source, count=len(values),
                strings_base64=[base64.b64encode(v.encode()).decode() for v in values])


def inputs():
    layout = bytearray(21 * 12)
    struct.pack_into("<III", layout, 5 * 12, 7058, 0, 0)
    struct.pack_into("<III", layout, 6 * 12, 3107, 512, 1)
    first = resource("ncs_snd_common", 5, 0, ["ab_1mc_boost", "uk_1mc_boost"])
    last = resource("ncs_snd_level", 5, 3, ["uk_1mc_boost", "uk_f2_mp_pain"])
    dynamic = resource("ncs_snl_common", 6, 0, ["ui_mp_timer_countdown"])
    data = dict(schema_version=1, read_only=True, client_sha256=CLIENT_SHA256,
                initialized=0, checksum=0, connection_state=9, localized_prefix="chi_",
                common_checksum=zlib.crc32(b"100"),
                network_dvars_base64=[base64.b64encode(b"100").decode()],
                category_layout_base64=base64.b64encode(layout).decode(),
                assets=[last, dynamic, first], linked=[])
    crc = compute(data)["ncs_crc"]
    data.update(initialized=1, checksum=crc, linked=[copy.deepcopy(first),
                resource("ncs_snd_level", 5, 3, ["uk_f2_mp_pain"]), copy.deepcopy(dynamic)])
    state = dict(schema_version=1, client_sha256=CLIENT_SHA256,
                 map="mp_test", mode="war", ncs_crc=crc)
    return data, state


def encode(value):
    return json.dumps(value).encode()


class VoicePreparation(unittest.TestCase):
    def test_state_output_preserves_source_and_resolves_existing_resource_paths(self):
        data, state = inputs()
        state["collision_file"] = "collision.json"
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); source = root / "source"; source.mkdir()
            ncs = source / "ncs.json"; ncs.write_bytes(encode(data))
            game = source / "game.json"; game.write_bytes(encode(state)); before = game.read_bytes()
            catalog = root / "diagnostic" / "voices.json"
            prepared = root / "diagnostic" / "map.json"
            script = Path(__file__).resolve().parents[1] / "prepare_voice.py"
            command = [sys.executable, str(script), str(ncs), "--game-state", str(game),
                       "--output", str(catalog), "--state-output", str(prepared), "--voice-variant", "2"]
            subprocess.run(command, check=True, capture_output=True)
            result = json.loads(prepared.read_bytes())
            self.assertEqual(game.read_bytes(), before)
            self.assertEqual((prepared.parent / result["voice_file"]).resolve(), catalog.resolve())
            self.assertEqual(result["collision_file"], str((source / "collision.json").resolve()))
            self.assertEqual(json.loads(catalog.read_bytes())["voice_variant"], 2)
            failed = subprocess.run(command, capture_output=True)
            self.assertNotEqual(failed.returncode, 0)
            self.assertEqual(game.read_bytes(), before)

    def test_constant_offset_zero_and_dynamic_countdown_use_separate_banks(self):
        data, state = inputs()
        catalog = prepare(encode(data), encode(state))
        self.assertEqual(catalog["mode_intro_gap_msec"], 3000)
        self.assertEqual(catalog["constant_sound_count"], 3)
        self.assertEqual(catalog["announcers"]["sc"]["match_start"]["play_command"], "p 512")
        self.assertEqual(catalog["announcers"]["tf"]["match_start"]["play_command"], "p 513")
        self.assertFalse(catalog["announcers"]["tf"]["mode_tdm"]["available"])
        self.assertEqual(catalog["countdown"]["configstring"], 3108)
        self.assertEqual(catalog["countdown"]["play_command"], "p 1")
        female = catalog["characters"]["uk_f2_"]
        self.assertEqual(female["prefix_command"], 's cg_vo_prefix "UK_F2_"')
        self.assertEqual(female["events"]["pain"]["selector"], 514)
        self.assertEqual(female["events"]["pain"]["resource_status"], "constant_only")

    def test_matching_loaded_alias_is_evidence_without_claiming_audio(self):
        data, state = inputs()
        voice = dict(schema_version=1, read_only=True, client_sha256=CLIENT_SHA256,
                     connection_state=10, ncs_crc=state["ncs_crc"], map="mp_test",
                     aliases=[dict(name="UK_F2_MP_PAIN", entry_flags=1, count=4)])
        catalog = prepare(encode(data), encode(state), encode(voice))
        cue = catalog["characters"]["uk_f2_"]["events"]["pain"]
        self.assertEqual(cue["resource_status"], "captured_alias")
        self.assertEqual(cue["variant_count"], 4)
        self.assertIn("audio acceptance pending", catalog["evidence"])
        self.assertEqual(len(catalog["sources_sha256"]), 3)
        voice["map"] = "maps/mp/mp_test.d3dbsp"
        voice["aliases"][0]["entry_flags"] = 2
        self.assertEqual(prepare(encode(data), encode(state), encode(voice))["characters"]["uk_f2_"]["events"]["pain"]["resource_status"], "captured_alias")
        voice["map"] = "mp_another"
        with self.assertRaises(ValueError): prepare(encode(data), encode(state), encode(voice))

    def test_crc_loaded_order_layout_and_initialization_are_required(self):
        data, state = inputs()
        bad = copy.deepcopy(data);bad["linked"][0]["strings_base64"].reverse()
        with self.assertRaisesRegex(ValueError, "order"):
            prepare(encode(bad), encode(state))
        bad = copy.deepcopy(data);bad["linked"][0]["count"] += 1
        with self.assertRaisesRegex(ValueError, "count"):
            prepare(encode(bad), encode(state))
        bad = copy.deepcopy(data);bad["checksum"] ^= 1
        with self.assertRaisesRegex(ValueError, "CRC"):
            prepare(encode(bad), encode(state))
        bad = copy.deepcopy(data);bad["initialized"] = 0
        with self.assertRaisesRegex(ValueError, "initialized"):
            prepare(encode(bad), encode(state))
        bad = copy.deepcopy(state);bad["ncs_crc"] ^= 1
        with self.assertRaisesRegex(ValueError, "another"):
            prepare(encode(data), encode(bad))
        layout = bytearray(base64.b64decode(data["category_layout_base64"]))
        struct.pack_into("<III", layout, 5 * 12, 7058, 512, 1)
        data["category_layout_base64"] = base64.b64encode(layout).decode()
        with self.assertRaisesRegex(ValueError, "layout"):
            prepare(encode(data), encode(state))

    def test_mode_intro_interval_is_bounded_without_reassigning_resource_indices(self):
        data, state = inputs()
        normal = prepare(encode(data), encode(state))
        changed = prepare(encode(data), encode(state), mode_intro_gap_msec=4000)
        self.assertEqual(changed["all_constant_sounds"], normal["all_constant_sounds"])
        self.assertEqual(changed["mode_intro_gap_msec"], 4000)
        for invalid in [0, 999, 10001, True, 3000.0]:
            with self.assertRaisesRegex(ValueError, "interval"):
                prepare(encode(data), encode(state), mode_intro_gap_msec=invalid)


if __name__ == "__main__":
    unittest.main()
