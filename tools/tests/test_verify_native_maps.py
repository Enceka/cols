import json
from pathlib import Path
import sys
import struct
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from clientlib import REPO
from verify_native_maps import Client, prepare, run


class NativeMapIsolation(unittest.TestCase):
    def test_short_catalog_keeps_normal_configuration_and_resolves_resources(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            state = root / 'normal.json'
            state.write_text(json.dumps(dict(map='mp_bog_sh', mode='war',
                arsenal_file='resources/arsenal.json', collision_file='resources/collision.json',
                appearance_file='resources/appearance.json',
                match_config=dict(score_limit=7500, time_limit_msec=600000))))
            catalog = root / 'normal-catalog.json'
            catalog.write_text(json.dumps(dict(schema_version=1,
                entries=[dict(map='mp_bog_sh', mode='war', file=state.name)])))
            original = [path.read_bytes() for path in (state, catalog)]
            prepared = prepare(catalog, root / 'diagnostic')
            diagnostic = json.loads((prepared.parent / 'mp_bog_sh.json').read_text())
            self.assertEqual(diagnostic['match_config'], dict(score_limit=1000, time_limit_msec=60000))
            for key in ('arsenal_file', 'collision_file', 'appearance_file'):
                self.assertEqual(Path(diagnostic[key]).parent, (root / 'resources').resolve())
            self.assertEqual([path.read_bytes() for path in (state, catalog)], original)
            with self.assertRaises(FileExistsError):
                prepare(catalog, root / 'diagnostic')

    def test_normal_service_data_is_rejected_before_any_client_input_or_report(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            catalog = root / 'catalog.json'
            experiment = root / 'experiment.json'
            output = root / 'report.json'
            experiment.write_text(json.dumps(dict(tls_argv=[
                '--game-map-catalog', str(catalog), '--service-data-dir', str(REPO / 'client-work/server-data')
            ])))
            with self.assertRaisesRegex(ValueError, 'isolated service data'):
                run(catalog, experiment, output)
            self.assertFalse(output.exists())

    def movement_client(self, samples):
        class SampledClient(Client):
            def __init__(self):
                self.samples=iter(samples)
                self.keys=[]

            def state(self):
                x,life=next(self.samples)
                return dict(connection_state=10,predicted_pm_type=0,predicted_health=100,
                    predicted_spawn_count=life,
                    **{'predicted_origin_'+axis+'_bits':struct.unpack('<I',struct.pack('<f',value))[0]
                       for axis,value in zip('xyz',(x,0,0))})

            def action(self, kind, key, seconds):
                self.keys.append(key)

        return SampledClient()

    def test_wall_facing_spawn_can_pass_using_observed_sideways_movement(self):
        client=self.movement_client([(0,1),(0,1),(0,1),(48,1)])
        distance,attempts=client.movement()
        self.assertEqual(client.keys,['w','d'])
        self.assertEqual(distance,48)
        self.assertEqual(attempts[0]['distance_units'],0)

    def test_respawn_teleports_cannot_pass_native_movement_check(self):
        client=self.movement_client([(0,1),(200,2),(200,2),(400,3),(400,3),(600,4)])
        with self.assertRaisesRegex(RuntimeError,'same life'):
            client.movement()


if __name__ == '__main__':
    unittest.main()
