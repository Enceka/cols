import base64
import copy
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from prepare_tdm_map import team_spawns
from prepare_game_state import CLIENT_SHA256


class TdmMapPreparation(unittest.TestCase):
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


if __name__ == "__main__":
    unittest.main()
