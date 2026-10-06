import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from gamehub_run import trust_lab_certificate, restore_certificates


class TemporaryTrust(unittest.TestCase):
    def test_missing_iips_bundle_is_removed_and_root_is_restored(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "IIPS").mkdir()
            output = root / "capture"
            output.mkdir()
            (root / "cacert.pem").write_bytes(b"original")
            cert = root / "lab.pem"
            cert.write_bytes(b"lab")
            backups = {}
            trust_lab_certificate(root, cert, output, backups)
            self.assertEqual((root / "IIPS/cacert.pem").read_bytes(), b"original\nlab")
            self.assertEqual((root / "cacert.pem").read_bytes(), b"original\nlab")
            restore_certificates(backups)
            self.assertEqual((root / "cacert.pem").read_bytes(), b"original")
            self.assertFalse((root / "IIPS/cacert.pem").exists())

    def test_existing_iips_bundle_is_preserved(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "IIPS").mkdir()
            output = root / "capture"
            output.mkdir()
            (root / "cacert.pem").write_bytes(b"root")
            (root / "IIPS/cacert.pem").write_bytes(b"different original")
            cert = root / "lab.pem"
            cert.write_bytes(b"lab")
            backups = {}
            trust_lab_certificate(root, cert, output, backups)
            restore_certificates(backups)
            self.assertEqual((root / "IIPS/cacert.pem").read_bytes(), b"different original")
