import base64
import copy
import sys
import unittest
from pathlib import Path
import zlib

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from prepare_game_state import CLIENT_SHA256, compute, spawn_origin


def encoded(values):
    return [base64.b64encode(v).decode("ascii") for v in values]


def asset(name, category, source, values):
    return dict(name=name, category=category, source=source,
                count=len(values), strings_base64=encoded(values))


def capture():
    return dict(schema_version=1, read_only=True, client_sha256=CLIENT_SHA256,
                network_dvars_base64=encoded([b"100", b"101"]),
                common_checksum=zlib.crc32(b"100101"), localized_prefix="chi_",
                initialized=0, checksum=0, linked=[], assets=[
                    asset("ncs_mdl_level", 0, 3, [b"a", b"C"]),
                    asset("ncs_mdl_common", 0, 0, [b"a", b"a", b"b"]),
                    asset("ncs_mat_common", 1, 0, [b"a"]),
                    asset("ncs_fxt_common", 15, 0, [b"ignored"]),
                    asset("void", 21, 4, []),
                ])


class GameStatePreparation(unittest.TestCase):
    def test_spawn_comes_from_expected_map_and_rejects_invalid_coordinates(self):
        def capture_map(text):
            return dict(schema_version=1,read_only=True,client_sha256=CLIENT_SHA256,maps=[dict(
                name="maps/mp/mp_afghan.d3dbsp",byte_count=len(text),entity_string_base64=encoded([text])[0])])
        text = b'{"classname" "worldspawn"}\n{"classname" "mp_tdm_spawn_allies_start" "origin" "1 2 -3.5"}'
        self.assertEqual(spawn_origin(capture_map(text)),[1,2,-3.5])
        for invalid in [b"NaN 2 3",b"1 2",b"1 2 1000001"]:
            with self.assertRaises(ValueError): spawn_origin(capture_map(text.replace(b"1 2 -3.5",invalid)))
        wrong=capture_map(text);wrong["maps"][0]["name"]="another_map"
        with self.assertRaises(ValueError): spawn_origin(wrong)

    def test_seed_resource_order_dedup_and_no_terminators(self):
        data = capture()
        result = compute(data)
        # Same-asset duplicate survives; cross-asset duplicate vanishes;
        # same string in a DIFFERENT category survives. Level sorts last.
        self.assertEqual(result["ncs_crc"], zlib.crc32(b"100101aabaC"))
        self.assertEqual(result["constant_count"], 5)
        data["initialized"], data["checksum"] = 1, result["ncs_crc"]
        self.assertTrue(compute(data)["observed_crc_verified"])
        data["checksum"] ^= 1
        with self.assertRaises(ValueError): compute(data)

    def test_language_sort_and_case_sensitive_value_dedup(self):
        data = capture()
        data["assets"] = [asset("ncs_mdl_chi_a", 0, 0, [b"A"]),
                          asset("ncs_mdl_zbase", 0, 0, [b"a"])]
        # Both names must have >12 bytes to activate language classification.
        data["assets"][0]["name"] += "long"
        data["assets"][1]["name"] += "long"
        self.assertEqual(compute(data)["ncs_crc"], zlib.crc32(b"100101aA"))

    def test_incomplete_resource_and_seed_mismatches_fail_closed(self):
        original = capture()
        variants = []
        for key, value in [("client_sha256", "wrong"), ("common_checksum", 0),
                           ("network_dvars_base64", []), ("assets", [{"error": "partial"}])]:
            data = copy.deepcopy(original); data[key] = value; variants.append(data)
        for field, value in [("count", 50), ("category", 21), ("strings_base64", ["!"])]:
            data = copy.deepcopy(original); data["assets"][0][field] = value; variants.append(data)
        for data in variants:
            with self.assertRaises(ValueError): compute(data)
