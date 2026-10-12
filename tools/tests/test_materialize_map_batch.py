import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import materialize_map_batch as batch


class NativeBatchLifecycle(unittest.TestCase):
    def test_complete_cached_resources_are_reprepared_without_restarting_the_client(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);cached=root/'cache';cached.mkdir()
            evidence=dict(entities='retained-entities',collision='retained-collision')
            (cached/'mp_overgrown-resource-paths.json').write_text(json.dumps(evidence))
            lab=batch.Lab(root,root/'service','owned',root/'catalog',root/'initial')
            with patch.object(lab,'start',side_effect=AssertionError('cache must avoid startup')), \
                    patch.object(lab,'prepare_captures',side_effect=ValueError('map identity mismatch')) as prepare:
                with self.assertRaisesRegex(ValueError,'map identity mismatch'):
                    lab.collect('mp_overgrown',root/'template',cached)
            prepare.assert_called_once_with('mp_overgrown',root/'template',evidence)

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


if __name__=='__main__':unittest.main()
