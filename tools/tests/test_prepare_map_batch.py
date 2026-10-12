import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from prepare_map_batch import prepare,verify
from prepare_game_state import CLIENT_SHA256


class MapBatch(unittest.TestCase):
    def test_four_player_quick_map_uses_its_own_catalog_and_fresh_owned_session(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);_,catalog,inventory=self.inputs(root)
            arena=root/'arena.json'
            arena.write_text(json.dumps(dict(map='za_coliseum',mode='war',client_sha256=CLIENT_SHA256,
                ncs_crc=123,match_config=dict(player_count=4,score_limit=7500,time_limit_msec=600000))))
            catalog.write_text(json.dumps(dict(schema_version=1,entries=[dict(map='za_coliseum',mode='war',file=arena.name)])))
            prepare(catalog,inventory,root/'batch')
            service=root/'service';(service/'publisher').mkdir(parents=True)
            (service/'publisher/mp_playlists_shipRetail_release.info').write_text('version 9\n')
            class Lab:
                def __init__(self):self.service=service;self.starts=[];self.stops=0
                def start(self,name,path,stage):
                    self.starts.append((name,path,stage))
                    return type('Session',(),dict(experiment=root/'experiment.json'))()
                def stop(self):self.stops+=1
            lab=Lab()
            def server(command,**kwargs):
                Path(command[command.index('--output')+1]).write_text(json.dumps(dict(passed=True,
                    maps=[dict(map='za_coliseum',passed=True,errors=[])])))
            def native(path,experiment,output,names,direction,**kwargs):
                self.assertEqual(kwargs,dict(quick_playlist=0))
                data=json.loads(path.read_text())
                self.assertEqual(len(data['entries']),1)
                self.assertEqual(data['rotations'],[dict(version=9,playlist=0,maps=[dict(map='za_coliseum',mode='war')])])
                state=json.loads(Path(data['entries'][0]['file']).read_text())
                self.assertEqual(state['match_config']['player_count'],4)
            with patch('prepare_map_batch.subprocess.run',side_effect=server), \
                    patch('prepare_map_batch.native_run',side_effect=native):
                result=verify(root/'batch/queue.json',root/'result',native_lab=lab)
            self.assertTrue(result['maps']['za_coliseum']['native_passed'])
            self.assertEqual(lab.stops,2)

    def inputs(self, root):
        state=root/'bog.json'
        state.write_text(json.dumps(dict(map='mp_bog_sh',mode='war',client_sha256=CLIENT_SHA256,
            ncs_crc=123,match_config=dict(score_limit=7500,time_limit_msec=600000))))
        catalog=root/'catalog.json'
        catalog.write_text(json.dumps(dict(schema_version=1,entries=[dict(map='mp_bog_sh',mode='war',file='bog.json')])))
        inventory=root/'inventory.json'
        inventory.write_text(json.dumps(dict(client_sha256=CLIENT_SHA256,read_only=True,
            maps=[dict(name='mp_estate_tropical',observed_fastfiles={'native-map.ff':['native.ifs']}),
                  dict(name='mp_bloc_sh',observed_fastfiles={'native-map.ff':[]})])))
        return state,catalog,inventory

    def test_all_native_tasks_are_configured_without_borrowing_missing_map_resources(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);state,catalog,inventory=self.inputs(root)
            original=state.read_bytes()
            queue=prepare(catalog,inventory,root/'batch')
            self.assertEqual(len(queue['maps']),26)
            by_name={m['map']:m for m in queue['maps']}
            self.assertEqual(by_name['mp_bog_sh']['status'],'ready')
            self.assertEqual(by_name['mp_estate_tropical']['status'],'needs_resources')
            self.assertEqual(by_name['mp_bloc_sh']['status'],'needs_resource_audit')
            self.assertEqual(by_name['za_coliseum']['maximum_players'],4)
            self.assertEqual(by_name['za_coliseum']['bot_count'],3)
            self.assertNotIn('ncs_crc',by_name['mp_estate_tropical'])
            self.assertNotIn('preparation',by_name['mp_estate_tropical'])
            self.assertEqual(state.read_bytes(),original)
            diagnostic=json.loads((root/'batch/diagnostic/mp_bog_sh.json').read_text())
            self.assertEqual(diagnostic['match_config'],dict(score_limit=1000,time_limit_msec=60000))

    def test_changed_preparation_is_rejected_before_reusing_old_batch_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);state,catalog,inventory=self.inputs(root)
            prepare(catalog,inventory,root/'batch')
            state.write_text(state.read_text()+' ')
            with self.assertRaisesRegex(ValueError,'preparation changed'):
                verify(root/'batch/queue.json',root/'result')

    def test_catalog_import_failure_is_isolated_and_final_status_uses_individual_results(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);_,catalog,inventory=self.inputs(root)
            prepare(catalog,inventory,root/'batch')
            calls=[]
            def server(command,**kwargs):
                calls.append(command)
                report=Path(command[command.index('--output')+1])
                if len(calls)>1:
                    report.write_text(json.dumps(dict(passed=True,maps=[
                        dict(map='mp_bog_sh',passed=True,errors=[])])))
                from subprocess import CompletedProcess
                return CompletedProcess(command,1 if len(calls)==1 else 0)
            with patch('prepare_map_batch.subprocess.run',side_effect=server):
                result=verify(root/'batch/queue.json',root/'result')
            self.assertTrue(result['ready_server_passed'])
            self.assertFalse(result['all_maps_passed'])
            self.assertEqual(len(calls),2)

    def test_native_failure_collects_state_and_restarts_only_failed_session(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);_,catalog,inventory=self.inputs(root)
            other=root/'crash.json'
            other.write_text(json.dumps(dict(map='mp_crash_tropical',mode='war',client_sha256=CLIENT_SHA256,
                ncs_crc=456,match_config=dict(score_limit=7500,time_limit_msec=600000))))
            data=json.loads(catalog.read_text())
            data['entries'].append(dict(map='mp_crash_tropical',mode='war',file='crash.json'))
            catalog.write_text(json.dumps(data));prepare(catalog,inventory,root/'batch')
            class Lab:
                def __init__(self):self.starts=[];self.stops=0
                def start(self,name,*args):
                    self.starts.append(name)
                    return type('Session',(),dict(experiment=root/(name+'-experiment.json')))()
                def stop(self):self.stops+=1
            lab=Lab()
            def server(command,**kwargs):
                report=Path(command[command.index('--output')+1])
                report.write_text(json.dumps(dict(passed=True,maps=[dict(map=n,passed=True,errors=[])
                    for n in ['mp_bog_sh','mp_crash_tropical']])))
            def native(*args):
                if args[3]==['mp_bog_sh']:raise RuntimeError('unknown frontend')
            with patch('prepare_map_batch.subprocess.run',side_effect=server), \
                    patch('prepare_map_batch.native_run',side_effect=native), \
                    patch('prepare_map_batch.Client') as client:
                client.return_value.state.return_value=dict(connection_state=0,room_member_count=0)
                result=verify(root/'batch/queue.json',root/'result',native_lab=lab)
                client.return_value.click.assert_not_called()
            self.assertEqual(lab.starts,['mp_bog_sh','mp_crash_tropical'])
            self.assertEqual(lab.stops,1)
            self.assertFalse(result['maps']['mp_bog_sh']['native_passed'])
            self.assertTrue(result['maps']['mp_crash_tropical']['native_passed'])
            self.assertTrue((root/'result/mp_bog_sh-failure-state.json').exists())

    def test_previous_map_frontend_failure_gets_one_fresh_retry_and_retains_initial_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);_,catalog,inventory=self.inputs(root)
            prepare(catalog,inventory,root/'batch')
            class Lab:
                def __init__(self):self.starts=[];self.stops=0
                def start(self,name,*args):
                    self.starts.append(name)
                    return type('Session',(),dict(experiment=root/(name+'-experiment.json')))()
                def stop(self):self.stops+=1
            lab=Lab()
            def server(command,**kwargs):
                report=Path(command[command.index('--output')+1])
                report.write_text(json.dumps(dict(passed=True,maps=[dict(map='mp_bog_sh',passed=True,errors=[])])))
            with patch('prepare_map_batch.subprocess.run',side_effect=server), \
                    patch('prepare_map_batch.native_run',side_effect=[RuntimeError('room timeout'),None]), \
                    patch('prepare_map_batch.Client') as client:
                client.return_value.state.return_value=dict(connection_state=0,frontend_state=5,
                    room_member_count=0,room_native_map='mp_estate_tropical')
                result=verify(root/'batch/queue.json',root/'result',native_lab=lab)
            self.assertEqual(lab.starts,['mp_bog_sh','mp_bog_sh-retry'])
            self.assertTrue(result['maps']['mp_bog_sh']['native_passed'])
            self.assertEqual(result['maps']['mp_bog_sh']['native_retries'],1)
            self.assertIsNone(result['native_error'])
            self.assertEqual(result['targeted_resource_collection'],[])
            self.assertTrue((root/'result/mp_bog_sh-failure-diagnostics.json').exists())


if __name__=='__main__':unittest.main()
