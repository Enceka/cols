import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import materialize_map_batch as batch


class NativeBatchLifecycle(unittest.TestCase):
    def test_compass_candidate_cannot_start_a_native_match_or_restore_session(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            lab=batch.Lab(root,root/'service','owned',root/'catalog',root/'initial')
            for stage in ('native','restore'):
                with self.assertRaisesRegex(ValueError,'isolated resource collection'):
                    lab.start('mp_highrise',root/'catalog',stage,root/'probe')
            self.assertEqual(list(root.iterdir()),[])
    def test_captured_starts_are_retained_before_authority_selects_clear_points(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);template=root/'template'/'state.json';template.parent.mkdir()
            evidence={k:'captured-'+k for k in ('net-constants','entities','collision','arsenal','characters','outfits')}
            lab=batch.Lab(root,root/'service','owned',root/'catalog',root/'initial')
            calls=[]
            def execute(command,**kwargs):
                calls.append(command)
                output=Path(command[command.index('--output')+1])
                if command[1].endswith('prepare_tdm_map.py'):
                    output.write_text(json.dumps(dict(appearance_file='appearance.json',match_config=dict(spawns=[[],[]]))))
                else:
                    source=Path(command[command.index('--game-state')+1])
                    self.assertEqual(source.name,'mp_abandon_sh_mk-captured-starts.json')
                    self.assertEqual(Path(batch.read(source)['appearance_file']),(template.parent/'appearance.json').resolve())
                    output.write_text(source.read_text())
            with patch('materialize_map_batch.subprocess.run',side_effect=execute):
                result=lab.prepare_captures('mp_abandon_sh_mk',template,evidence)
            self.assertEqual(result.name,'mp_abandon_sh_mk-state.json')
            self.assertEqual(calls[1][1],'prepare-map-starts')
            self.assertEqual(batch.read(root/'mp_abandon_sh_mk-resource-paths.json'),evidence)
            self.assertTrue((root/'mp_abandon_sh_mk-captured-starts.json').exists())

    def test_complete_cached_resources_are_reprepared_without_restarting_the_client(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);cached=root/'cache';cached.mkdir()
            collision=root/'collision.json';collision.write_text(json.dumps(dict(world_membership=dict(bsp_root=0))))
            evidence=dict(entities='retained-entities',collision=str(collision))
            (cached/'mp_overgrown-resource-paths.json').write_text(json.dumps(evidence))
            lab=batch.Lab(root,root/'service','owned',root/'catalog',root/'initial')
            with patch.object(lab,'start',side_effect=AssertionError('cache must avoid startup')), \
                    patch.object(lab,'prepare_captures',side_effect=ValueError('map identity mismatch')) as prepare:
                with self.assertRaisesRegex(ValueError,'map identity mismatch'):
                    lab.collect('mp_overgrown',root/'template',cached)
            prepare.assert_called_once_with('mp_overgrown',root/'template',evidence)

    def test_cached_missing_compass_registers_only_the_candidate_and_retains_other_resources(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);cache=root/'cache';cache.mkdir()
            collision=root/'collision.json';batch.write(collision,dict(world_membership=dict(bsp_root=0)))
            characters=root/'characters.json';batch.write(characters,{})
            entities=root/'entities.json';batch.write(entities,{})
            evidence=dict(collision=str(collision),characters=str(characters),entities=str(entities),arsenal='retained')
            batch.write(cache/'mp_highrise-resource-paths.json',evidence)
            updated=dict(evidence,characters='registered-native-material')
            lab=batch.Lab(root,root/'service','owned',root/'catalog',root/'initial')
            with patch('materialize_map_batch.minimap',side_effect=ValueError('missing material')), \
                    patch.object(lab,'probe_compass',return_value=updated) as probe, \
                    patch.object(lab,'prepare_captures',return_value=root/'state') as prepare, \
                    patch.object(lab,'refresh_collision',side_effect=AssertionError('current world collision stays cached')):
                lab.collect('mp_highrise',root/'template',cache)
            probe.assert_called_once_with('mp_highrise',evidence)
            prepare.assert_called_once_with('mp_highrise',root/'template',updated)

    def test_legacy_cached_collision_refreshes_only_geometry_and_retains_other_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);cache=root/'cache';cache.mkdir()
            collision=root/'legacy.json';collision.write_text('{}')
            constants=root/'constants.json';constants.write_text('{}')
            catalog=root/'catalog.json';catalog.write_text(json.dumps(dict(entries=[])))
            evidence=dict(collision=str(collision),**{'net-constants':str(constants)},characters='retained-characters')
            (cache/'mp_nuked-resource-paths.json').write_text(json.dumps(evidence))
            lab=batch.Lab(root,root/'service','owned',catalog,root/'initial')
            with patch('materialize_map_batch.compute',return_value=dict(ncs_crc=123)), \
                    patch.object(lab,'start') as start,patch.object(lab,'enter'),patch.object(lab,'stop') as stop, \
                    patch.object(lab,'prepare_captures',return_value=root/'state.json') as prepare:
                start.return_value.capture.return_value=('fresh-collision',dict(world_membership=dict(bsp_root=0)))
                lab.collect('mp_nuked',root/'template',cache)
                start.return_value.capture.assert_called_once_with('collision')
                updated=prepare.call_args.args[2]
                self.assertEqual(updated['collision'],'fresh-collision')
                self.assertEqual(updated['characters'],'retained-characters')
                stop.assert_called_once()

    def test_harness_command_with_another_service_cannot_be_signalled(self):
        command=f'python {batch.REPO}/tools/gamehub_run.py client --service-data-dir /another --prefix-name owned'
        with patch('materialize_map_batch.subprocess.check_output',return_value=command), \
                patch('materialize_map_batch.os.kill') as kill:
            with self.assertRaisesRegex(RuntimeError,'identity mismatch'):
                batch.stop_owned_harness(123,Path('manifest'),Path('/expected'),'owned')
            kill.assert_not_called()

    def test_native_acceptance_exception_still_stops_diagnostics_and_restores_normal_session(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            catalog=root/'catalog.json';queue=root/'queue.json'
            catalog.write_text(json.dumps(dict(entries=[dict(map='mp_bog_sh',mode='war',file=str(root/'state.json'))])))
            queue.write_text(json.dumps(dict(ready_catalog=str(catalog),maps=[])))
            service=root/'client-work/isolated';service.mkdir(parents=True)
            output=root/'batch'
            argv=['materialize_map_batch.py','--queue',str(queue),'--template',str(root/'template'),
                '--inventory',str(root/'inventory'),'--service-data-dir',str(service),
                '--normal-experiment',str(root/'normal.json'),'--skip-collection','--output',str(output)]
            with patch.object(batch,'REPO',root),patch.object(sys,'argv',argv), \
                    patch.object(batch,'Lab') as lab,patch.object(batch,'NormalSession') as normal, \
                    patch.object(batch,'prepare_batch'),patch.object(batch,'verify',side_effect=[
                        dict(maps={'mp_bog_sh':dict(server_passed=True)}),RuntimeError('native startup failed')]):
                with self.assertRaisesRegex(RuntimeError,'native startup failed'):batch.main()
                normal.return_value.suspend.assert_called_once()
                lab.return_value.stop.assert_called_once()
                normal.return_value.restore.assert_called_once()

    def test_diagnostic_service_nested_under_normal_data_is_rejected_without_creating_output(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);output=root/'output'
            argv=['materialize_map_batch.py','--queue','unused','--template','unused','--inventory','unused',
                '--service-data-dir',str(root/'client-work/server-data/nested'),'--output',str(output)]
            with patch.object(batch,'REPO',root),patch.object(sys,'argv',argv):
                with self.assertRaises(SystemExit):batch.main()
            self.assertFalse(output.exists())

    def test_new_only_acceptance_does_not_repeat_existing_preparations(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);service=root/'client-work/isolated';service.mkdir(parents=True)
            catalog=root/'catalog.json';queue=root/'queue.json';output=root/'output'
            catalog.write_text(json.dumps(dict(entries=[dict(map='mp_bog_sh',mode='war',file=str(root/'bog.json'))])))
            queue.write_text(json.dumps(dict(ready_catalog=str(catalog),maps=[dict(map='mp_rust_long',status='needs_resources')])))
            argv=['materialize_map_batch.py','--queue',str(queue),'--template',str(root/'template'),
                '--inventory',str(root/'inventory'),'--service-data-dir',str(service),
                '--native-new-only','--output',str(output)]
            with patch.object(batch,'REPO',root),patch.object(sys,'argv',argv),patch.object(batch,'Lab') as lab, \
                    patch.object(batch,'prepare_batch'),patch.object(batch,'verify',side_effect=[
                        dict(maps={'mp_bog_sh':dict(server_passed=True)}),dict(targeted_resource_collection=[],
                             ready_server_passed=True,all_maps_passed=False)]) as verify:
                lab.return_value.collect.return_value=root/'rust.json'
                batch.main()
                self.assertEqual(verify.call_args.kwargs['names'],['mp_rust_long'])

    def test_conflicting_native_targets_are_rejected_before_output_creation(self):
        with tempfile.TemporaryDirectory() as directory:
            output=Path(directory)/'output'
            argv=['materialize_map_batch.py','--queue','unused','--template','unused','--inventory','unused',
                '--native-new-only','--native-maps','mp_bog_sh','--output',str(output)]
            with patch.object(sys,'argv',argv):
                with self.assertRaises(SystemExit):batch.main()
            self.assertFalse(output.exists())


if __name__=='__main__':unittest.main()
