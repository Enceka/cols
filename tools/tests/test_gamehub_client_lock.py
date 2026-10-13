import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import gamehub_run


class SharedClientLock(unittest.TestCase):
    def test_different_prefixes_cannot_share_configs_or_certificates(self):
        with tempfile.TemporaryDirectory() as folder, \
                patch.object(gamehub_run, 'existing_client_owner', return_value=None):
            work = Path(folder)
            for name in ('dwauth.ini', 'cacert.pem'):
                (work / name).write_bytes(b'normal client data')
            first = gamehub_run.acquire_client_lock(work, work / 'wine10')
            try:
                before = (work / '.codol-client-lock').read_bytes()
                with self.assertRaisesRegex(RuntimeError, 'already in use'):
                    gamehub_run.acquire_client_lock(work, work / 'wine11')
                self.assertEqual((work / '.codol-client-lock').read_bytes(), before)
                for name in ('dwauth.ini', 'cacert.pem'):
                    self.assertEqual((work / name).read_bytes(), b'normal client data')
            finally:
                first.close()
            # A stale metadata file is not a live lock and does not block reuse.
            with gamehub_run.acquire_client_lock(work, work / 'wine11'):
                owner = json.loads((work / '.codol-client-lock').read_text())
                self.assertEqual(owner['prefix'], str(work / 'wine11'))

    def test_process_exit_releases_the_lock(self):
        with tempfile.TemporaryDirectory() as folder:
            command = [sys.executable, '-c',
                       'import fcntl,sys; f=open(sys.argv[1],"a+"); '
                       'fcntl.flock(f,fcntl.LOCK_EX); print("locked",flush=True); '
                       'sys.stdin.read()', str(Path(folder) / '.codol-client-lock')]
            child = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
            try:
                self.assertEqual(child.stdout.readline().strip(), 'locked')
                with self.assertRaisesRegex(RuntimeError, 'already in use'):
                    gamehub_run.acquire_client_lock(Path(folder), Path(folder) / 'prefix')
            finally:
                child.communicate('', timeout=5)
            with patch.object(gamehub_run, 'existing_client_owner', return_value=None):
                with gamehub_run.acquire_client_lock(Path(folder), Path(folder) / 'prefix'):
                    pass

    def test_legacy_harness_is_rejected_and_failed_acquisition_releases_lock(self):
        with tempfile.TemporaryDirectory() as folder:
            work = Path(folder)
            with patch.object(gamehub_run, 'existing_client_owner', return_value=42):
                with self.assertRaisesRegex(RuntimeError, 'process 42'):
                    gamehub_run.acquire_client_lock(work, work / 'prefix')
            with patch.object(gamehub_run, 'existing_client_owner', return_value=None):
                with gamehub_run.acquire_client_lock(work, work / 'prefix'):
                    pass

    def test_process_scan_ignores_itself_and_shells_and_recognizes_native_images(self):
        work = Path('/owned/CODOL')
        harness = str(Path(gamehub_run.__file__).resolve()) + ' client --prefix-name voice'
        rows = f'{os.getpid()} Python {harness}\n20 /bin/zsh -lc Python {harness}\n'
        with patch.object(gamehub_run.subprocess, 'check_output', return_value=rows), \
                patch.object(gamehub_run.subprocess, 'run', return_value=SimpleNamespace(
                    returncode=0, stdout='/bin/zsh\n')):
            self.assertIsNone(gamehub_run.existing_client_owner(work))
        for image in ('/owned/CODOL/codoMP_client_shipRetail.exe',
                      'Z:\\owned\\CODOL\\codoMP_client_shipRetail.exe'):
            with patch.object(gamehub_run.subprocess, 'check_output', return_value=f'30 {image} -q 0\n'):
                self.assertEqual(gamehub_run.existing_client_owner(work), 30)
        with patch.object(gamehub_run.subprocess, 'check_output', return_value=f'40 /usr/bin/python3 {harness}\n'), \
                patch.object(gamehub_run.subprocess, 'run', return_value=SimpleNamespace(
                    returncode=0, stdout='/usr/bin/python3\n')):
            self.assertEqual(gamehub_run.existing_client_owner(work), 40)
