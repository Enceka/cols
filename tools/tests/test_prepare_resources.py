import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import prepare_resources as resources
from verify_native_maps import Client


class LocalResourcePreparation(unittest.TestCase):
    def test_capture_plan_uses_its_own_directory_and_requires_target_identity(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp).resolve()
            plan = root/'capture-plan.json'
            value = dict(schema_version=1, client_sha256=resources.CLIENT_SHA256,
                         assets='assets.json', maps=[])
            plan.write_text(json.dumps(value))
            result, base = resources.plan(root, 'war')
            self.assertEqual(result, value)
            self.assertEqual(resources.resolve(base, result['assets']), root/'assets.json')
            value['client_sha256'] = '0'*64
            plan.write_text(json.dumps(value))
            with self.assertRaisesRegex(ValueError, 'identity'):
                resources.plan(root, 'war')

    def test_existing_cache_reuses_map_capture_paths_without_reading_accounts(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp).resolve()
            (root/'assets.json').write_text('{}')
            (root/'mp_afghan-resource-paths.json').write_text(json.dumps({
                'collision':'world.json', 'entities':'entities.json', 'experiment':'account.json'}))
            result, base = resources.plan(root, 'war')
            self.assertEqual(result['maps'][0]['captures'], {
                'collision':str(root/'world.json'), 'entities':str(root/'entities.json')})
            self.assertEqual(base, root)

    def test_requested_maps_must_have_captures_in_the_selected_mode(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp).resolve()
            value=dict(schema_version=1,client_sha256=resources.CLIENT_SHA256,
                assets='assets.json',maps=[dict(map='mp_nuked',mode='cha',captures={})])
            (root/'capture-plan.json').write_text(json.dumps(value))
            (root/'assets.json').write_text(json.dumps(dict(read_only=True,client_sha256=resources.CLIENT_SHA256)))
            with self.assertRaisesRegex(ValueError, 'selected mode'):
                resources.prepare_captures(root, root/'out', Path('/server'), 'war', ['mp_nuked'])
            self.assertFalse((root/'out').exists())

    def test_failed_helper_propagates_its_log_and_probe_paths_are_isolated(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp).resolve()
            with self.assertRaisesRegex(RuntimeError, 'missing local resource'):
                resources.command([sys.executable,'-c',"raise SystemExit('missing local resource')"],root/'log')
            client=Client(root/'experiment.json',root/'probe')
            completed=subprocess.CompletedProcess([],0,json.dumps(dict(state={'connection_state':9})), '')
            with patch('verify_native_maps.subprocess.run',return_value=completed) as run:
                self.assertEqual(client.state()['connection_state'],9)
                args=run.call_args.args[0]
                self.assertEqual(args[args.index('--probe-dir')+1],str(root/'probe'))

