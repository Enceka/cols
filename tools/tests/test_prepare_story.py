import base64
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from prepare_story import CLIENT_SHA256, FIRST_MAP, prepare, progress


class StoryPreparation(unittest.TestCase):
    def capture(self):
        cells = []
        for name, native_id, flags in [(FIRST_MAP, 17, ["1", "0", "1"]),
                                      ("coop_defense_mp_suburbs", 28, ["1", "1", "0"])]:
            row = [""] * 37
            row[:4] = flags + [str(native_id)]
            row[4], row[8], row[9] = name, "coop", "coop"
            row[10:14] = ["1", "1", "1", "2"]
            row[33] = "101"
            cells.extend(base64.b64encode(c.encode()).decode() for c in row)
        return dict(schema_version=1, read_only=True, client_sha256=CLIENT_SHA256,
                    build_version_column=2, tables={"mp/coMapList.csv":
                    dict(found=True, rows=2, columns=37, cells_base64=cells)})

    def test_current_version_column_controls_admission_and_retains_closed_metadata(self):
        result = prepare(self.capture())
        self.assertEqual([m["enabled_by_version"] for m in result["maps"]], [True, False])
        self.assertEqual(result["maps"][0]["level"], 2)
        self.assertFalse(result["resource_preparation_claimed"])
        self.assertFalse(result["gameplay_verified"])

    def test_metadata_cannot_enable_wrong_version_capacity_or_ambiguous_identity(self):
        for column, value in [(2, "0"), (4, "../coop_training_mp_cargoship"), (9, "war"),
                              (11, "12"), (40, "17"), (41, FIRST_MAP), (33, "0")]:
            capture = self.capture()
            capture["tables"]["mp/coMapList.csv"]["cells_base64"][column] = base64.b64encode(value.encode()).decode()
            with self.subTest(column=column), self.assertRaises(ValueError):
                prepare(capture)
        for key, value in [("read_only", False), ("client_sha256", "other"),
                           ("build_version_column", 1)]:
            capture = self.capture()
            capture[key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                prepare(capture)

    def test_completion_mapping_ignores_table_headers_and_requires_the_native_flags(self):
        definition = dict(version=19, size=8192, leafCount=4096, rootType=5, rootIndex=0,
                          structs=[dict(leafCount=4096, properties=[dict(name="unlockFlags",
                          type=6, index=0, leaf=0, extra4=0),dict(name="unlockFlagsNew",
                          type=6, index=1, leaf=2048, extra4=0)])],
                          indexedArrays=[dict(count=2048, type=2, index=0),dict(count=2048,type=9,index=0)],
                          offsets=list(range(64, 2112))+list(range(4096,8192,2)))
        values = ["flag", "", "", "cf_ch_" + FIRST_MAP, "150", "",
                  "cf_ch_coop_defense_mp_suburbs", "160", ""]
        assets = dict(schema_version=1, read_only=True, client_sha256=CLIENT_SHA256,
                      schemas={"mp/playersecuredata.def": dict(definitions=[definition])},
                      tables={"mp/unlockFlags.csv": dict(found=True, rows=3, columns=3,
                      cells_base64=[base64.b64encode(v.encode()).decode() for v in values])})
        catalog = prepare(self.capture())
        progress(assets, catalog)
        self.assertEqual(catalog["maps"][0]["completion_flag_id"], 150)
        self.assertEqual(catalog["maps"][0]["completion_field"]["type"], 9)
        self.assertEqual(catalog["maps"][0]["completion_minimum"], 1)
        self.assertEqual(catalog["maps"][0]["legacy_completion_field"]["bit"], 6)
        assets["tables"]["mp/unlockFlags.csv"]["cells_base64"][3] = base64.b64encode(b"missing").decode()
        with self.assertRaises(ValueError):
            progress(assets, prepare(self.capture()))


if __name__ == "__main__":
    unittest.main()
