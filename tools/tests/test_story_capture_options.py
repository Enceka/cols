import contextlib
import io
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import gamehub_run


class StoryCaptureOptions(unittest.TestCase):
    def test_story_capture_rejects_normal_service_data_and_world_configuration(self):
        base = ["gamehub_run.py", "client", "--rust-auth", "--local-auth-tickets",
                "--rust-lobby", "--local-services", "--rust-game", "--prefix-name", "story-test",
                "--story-capture-map", "coop_training_mp_cargoship"]
        for tail, message in [
            ([], "isolated service-data-dir"),
            (["--service-data-dir", str(gamehub_run.REPO / "client-work/server-data")], "isolated service data"),
            (["--service-data-dir", "/tmp/story-data", "--native-bot-rooms"], "without TDM preparations"),
        ]:
            error = io.StringIO()
            with self.subTest(tail=tail), patch.object(sys, "argv", base + tail), \
                    patch.object(gamehub_run, "engines", return_value={}), \
                    contextlib.redirect_stderr(error), self.assertRaises(SystemExit) as result:
                gamehub_run.main()
            self.assertEqual(result.exception.code, 2)
            self.assertIn(message, error.getvalue())


if __name__ == "__main__":
    unittest.main()
