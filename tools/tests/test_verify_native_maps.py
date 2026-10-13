import json
from pathlib import Path
import sys
import struct
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from clientlib import REPO
from verify_native_maps import Client, prepare, run, movement_frames


class NativeMapIsolation(unittest.TestCase):
    def test_sampled_movement_never_posts_input_before_native_readiness(self):
        class SamplingClient(Client):
            def capture(self,mode,ready=None):
                return 'unused',dict(frames=[])
            def action(self,*args):
                raise AssertionError('input before readiness')
        with tempfile.TemporaryDirectory() as directory:
            client=SamplingClient(Path(directory)/'experiment.json')
            with self.assertRaisesRegex(RuntimeError,'before readiness'):
                client.sampled_movement()

    def test_continuous_movement_requires_one_living_spawn_and_excludes_replay_and_warps(self):
        def frame(x,time=1000,life=1,**extra):
            return dict(origin=[x,0,0],command_time=time,spawn_count=life,active=0,
                        client=0,pm_type=0,health=100,stable_command=True,**extra)
        self.assertEqual(movement_frames([frame(0),frame(10,1050)])['distance_units'],10)
        self.assertIsNone(movement_frames([frame(0),frame(200,1050)]))
        self.assertIsNone(movement_frames([frame(0),frame(10,1050,2)]))
        dead=frame(10,1050);dead['health']=0
        replay=frame(10,1050);replay['active']=1
        unstable=frame(10,1050);unstable['stable_command']=False
        for bad in [dead,replay,unstable]:
            self.assertIsNone(movement_frames([frame(0),bad]))
    def test_short_catalog_keeps_normal_configuration_and_resolves_resources(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            state = root / 'normal.json'
            state.write_text(json.dumps(dict(map='mp_bog_sh', mode='war',
                arsenal_file='resources/arsenal.json', collision_file='resources/collision.json',
                appearance_file='resources/appearance.json', voice_file='resources/voice.json',
                match_config=dict(score_limit=7500, time_limit_msec=600000))))
            catalog = root / 'normal-catalog.json'
            catalog.write_text(json.dumps(dict(schema_version=1,
                entries=[dict(map='mp_bog_sh', mode='war', file=state.name)])))
            original = [path.read_bytes() for path in (state, catalog)]
            prepared = prepare(catalog, root / 'diagnostic')
            diagnostic = json.loads((prepared.parent / 'mp_bog_sh.json').read_text())
            self.assertEqual(diagnostic['match_config'], dict(score_limit=1000, time_limit_msec=60000))
            for key in ('arsenal_file', 'collision_file', 'appearance_file', 'voice_file'):
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
            def __init__(self):self.samples=iter([11,11,12])
            def state(self):
                return dict(room_native_map='mp_cross_fire',room_member_count=next(self.samples))
            def click(self,*args):raise AssertionError('target map must remain selected')
        self.assertEqual(AdmittingClient().select_map('mp_cross_fire')['room_member_count'],12)

    def test_requested_map_autostart_after_arrow_finishes_selection_without_more_input(self):
        class StartingClient(Client):
            def __init__(self):self.clicked=False;self.clicks=0
            def state(self):
                return dict(room_native_map='mp_subbase_sh' if self.clicked else 'mp_cargoship',
                            room_member_count=12,connection_state=3 if self.clicked else 0)
            def click(self,*args):self.clicked=True;self.clicks+=1
        client=StartingClient()
        result=client.select_map('mp_subbase_sh','left')
        self.assertEqual(result['connection_state'],3)
        self.assertEqual(client.clicks,1)

    def test_requested_map_already_loading_does_not_send_room_inputs(self):
        class LoadingClient(Client):
            def __init__(self):pass
            def state(self):
                return dict(room_native_map='mp_subbase_sh',room_member_count=12,connection_state=6)
            def click(self,*args):raise AssertionError('loading map must not receive room inputs')
        self.assertEqual(LoadingClient().select_map('mp_subbase_sh')['connection_state'],6)

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
            def __init__(self):self.clicks=0;self.current='mp_bog_sh'
            def click(self,x,y,*args):
                if y==.91:self.clicks+=1
                elif self.clicks>=2:self.current='mp_crash_tropical' if x==.878 else 'mp_bog_sh'
            def state(self):
                return dict(connection_state=0,frontend_state=5,room_is_host=1,
                           room_member_count=12,room_native_map=self.current,ui_scoreboard_row_count=12)
            def wait(self,predicate,seconds):
                state=self.state()
                if predicate(state):return state
                raise RuntimeError('button not ready')
        client=ReturningClient()
        with patch('verify_native_maps.time.sleep'):
            returned=client.return_to_room()
        self.assertEqual(returned['room_member_count'],12)
        self.assertEqual(returned['room_native_map'],'mp_bog_sh')
        self.assertEqual(client.clicks,2)

    def test_retained_after_action_members_do_not_prove_a_returned_room(self):
        class AfterActionClient(Client):
            def __init__(self):self.buttons=0
            def click(self,x,y,*args):self.buttons+=int(y==.91)
            def state(self):
                return dict(connection_state=0,frontend_state=5,room_is_host=1,
                            room_member_count=12,room_native_map='mp_bog_sh')
        client=AfterActionClient()
        with patch('verify_native_maps.time.sleep'),self.assertRaisesRegex(RuntimeError,'controls unavailable'):
            client.return_to_room()
        self.assertEqual(client.buttons,3)

    def test_after_action_that_leaves_the_channel_recreates_a_verified_native_room(self):
        class LobbyClient(Client):
            def __init__(self):self.clicks=[];self.current='mp_bog_sh'
            def click(self,x,y,*args):
                self.clicks.append((x,y))
                if y==.769:self.current='mp_crash_tropical' if x==.878 else 'mp_bog_sh'
            def state(self):
                created=len(self.clicks)>=4
                return dict(connection_state=0,frontend_state=5,room_is_host=int(created),
                    room_member_count=12 if created else 0,room_native_map=self.current,
                    room_channel_pointer=123 if created else 0,
                    room_join_request_handler=456 if created else 0,active_inventory_count=100)
        client=LobbyClient()
        with patch('verify_native_maps.time.sleep'):
            result=client.return_to_room()
        self.assertEqual(result['room_native_map'],'mp_bog_sh')
        self.assertEqual(result['room_member_count'],12)
        self.assertTrue(client.return_recreated_room)
        self.assertNotIn((.765,.91),client.clicks)

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

    def test_target_created_without_owner_broadcast_is_announced_and_restored_before_start(self):
        class InitialClient(Client):
            def __init__(self):self.clicks=0
            def state(self):
                return dict(room_native_map='mp_storm_spring' if self.clicks!=1 else 'mp_mountain',
                            room_member_count=12 if self.clicks==2 else 1,connection_state=0)
            def click(self,*args):self.clicks+=1
        client=InitialClient()
        self.assertEqual(client.select_map('mp_storm_spring','auto')['room_member_count'],12)
        self.assertEqual(client.clicks,2)

    def test_admission_during_target_announcement_routes_from_the_changed_native_list(self):
        class RebuiltClient(Client):
            def __init__(self):self.index=0
            def state(self):
                name,count=[('mp_ambush_sh',1),('mp_cross_fire',12),
                            ('mp_crash_tropical',12),('mp_ambush_sh',12)][self.index]
                return dict(connection_state=0,room_native_map=name,room_member_count=count)
            def click(self,*args):self.index+=1
        client=RebuiltClient()
        self.assertEqual(client.select_map('mp_ambush_sh','auto')['room_native_map'],'mp_ambush_sh')
        self.assertEqual(client.index,3)


if __name__ == '__main__':
    unittest.main()
