import json
from pathlib import Path
import sys
import struct
import tempfile
import unittest
from unittest.mock import patch

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

    def test_map_seen_before_bot_admission_does_not_end_the_new_selector_cycle(self):
        class JoiningClient(Client):
            def __init__(self):
                self.index=0
                self.maps=[('mp_bog_sh',1),('mp_crash_tropical',12),('mp_bog_sh',12),('mp_cross_fire',12)]

            def state(self):
                name,count=self.maps[self.index]
                return dict(room_native_map=name,room_member_count=count,ui_scoreboard_row_count=0)

            def click(self, *args):
                self.index+=1

        client=JoiningClient()
        self.assertEqual(client.select_map('mp_cross_fire')['room_native_map'],'mp_cross_fire')

    def test_target_waits_for_bot_admission_without_clicking_away(self):
        class AdmittingClient(Client):
            def __init__(self):self.samples=iter([1,1,12])
            def state(self):
                return dict(room_native_map='mp_cross_fire',room_member_count=next(self.samples))
            def click(self,*args):raise AssertionError('target map must remain selected')
        self.assertEqual(AdmittingClient().select_map('mp_cross_fire')['room_member_count'],12)

    def test_auto_route_keeps_one_direction_when_observed_order_is_incomplete(self):
        class ReorderedClient(Client):
            def __init__(self):
                self.index=0
                self.maps=['mp_bog_sh','mp_mountain','mp_crash_tropical','mp_cross_fire']
                self.arrows=[]
            def state(self):
                return dict(room_native_map=self.maps[self.index],room_member_count=12)
            def click(self,x,y):
                self.arrows.append(x)
                self.index+=1
        client=ReorderedClient()
        self.assertEqual(client.select_map('mp_cross_fire','auto')['room_native_map'],'mp_cross_fire')
        self.assertEqual(client.arrows,[.878,.878,.878])

    def test_after_action_return_waits_for_real_room_members_when_first_click_is_early(self):
        class ReturningClient(Client):
            def __init__(self):self.clicks=0
            def click(self,*args):self.clicks+=1
            def wait(self,predicate,seconds):
                state=dict(connection_state=0,frontend_state=5,room_is_host=1,
                           room_member_count=0 if self.clicks==1 else 12,ui_scoreboard_row_count=12)
                if predicate(state):return state
                raise RuntimeError('button not ready')
        client=ReturningClient()
        with patch('verify_native_maps.time.sleep'):
            returned=client.return_to_room()
        self.assertEqual(returned['room_member_count'],12)
        self.assertEqual(client.clicks,2)

    def test_loading_transition_stops_map_selector_before_more_room_inputs(self):
        class LeavingClient(Client):
            def __init__(self):self.clicks=0
            def state(self):
                return dict(room_native_map='mp_bog_sh' if self.clicks==0 else 'mp_crash_tropical',
                            room_member_count=12,connection_state=0 if self.clicks==0 else 6)
            def click(self,*args):self.clicks+=1
        client=LeavingClient()
        with self.assertRaisesRegex(RuntimeError,'left room during map selection'):
            client.select_map('mp_cross_fire')
        self.assertEqual(client.clicks,1)

    def test_auto_selector_tries_the_other_direction_at_a_disabled_endpoint(self):
        class EndpointClient(Client):
            def __init__(self):self.current='za_coliseum';self.arrows=[]
            def state(self):return dict(room_native_map=self.current,room_member_count=12)
            def click(self,x,y):
                self.arrows.append(x)
                if x==.707:self.current='mp_cross_fire'
        client=EndpointClient()
        self.assertEqual(client.select_map('mp_cross_fire','auto')['room_native_map'],'mp_cross_fire')
        self.assertEqual(client.arrows,[.878,.878,.878,.707])


if __name__ == '__main__':
    unittest.main()
