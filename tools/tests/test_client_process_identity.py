import tempfile
import unittest
import sys
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from clientlib import matches_client_image
import gamehub_run


class ClientProcessIdentity(unittest.TestCase):
    def test_successful_wrapper_exit_still_waits_for_verified_detached_game(self):
        launcher = SimpleNamespace(pid=10, poll=lambda: 0)
        with patch.object(gamehub_run, 'client_pid', side_effect=[None, 20]) as resolve, \
                patch.object(gamehub_run.time, 'sleep') as sleep:
            self.assertEqual(gamehub_run.wait_client_pid(launcher, '/owned/game.exe', Path('/owned/prefix')), 20)
        self.assertEqual(resolve.call_count, 2)
        sleep.assert_called_once_with(0.25)

    def test_failed_wrapper_and_missing_game_keep_bounded_startup_errors(self):
        launcher = SimpleNamespace(pid=10, poll=lambda: 3)
        with patch.object(gamehub_run, 'client_pid', return_value=None), \
                self.assertRaisesRegex(RuntimeError, 'status 3'):
            gamehub_run.wait_client_pid(launcher, '/owned/game.exe', Path('/owned/prefix'))
        launcher.poll = lambda: 0
        with patch.object(gamehub_run, 'client_pid', return_value=None), \
                patch.object(gamehub_run.time, 'monotonic', side_effect=[0, 1, 91]), \
                patch.object(gamehub_run.time, 'sleep'), \
                self.assertRaisesRegex(RuntimeError, 'marked client process did not appear'):
            gamehub_run.wait_client_pid(launcher, '/owned/game.exe', Path('/owned/prefix'))

    def test_dos_image_requires_the_exact_executable_and_root_drive_mapping(self):
        with tempfile.TemporaryDirectory() as name:
            prefix = Path(name)
            (prefix / 'dosdevices').mkdir()
            (prefix / 'dosdevices/z:').symlink_to('/')
            executable = '/owned/CODOL/codoMP_client_shipRetail.exe'
            dos = 'Z:' + executable.replace('/', '\\')
            self.assertTrue(matches_client_image(executable, executable, prefix))
            self.assertTrue(matches_client_image(dos, executable, prefix))
            self.assertFalse(matches_client_image(dos.replace('owned', 'foreign'), executable, prefix))
            self.assertFalse(matches_client_image('start.exe', executable, prefix))
            (prefix / 'dosdevices/z:').unlink()
            (prefix / 'dosdevices/z:').symlink_to('/other-root')
            self.assertFalse(matches_client_image(dos, executable, prefix))

    def test_wrapper_cannot_adopt_a_game_from_another_prefix(self):
        with tempfile.TemporaryDirectory() as name:
            prefix = Path(name)
            (prefix / 'dosdevices').mkdir()
            (prefix / 'dosdevices/z:').symlink_to('/')
            executable = '/owned/CODOL/codoMP_client_shipRetail.exe'
            dos = 'Z:' + executable.replace('/', '\\')

            def command(args, **kwargs):
                if args[0] == 'lsof':
                    return SimpleNamespace(returncode=0, stdout='n/foreign/drive_c/windows/file\n')
                if args[1] == 'e':
                    return SimpleNamespace(returncode=0, stdout='app WINEPREFIX=/foreign ')
                return SimpleNamespace(returncode=0, stdout='start.exe\n')

            with patch.object(gamehub_run.subprocess, 'run', side_effect=command), \
                    patch.object(gamehub_run.subprocess, 'check_output', return_value='20 ' + dos + '\n'):
                self.assertIsNone(gamehub_run.client_pid(10, executable, prefix))

            def own_command(args, **kwargs):
                if args[0] == 'lsof':
                    return SimpleNamespace(returncode=0, stdout='n' + str(prefix.resolve()) + '/drive_c/windows/file\n')
                return command(args, **kwargs)

            with patch.object(gamehub_run.subprocess, 'run', side_effect=own_command), \
                    patch.object(gamehub_run.subprocess, 'check_output', return_value='20 ' + dos + '\n'):
                self.assertEqual(gamehub_run.client_pid(10, executable, prefix), 20)
