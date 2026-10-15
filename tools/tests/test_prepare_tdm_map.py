import base64
import copy
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from prepare_tdm_map import team_spawns, team_spawn_angles, respawn_points, preparation_text, charge_spawns
from prepare_game_state import CLIENT_SHA256


class TdmMapPreparation(unittest.TestCase):
    def test_small_charge_pool_reuses_only_own_points_and_keeps_headings_paired(self):
        capture=self.capture();record=capture['maps'][0]
        raw=base64.b64decode(record['entity_string_base64'])
        for team,x in [('axis',100),('allies',200)]:
            for y in range(5):
                raw+=('{"classname" "mp_cha_spawn_%s" "origin" "%d %d 56" "angle" "%d"}'%(team,x,y,y*45)).encode()
        record.update(byte_count=len(raw),entity_string_base64=base64.b64encode(raw).decode())
        starts,bases=charge_spawns(capture,'mp_killhouse_sh')
        self.assertEqual([len(t) for t in starts],[5,5])
        points=team_spawns(capture,'mp_killhouse_sh',mode='cha')
        angles=team_spawn_angles(capture,'mp_killhouse_sh',mode='cha')
        self.assertEqual([len(t) for t in points],[6,6])
        for team in range(2):
            self.assertEqual(points[team][5],points[team][0])
            self.assertEqual(angles[team][5],angles[team][0])
            self.assertEqual(points[team][:5],[p['origin'] for p in bases[team]])
        self.assertEqual(angles[0][4],[0,32768,0])

    def test_charge_uses_its_own_opening_and_base_groups_and_never_tdm_starts(self):
        capture=self.capture();record=capture['maps'][0]
        raw=base64.b64decode(record['entity_string_base64'])
        for team,x in [('axis',100),('allies',200)]:
            for y in range(6):
                raw+=('{"classname" "mp_cha_spawn_%s" "origin" "%d %d 56" "angle" "180"}'%(team,x,y)).encode()
                raw+=('{"classname" "mp_cha_spawn_%s_start" "origin" "%d %d 56"}'%(team,x+10,y)).encode()
        record.update(byte_count=len(raw),entity_string_base64=base64.b64encode(raw).decode())
        starts,bases=charge_spawns(capture,'mp_killhouse_sh')
        self.assertEqual([p[0]['origin'][0] for p in starts],[110,210])
        self.assertEqual([p[0]['origin'][0] for p in bases],[100,200])
        self.assertEqual(bases[0][0]['angles'],[0,32768,0])
        raw=raw.replace(b'mp_cha_spawn_axis_start',b'irrelevant').replace(b'mp_cha_spawn_allies_start',b'irrelevant')
        record.update(byte_count=len(raw),entity_string_base64=base64.b64encode(raw).decode())
        starts,bases=charge_spawns(capture,'mp_killhouse_sh');self.assertEqual(starts,bases)
        with self.assertRaises(ValueError):charge_spawns(self.capture(),'mp_killhouse_sh')
    def test_shared_respawns_are_distinct_from_opening_starts_and_keep_map_variant_and_heading(self):
        capture = self.capture()
        record = capture['maps'][0]
        raw = base64.b64decode(record['entity_string_base64'])
        raw += b'\n{"classname" "mp_tdm_spawn" "origin" "1 2 56" "angle" "180"}'
        raw += b'\n{"classname" "mp_tdm_spawn" "origin" "9 9 56" "script_specialopsname" "other_map"}'
        record.update(byte_count=len(raw), entity_string_base64=base64.b64encode(raw).decode())
        self.assertEqual(respawn_points(capture, 'mp_killhouse_sh'), [dict(origin=[1,2,56], angles=[0,32768,0])])
        bad = copy.deepcopy(capture)
        raw = raw.replace(b'1 2 56', b'NaN 2 56')
        bad['maps'][0].update(byte_count=len(raw), entity_string_base64=base64.b64encode(raw).decode())
        with self.assertRaises(ValueError): respawn_points(bad, 'mp_killhouse_sh')

    def test_preparation_text_requires_the_initialized_map_local_bank_and_rejects_ambiguous_slots(self):
        encode = lambda names: [base64.b64encode(name).decode() for name in names]
        capture = dict(initialized=1, linked=[dict(category=4, strings_base64=encode([b'OTHER', b'MP_MATCH_STARTING_IN']))])
        self.assertEqual(preparation_text(capture), 2)
        capture['initialized'] = 0
        with self.assertRaises(ValueError): preparation_text(capture)
        capture['initialized'] = 1
        capture['linked'][0]['strings_base64'] = encode([b'MP_MATCH_STARTING_IN']*2)
        with self.assertRaises(ValueError): preparation_text(capture)

    def capture(self):
        text = b"\n".join(
            ('{"classname" "mp_tdm_spawn_%s_start" "origin" "%s %s 56"}' % (team, x, y)).encode()
            for team, x in (("axis", 4000), ("allies", 3000)) for y in range(6)
        )
        return dict(schema_version=1, read_only=True, client_sha256=CLIENT_SHA256,
                    maps=[dict(name="maps/mp/mp_killhouse_sh.d3dbsp", byte_count=len(text),
                               entity_string_base64=base64.b64encode(text).decode())])

    def test_each_team_uses_its_own_captured_starts_from_the_requested_map(self):
        starts = team_spawns(self.capture(), "mp_killhouse_sh")
        self.assertEqual([len(t) for t in starts], [6, 6])
        self.assertEqual(starts[0][0], [4000, 0, 56])
        self.assertEqual(starts[1][0], [3000, 0, 56])
        with self.assertRaises(ValueError):
            team_spawns(self.capture(), "mp_afghan")

    def test_rejects_duplicate_map_resources_invalid_identity_and_coordinates(self):
        for change in ("duplicate", "client", "length", "position"):
            capture = self.capture()
            if change == "duplicate":
                capture["maps"].append(copy.deepcopy(capture["maps"][0]))
            elif change == "client":
                capture["client_sha256"] = "other"
            elif change == "length":
                capture["maps"][0]["byte_count"] += 1
            else:
                record = capture["maps"][0]
                data = base64.b64decode(record["entity_string_base64"]).replace(b"4000 0 56", b"NaN 0 56")
                record.update(byte_count=len(data), entity_string_base64=base64.b64encode(data).decode())
            with self.subTest(change=change), self.assertRaises(ValueError):
                team_spawns(capture, "mp_killhouse_sh")

    def test_current_map_variant_keeps_matching_spawns_and_excludes_other_variants(self):
        capture=self.capture()
        record=capture["maps"][0]
        source=base64.b64decode(record["entity_string_base64"])
        matching=source.replace(b'}', b' "script_specialopsname" "mp_other, mp_crash_tropical mp_more"}')
        excluded=source.replace(b'56',b'999').replace(b'}',b' "script_specialopsname" "mp_crash_tropical_extra,mp_other"}')
        data=excluded+matching
        record.update(name="maps/mp/mp_crash_tropical.d3dbsp",byte_count=len(data),entity_string_base64=base64.b64encode(data).decode())
        teams=team_spawns(capture,"mp_crash_tropical")
        self.assertEqual([len(team) for team in teams],[6,6])
        self.assertTrue(all(p[2]==56 for team in teams for p in team))

    def test_spawn_headings_keep_team_and_record_order_and_wrap_negative_angles(self):
        capture = self.capture()
        record = capture['maps'][0]
        raw = base64.b64decode(record['entity_string_base64'])
        raw = raw.replace(b'4000 0 56"}', b'4000 0 56" "angles" "0 180 0"}')
        raw = raw.replace(b'4000 1 56"}', b'4000 1 56" "angle" "-90"}')
        raw = raw.replace(b'3000 0 56"}', b'3000 0 56" "angles" "10 30 0"}')
        record.update(byte_count=len(raw), entity_string_base64=base64.b64encode(raw).decode())
        angles = team_spawn_angles(capture, 'mp_killhouse_sh')
        self.assertEqual(angles[0][:3], [[0,32768,0], [0,49152,0], [0,0,0]])
        self.assertEqual(angles[1][0], [1820,5461,0])
        self.assertEqual([len(t) for t in angles], [len(t) for t in team_spawns(capture, 'mp_killhouse_sh')])
        for value in [b'NaN 90 0', b'0 90', b'0 999999 0']:
            bad = copy.deepcopy(capture)
            source = raw.replace(b'0 180 0', value)
            bad['maps'][0].update(byte_count=len(source), entity_string_base64=base64.b64encode(source).decode())
            with self.assertRaises(ValueError): team_spawn_angles(bad, 'mp_killhouse_sh')


if __name__ == "__main__":
    unittest.main()
