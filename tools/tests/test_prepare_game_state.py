import base64
import copy
import sys
import unittest
from pathlib import Path
import zlib

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from prepare_game_state import CLIENT_SHA256, compute, spawn_origin, weapon_ammo, weapon_parts
import json
import struct


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
    def test_replay_prompt_uses_sorted_localized_bank_and_loaded_order(self):
        data = capture()
        layout = bytearray(21 * 12)
        struct.pack_into("<III", layout, 4 * 12, 952, 1023, 1)
        data["category_layout_base64"] = base64.b64encode(layout).decode()
        data["assets"] = [asset("ncs_loc_level", 4, 3, [b"first", b"PLATFORM_PRESS_TO_SKIP"]),
                          asset("ncs_loc_common", 4, 0, [b"first"])]
        result = compute(data)
        self.assertEqual(result["replay_resources"], dict(ncs_crc=result["ncs_crc"], skip_text=2))
        data.update(initialized=1, checksum=result["ncs_crc"],
                    linked=[asset("loaded", 4, 0, [b"first", b"PLATFORM_PRESS_TO_SKIP"])])
        self.assertEqual(compute(data)["replay_resources"], result["replay_resources"])
        data["linked"][0]["strings_base64"].reverse()
        with self.assertRaises(ValueError): compute(data)
        data["initialized"] = 0
        struct.pack_into("<III", layout, 4 * 12, 952, 1023, 0)
        data["category_layout_base64"] = base64.b64encode(layout).decode()
        with self.assertRaises(ValueError): compute(data)

    def test_top_players_uses_zero_based_loaded_menu_bank(self):
        data = capture()
        layout = bytearray(21 * 12)
        struct.pack_into("<III", layout, 8 * 12, 4291, 64, 0)
        data["category_layout_base64"] = base64.b64encode(layout).decode()
        data["assets"] = [asset("ncs_mnu_level", 8, 3, [b"sf_top3players", b"pause"]),
                          asset("ncs_mnu_common", 8, 0, [b"pause"])]
        result = compute(data)
        self.assertEqual(result["top_players"], dict(ncs_crc=result["ncs_crc"], menu_index=1))
        data.update(initialized=1, checksum=result["ncs_crc"],
                    linked=[asset("loaded", 8, 0, [b"pause", b"sf_top3players"])])
        self.assertEqual(compute(data), {**result, "observed_crc_verified": True})
        data["linked"][0]["strings_base64"].reverse()
        with self.assertRaises(ValueError): compute(data)
        data["initialized"] = 0
        struct.pack_into("<III", layout, 8 * 12, 4291, 64, 1)
        data["category_layout_base64"] = base64.b64encode(layout).decode()
        with self.assertRaises(ValueError): compute(data)

    def test_feedback_uses_each_maps_sorted_banks_and_reserves_a_free_sound_slot(self):
        data = capture()
        data["assets"] = []
        layout = bytearray(21 * 12)
        struct.pack_into("<III", layout, 12, 4355, 1024, 1)
        struct.pack_into("<III", layout, 72, 3107, 512, 1)
        data["category_layout_base64"] = base64.b64encode(layout).decode()
        data["assets"] += [asset("ncs_mat_level", 1, 3, [b"damage_feedback", b"kill_feedback"]),
                           asset("ncs_mat_common", 1, 0, [b"other", b"damage_feedback"]),
                           asset("ncs_snl_common", 6, 0, [b"fire"])]
        result = compute(data)
        expected = dict(ncs_crc=result["ncs_crc"], hit_material=2, kill_material=3,
                        sound_index=2, register_sound=True)
        self.assertEqual(result["feedback_resources"], expected)
        data["assets"].append(asset("ncs_mat_death", 1, 3, [b"waypoint_dead"]))
        self.assertEqual(compute(data)["feedback_resources"]["death_material"], 3)
        data["assets"].append(asset("ncs_mat_scavenger", 1, 3, [b"scavenger_pickup"]))
        self.assertEqual(compute(data)["feedback_resources"]["scavenger_material"], 5)
        data["assets"].pop()
        data["assets"].pop()
        data["assets"][-1] = asset("ncs_snl_common", 6, 0, [b"fire", b"MP_hit_alert"])
        result = compute(data)
        self.assertFalse(result["feedback_resources"]["register_sound"])
        self.assertNotEqual(result["ncs_crc"], expected["ncs_crc"])
        data["assets"][-1] = asset("ncs_snl_common", 6, 0, [str(i).encode() for i in range(511)])
        with self.assertRaises(ValueError): compute(data)
        data["assets"][-1] = asset("ncs_snl_common", 6, 0, [])
        data["assets"][-3] = asset("ncs_mat_level", 1, 3, [b"damage_feedback"])
        with self.assertRaises(ValueError): compute(data)

    def test_only_cosmetic_camo_change_may_reuse_matching_runtime_weapon_parameters(self):
        weapon={"name":"m4_mp","index":352,"composed_name":"** 163 0 0 2683"}
        data=dict(schema_version=1,read_only=True,client_sha256=CLIENT_SHA256,ncs_crc=123,
            composed_weapon={"name":"** 163 3 0 2683","index":352,"ammo_key":163,"clip_key":352})
        self.assertEqual(weapon_ammo(json.dumps(data).encode(),123,weapon)["clip_key"],352)
        for name in ("** 164 3 0 2683","** 163 3 0 2684","** 163 3 1 2683","** 163 256 0 2683"):
            invalid=copy.deepcopy(data);invalid["composed_weapon"]["name"]=name
            with self.assertRaises(ValueError):weapon_ammo(json.dumps(invalid).encode(),123,weapon)
    def test_default_parts_keep_target_slot_order_and_reject_ambiguous_or_wrong_map_evidence(self):
        parts=[dict(kind=9,item_id=20031,name="mag_default",wire_index=9641),
               dict(kind=1,item_id=10603,name="sto_default",wire_index=2683)]
        capture=dict(schema_version=1,read_only=True,client_sha256=CLIENT_SHA256,ncs_crc=123,
                     connection_state=10,m4_default_parts=dict(weapon="m4_mp",parts=parts))
        self.assertEqual(weapon_parts(json.dumps(capture).encode(),123)["attachments"],[2683,9641])
        for key,bad in [("kind",9),("wire_index",9641),("item_id",20008),("wire_index",16383)]:
            invalid=copy.deepcopy(capture);invalid["m4_default_parts"]["parts"][1][key]=bad
            with self.assertRaises(ValueError):weapon_parts(json.dumps(invalid).encode(),123)
        with self.assertRaises(ValueError):weapon_parts(json.dumps(capture).encode(),124)
    def test_ammo_keys_are_bound_to_runtime_build_map_and_weapon(self):
        weapon={"name":"m4_mp","index":163}
        data=dict(schema_version=1,read_only=True,client_sha256=CLIENT_SHA256,ncs_crc=123,
                  weapon={**weapon,"ammo_key":163,"clip_key":163})
        result=weapon_ammo(json.dumps(data).encode(),123,weapon)
        self.assertEqual((result["clip"],result["reserve"]),(30,90))
        data["weapon"]["runtime_fields"] = {"fire_delay_msec":0,"fire_interval_msec":79}
        self.assertEqual(weapon_ammo(json.dumps(data).encode(),123,weapon)["fire_interval_msec"],79)
        for bad in [0,49,2001,79.5,True]:
            invalid=copy.deepcopy(data);invalid["weapon"]["runtime_fields"]["fire_interval_msec"]=bad
            with self.assertRaises(ValueError):weapon_ammo(json.dumps(invalid).encode(),123,weapon)
        invalid=copy.deepcopy(data);invalid["weapon"]["runtime_fields"]["fire_delay_msec"]=1
        with self.assertRaises(ValueError):weapon_ammo(json.dumps(invalid).encode(),123,weapon)
        data["weapon"]["runtime_fields"].update(reload_msec=2029,reload_empty_msec=2799,
            reload_add_msec=1399,reload_empty_add_msec=0)
        reload=weapon_ammo(json.dumps(data).encode(),123,weapon)["reload"]
        self.assertEqual(reload,{"normal_msec":2029,"empty_msec":2799,"add_msec":1399,"empty_add_msec":2799})
        invalid=copy.deepcopy(data);invalid["weapon"]["runtime_fields"]["reload_msec"]=0
        with self.assertRaises(ValueError):weapon_ammo(json.dumps(invalid).encode(),123,weapon)
        data["weapon"]["runtime_fields"].update(ads_enabled=1,ads_in_rate_bits=998445679,
            ads_out_rate_bits=998445679,ads_reload_allowed=0,ads_reload_interruptible=0,ads_reload_end_msec=349)
        aim=weapon_ammo(json.dumps(data).encode(),123,weapon)["ads"]
        self.assertAlmostEqual(aim["in_rate"],0.004,places=8)
        self.assertEqual(aim["reload_end_msec"],349)
        for key,value in [("ads_in_rate_bits",0x7fc00000),("ads_out_rate_bits",0xbf800000),
                          ("ads_reload_allowed",2),("ads_reload_end_msec",-1)]:
            invalid=copy.deepcopy(data);invalid["weapon"]["runtime_fields"][key]=value
            with self.assertRaises(ValueError):weapon_ammo(json.dumps(invalid).encode(),123,weapon)
        for key,value in [("ammo_key",1024),("clip_key",-1),("index",164),("ammo_key",1.5)]:
            invalid=copy.deepcopy(data);invalid["weapon"][key]=value
            with self.assertRaises(ValueError):weapon_ammo(json.dumps(invalid).encode(),123,weapon)
        with self.assertRaises(ValueError):weapon_ammo(json.dumps(data).encode(),124,weapon)
    def test_weapon_id_uses_deduplicated_constant_order_not_marketplace_item(self):
        data=capture()
        data["assets"] += [asset("ncs_wep_level",13,3,[b"default",b"m4_mp"]),
                           asset("ncs_wep_common",13,0,[b"default",b"another"])]
        self.assertEqual(compute(data,"m4_mp")["bootstrap_weapon"],{"name":"m4_mp","index":3})
        self.assertEqual(compute(data,"m4_mp",True)["bootstrap_weapon"],
            {"name":"m4_mp","index":4,"base_index":3,"camo":0,"composed_name":"** 3 0 0"})
        data["assets"].append(asset("ncs_mdl_hands",0,0,[b"viewhands_player_tf141",b"viewhands_tf141"]))
        self.assertEqual(compute(data,"m4_mp",True,True)["bootstrap_weapon"]["viewmodel_index"],5)
        self.assertEqual(compute(data,"m4_mp",True,True,"viewhands_player_tf141")["bootstrap_weapon"]["viewmodel_index"],4)
        with self.assertRaises(ValueError):compute(data,"missing")
        data["assets"][-3]["strings_base64"] = encoded([b"default",b"missing"])
        with self.assertRaises(ValueError):compute(data,"m4_mp")

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

    def test_explicit_second_map_never_reuses_the_first_maps_spawn(self):
        text = b'{"classname" "mp_tdm_spawn_allies_start" "origin" "10 20 30"}'
        maps = dict(schema_version=1,read_only=True,client_sha256=CLIENT_SHA256,maps=[dict(
            name="maps/mp/mp_killhouse_sh.d3dbsp",byte_count=len(text),entity_string_base64=encoded([text])[0])])
        self.assertEqual(spawn_origin(maps,"mp_killhouse_sh"),[10,20,30])
        with self.assertRaises(ValueError):spawn_origin(maps)

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
