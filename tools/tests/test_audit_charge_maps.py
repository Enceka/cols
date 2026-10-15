import base64
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from audit_charge_maps import audit
from prepare_game_state import CLIENT_SHA256


class ChargePoolAudit(unittest.TestCase):
    def source(self, directory, classes):
        text = '\n'.join('{"classname" "%s" "origin" "%d 0 56"}' % (name, slot)
                         for name, count in classes.items() for slot in range(count)).encode()
        capture = dict(schema_version=1, read_only=True, client_sha256=CLIENT_SHA256,
                       maps=[dict(name='maps/mp/mp_nuked.d3dbsp', byte_count=len(text),
                                  entity_string_base64=base64.b64encode(text).decode())])
        path = Path(directory) / 'entities.json'
        raw = json.dumps(capture).encode()
        path.write_bytes(raw)
        return {'mp_nuked': dict(entity_capture=str(path), entity_sha256=hashlib.sha256(raw).hexdigest())}

    def test_tactical_starts_do_not_make_an_assault_map_eligible(self):
        with tempfile.TemporaryDirectory() as directory:
            source = self.source(directory, {'mp_tdm_spawn_axis_start': 9, 'mp_tdm_spawn_allies_start': 9})
            result = audit(source, dict(entries=[]))
            row = next(row for row in result['maps'] if row['map'] == 'mp_nuked')
            self.assertEqual(row['status'], 'excluded_no_native_pools')
            with self.assertRaisesRegex(ValueError, 'catalog includes map without complete native assault pools'):
                audit(source, dict(entries=[dict(map='mp_nuked', mode='cha')]))

    def test_both_base_pools_can_supply_opening_without_separate_start_groups(self):
        with tempfile.TemporaryDirectory() as directory:
            source = self.source(directory, {'mp_cha_spawn_axis': 9, 'mp_cha_spawn_allies': 8})
            result = audit(source, dict(entries=[dict(map='mp_nuked', mode='cha')]))
            row = next(row for row in result['maps'] if row['map'] == 'mp_nuked')
            self.assertEqual((row['status'], row['opening_counts'], row['base_counts']),
                             ('prepared', [9, 8], [9, 8]))
            self.assertFalse(result['native_gameplay_claimed'])

    def test_opening_groups_without_both_base_pools_are_excluded(self):
        with tempfile.TemporaryDirectory() as directory:
            source = self.source(directory, {'mp_cha_spawn_axis_start': 9, 'mp_cha_spawn_allies_start': 9,
                                             'mp_cha_spawn_axis': 9})
            result = audit(source, dict(entries=[]))
            row = next(row for row in result['maps'] if row['map'] == 'mp_nuked')
            self.assertEqual(row['status'], 'excluded_no_native_pools')

    def test_five_native_points_are_a_real_pool_even_for_a_twelve_player_room(self):
        with tempfile.TemporaryDirectory() as directory:
            source = self.source(directory, {'mp_cha_spawn_axis': 5, 'mp_cha_spawn_allies': 5})
            result = audit(source, dict(entries=[]))
            row = next(row for row in result['maps'] if row['map'] == 'mp_nuked')
            self.assertEqual((row['status'], row['opening_counts']), ('eligible_unprepared', [5, 5]))

    def test_changed_capture_is_rejected_instead_of_trusting_saved_counts(self):
        with tempfile.TemporaryDirectory() as directory:
            source = self.source(directory, {'mp_cha_spawn_axis': 9, 'mp_cha_spawn_allies': 9})
            path = Path(source['mp_nuked']['entity_capture'])
            path.write_bytes(path.read_bytes() + b'\n')
            with self.assertRaisesRegex(ValueError, 'entity source hash mismatch'):
                audit(source, dict(entries=[]))


if __name__ == '__main__':
    unittest.main()