class OwnedNativeCollection(unittest.TestCase):
    def test_native_session_uses_a_stable_private_handle_and_no_playable_catalog(self):
        import resource_capture
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp).resolve()
            prefix=root/'prefix';prefix.mkdir()
            (prefix/'.codol-prefix.json').write_text(json.dumps(dict(engine_id='10000033')))
            partial=root/'sessions/client-partial';partial.mkdir(parents=True)
            (partial/'experiment.json').write_text('{')
            session=resource_capture.Session(root,root/'client',prefix,root/'service',root/'server',root/'tls')
            with patch('resource_capture.subprocess.Popen') as popen:
                popen.return_value.poll.return_value=1
                with self.assertRaisesRegex(RuntimeError,'startup failed'):
                    session.start('mp_afghan')
                argv=popen.call_args.args[0]
                self.assertIn('--resource-capture-only',argv)
                self.assertNotIn('--native-bot-rooms',argv)
                self.assertNotIn('--game-map-catalog',argv)
                self.assertEqual(argv[argv.index('--handle')+1],session.handle)
                self.assertRegex(session.handle,r'^res[a-f0-9]{12}$')
                self.assertEqual(argv[argv.index('--output-root')+1],str(root/'sessions'))

    def test_failed_certificate_cleanup_cannot_be_reported_as_complete(self):
        import resource_capture
        from unittest.mock import Mock
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp).resolve()
            session=resource_capture.Session(root,root/'client',root/'prefix',root/'service',root/'server',root/'tls')
            session.process=Mock();session.process.poll.return_value=0
            session.experiment=root/'experiment.json'
            session.experiment.write_text(json.dumps(dict(finished_unix=1,lab_certificate_restored=False)))
            with self.assertRaisesRegex(RuntimeError,'cleanup incomplete'):
                session.stop()

    def test_external_checkout_can_clone_an_idle_marked_prefix_next_to_the_source(self):
        import resource_capture
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp).resolve()
            source=root/'original/client-work/CODOL';source.mkdir(parents=True)
            prefix=source.parent/'wine10-owned';prefix.mkdir()
            (prefix/'.codol-prefix.json').write_text(json.dumps(dict(engine_id='10000033')))
            with patch('resource_capture.REPO',root/'isolated-checkout'):
                self.assertEqual(resource_capture.choose_prefix(source),prefix)

    def test_bootstrap_rebuild_uses_a_new_file_and_keeps_the_source_unchanged(self):
        import hashlib
        import resource_capture
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp).resolve();source=root/'source';client=root/'client'
            source.mkdir();client.mkdir();data=b'fixed local bootstrap'
            (source/'init.ifs').write_bytes(data);(client/'init.ifs').write_bytes(data)
            evidence=('init.ifs',len(data),hashlib.sha256(data).hexdigest())
            with patch('resource_capture.BOOTSTRAP_FILE',evidence):
                resource_capture.isolate_bootstrap_file(source,client)
            self.assertEqual((source/'init.ifs').read_bytes(),data)
            self.assertEqual((client/'init.ifs').read_bytes(),data)
            self.assertEqual((client/'init.codol-before').read_bytes(),data)
            self.assertNotEqual((source/'init.ifs').stat().st_ino,(client/'init.ifs').stat().st_ino)

    def test_unknown_or_linked_bootstrap_cannot_replace_the_cloned_file(self):
        import resource_capture
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp).resolve();source=root/'source';client=root/'client'
            source.mkdir();client.mkdir();(source/'init.ifs').write_bytes(b'wrong')
            (client/'init.ifs').write_bytes(b'keep')
            with patch('resource_capture.BOOTSTRAP_FILE',('init.ifs',5,'0'*64)):
                with self.assertRaisesRegex(ValueError,'校验和'):
                    resource_capture.isolate_bootstrap_file(source,client)
                (source/'init.ifs').unlink();(source/'init.ifs').symlink_to(root/'missing')
                with self.assertRaisesRegex(ValueError,'资源链接'):
                    resource_capture.isolate_bootstrap_file(source,client)
            self.assertEqual((client/'init.ifs').read_bytes(),b'keep')
            self.assertFalse((client/'init.codol-new').exists())

    def test_shared_discovery_requires_the_explicit_mode_and_valid_wire_reply(self):
        import resource_capture
        with patch('resource_capture.socket.socket') as create:
            probe=create.return_value.__enter__.return_value
            probe.getsockname.return_value=('127.0.0.1',30000)
            data=bytes([31,2,0,127,0,0,1])+int(30000).to_bytes(2,'little')+b'0'*6
            probe.recvfrom.side_effect=[(data,('127.0.0.1',3074)),(data,('127.0.0.1',3078))]
            resource_capture.check_stun(True)
            self.assertEqual(probe.sendto.call_count,2)
        with patch('resource_capture.socket.socket') as create:
            probe=create.return_value.__enter__.return_value
            probe.recvfrom.return_value=(b'unknown',('127.0.0.1',3074))
            with self.assertRaisesRegex(ValueError,'协议校验'):
                resource_capture.check_stun(True)
        with patch('resource_capture.socket.socket') as create:
            probe=create.return_value.__enter__.return_value
            probe.bind.side_effect=OSError('busy')
            with self.assertRaisesRegex(ValueError,'端口被占用'):
                resource_capture.check_stun(False)
            probe.sendto.assert_not_called()


if __name__=='__main__':unittest.main()
