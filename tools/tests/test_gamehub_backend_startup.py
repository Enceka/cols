import tempfile
import unittest
import sys
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import gamehub_run


class BackendStartupTests(unittest.TestCase):
    def ready_captures(self, directory):
        for name in ('captures/run-1', 'auth-captures/auth-1'):
            path = directory / name
            path.mkdir(parents=True)
            (path / 'run.json').write_text('{}')
        (directory / 'tls-status.log').write_text('Rust AUTH listener ready\n')

    def test_auth_manifest_does_not_launch_game_before_map_preparation_finishes(self):
        with tempfile.TemporaryDirectory() as name:
            directory = Path(name)
            self.ready_captures(directory)
            process = SimpleNamespace(poll=lambda: None)
            polls = []

            def advance(_):
                polls.append(1)
                if len(polls) == 2:
                    with (directory / 'tls-status.log').open('a') as log:
                        log.write('experimental Rust game UDP endpoint ready\n')

            with patch.object(gamehub_run.time, 'sleep', side_effect=advance):
                gamehub_run.wait_backend_ready(directory, process, process, True, True)
            self.assertEqual(len(polls), 2)

    def test_failed_backend_and_missing_game_readiness_have_bounded_errors(self):
        with tempfile.TemporaryDirectory() as name:
            directory = Path(name)
            self.ready_captures(directory)
            process = SimpleNamespace(poll=lambda: None)
            failed = SimpleNamespace(poll=lambda: 1)
            with self.assertRaisesRegex(RuntimeError, 'AUTH backend exited'):
                gamehub_run.wait_backend_ready(directory, process, failed, True, True)
            with patch.object(gamehub_run.time, 'monotonic', side_effect=[0, 181]), \
                    self.assertRaisesRegex(TimeoutError, 'backend startup'):
                gamehub_run.wait_backend_ready(directory, process, process, True, True)

    def test_auth_only_experiment_keeps_its_original_readiness_condition(self):
        with tempfile.TemporaryDirectory() as name:
            directory = Path(name)
            self.ready_captures(directory)
            process = SimpleNamespace(poll=lambda: None)
            with patch.object(gamehub_run.time, 'sleep', side_effect=AssertionError('already ready')):
                gamehub_run.wait_backend_ready(directory, process, process, True, False)


if __name__ == '__main__':
    unittest.main()
