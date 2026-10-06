import hashlib
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from check_capture import check_run


class CaptureIntegrity(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.path = Path(self.temp.name)
        (self.path / "run.json").write_text(json.dumps({"schema_version": 1, "mode": "tcp_observe_no_response"}))
        self.data = b"\x00\xffsynthetic"
        (self.path / "test.bin").write_bytes(self.data)
        self.events = [
            {"event": "session_start", "schema_version": 1, "transport": "tcp", "direction": "client_to_server"},
            {"event": "stream_chunk", "offset": 0, "length": 2},
            {"event": "stream_chunk", "offset": 2, "length": len(self.data) - 2},
            {"event": "session_end", "bytes": len(self.data), "chunks": 2, "reason": "peer_eof", "sha256": hashlib.sha256(self.data).hexdigest()},
        ]
        self.write_events()

    def write_events(self):
        (self.path / "test.jsonl").write_text("".join(json.dumps(e) + "\n" for e in self.events))

    def test_complete_capture_and_corrupted_bytes(self):
        self.assertTrue(check_run(self.path)["verified"])
        (self.path / "test.bin").write_bytes(b"x" + self.data[1:])
        with self.assertRaisesRegex(ValueError, "hash mismatch"):
            check_run(self.path)

    def test_incomplete_capture_is_not_reported_as_verified(self):
        self.events.pop()
        self.write_events()
        with self.assertRaisesRegex(ValueError, "unfinished capture"):
            check_run(self.path)

    def test_invalid_chunk_offset_and_orphan_files(self):
        self.events[2]["offset"] = 3
        self.write_events()
        with self.assertRaisesRegex(ValueError, "non-contiguous"):
            check_run(self.path)
        (self.path / "orphan.bin").write_bytes(b"")
        with self.assertRaisesRegex(ValueError, "orphan"):
            check_run(self.path)


if __name__ == "__main__":
    unittest.main()
