import base64
import copy
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from prepare_progression import CLIENT_SHA256, prepare, runtime_rules


def table(rows):
    return dict(found=True, rows=len(rows), columns=len(rows[0]),
                cells_base64=[base64.b64encode(cell.encode()).decode()
                              for row in rows for cell in row])


def capture():
    rank = [["maxrank", "69", "version", "4"] + [""] * 14,
            ["maxprestige", "3"] + [""] * 16]
    for index in range(70):
        row = [""] * 18
        row[0], row[2], row[3], row[7], row[13] = map(str, (index, index * 2000, 2000,
                                                       (index + 1) * 2000, index + 1))
        rank.append(row)
    names = ["experience", "rank", "rankVersion", "prestige", "unlockFlagsNew"]
    definition = dict(version=19, size=8300, leafCount=5, rootType=5, rootIndex=0,
                      structs=[dict(leafCount=5, properties=[
                          dict(name=name, type=6 if index == 4 else 0, index=0,
                               leaf=index, extra4=0 if index == 4 else 32)
                          for index, name in enumerate(names)])],
                      indexedArrays=[dict(count=2, type=9, index=0)],
                      offsets=[4840, 4883, 4887, 4879, 8291, 8293])
    # The two unlock-array entries are leaves 4 and 5.
    definition["leafCount"] = definition["structs"][0]["leafCount"] = 6
    return dict(schema_version=1, read_only=True, client_sha256=CLIENT_SHA256,
                schemas={"mp/playersecuredata.def": dict(definitions=[definition])},
                tables={"mp/rankTable.csv": table(rank),
                        "mp/coWeaponPrimaryRankTable.csv": table([[str(i), str(i * 500)] for i in range(31)]),
                        "mp/coWeaponSecondaryRankTable.csv": table([[str(i), str(i * 500)] for i in range(20)]),
                        "mp/unlockFlags.csv": table([["uf_prestige_rank", "1", "1"]]),
                        "mp/coMarketplaceItems.csv": table([
                            ["10001", "m4_mp", "wea", "ast"] + [""] * 8,
                            ["10049", "m9_mp", "wea", "pst"] + [""] * 8])})


def set_cell(data, name, row, column, value):
    entry = data["tables"][name]
    entry["cells_base64"][row * entry["columns"] + column] = base64.b64encode(value.encode()).decode()


class ProgressionPreparation(unittest.TestCase):
    def test_runtime_classification_uses_native_records_instead_of_marketplace_subclass(self):
        assets = capture()
        assets["progression_items"] = dict(found=True, items=[
            dict(item_id=10001, name="m4_mp", kind=0, weapon_class=0),
            dict(item_id=10049, name="m9_mp", kind=0, weapon_class=6)])
        _, catalog = prepare(assets)
        rules = runtime_rules(assets, catalog)
        self.assertEqual([w["is_primary"] for w in rules["weapons"]], [True, False])
        self.assertTrue(catalog["native_weapon_classification_verified"])
        self.assertFalse(catalog["attachment_requirements_verified"])
        for key, bad in [("kind", 1), ("weapon_class", -1), ("name", "different"), ("item_id", 10049)]:
            wrong = copy.deepcopy(assets)
            wrong["progression_items"]["items"][0][key] = bad
            with self.subTest(key=key), self.assertRaises(ValueError):
                runtime_rules(wrong, prepare(wrong)[1])
        with self.assertRaises(ValueError):
            runtime_rules(capture(), prepare(capture())[1])

    def test_prepares_numeric_curves_and_marketplace_ids_without_claiming_unlocks(self):
        assets = capture()
        before = copy.deepcopy(assets)
        curves, catalog = prepare(assets)
        self.assertEqual([len(curves[k]) for k in ("player", "primary", "secondary")], [70, 31, 20])
        self.assertEqual(catalog["weapon_items"][0]["item_id"], 10001)
        self.assertEqual(catalog["profile_fields"]["experience"]["offset"], 4844)
        self.assertFalse(catalog["runtime_enabled"])
        self.assertFalse(catalog["attachment_requirements_verified"])
        self.assertFalse(catalog["native_notifications_verified"])
        self.assertEqual(assets, before)

    def test_rejects_other_builds_old_rank_versions_or_wrong_schema_offsets(self):
        for key, value in [("client_sha256", "old"), ("schema_version", 2), ("read_only", False)]:
            data = capture()
            data[key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                prepare(data)
        data = capture()
        set_cell(data, "mp/rankTable.csv", 0, 3, "2")
        with self.assertRaises(ValueError):
            prepare(data)
        for key, value in [("version", 18), ("offsets", [4839, 4883, 4887, 4879, 8291, 8293])]:
            data = capture()
            data["schemas"]["mp/playersecuredata.def"]["definitions"][0][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                prepare(data)

    def test_rejects_missing_reordered_nonmonotonic_and_signed_overflow_curves(self):
        for row, column, value in [(1, 0, "0"), (1, 1, "0"), (2, 1, "499"),
                                   (1, 1, "-1"), (30, 1, str(2**31))]:
            data = capture()
            set_cell(data, "mp/coWeaponPrimaryRankTable.csv", row, column, value)
            with self.subTest(value=value), self.assertRaises(ValueError):
                prepare(data)
        data = capture()
        data["tables"]["mp/coWeaponPrimaryRankTable.csv"]["cells_base64"].pop()
        with self.assertRaises(ValueError):
            prepare(data)
        for column in [3, 7, 13]:
            data = capture()
            set_cell(data, "mp/rankTable.csv", 3, column, "42")
            with self.subTest(column=column), self.assertRaises(ValueError):
                prepare(data)

    def test_rejects_duplicate_marketplace_ids_and_rank_flag_ambiguity(self):
        data = capture()
        set_cell(data, "mp/coMarketplaceItems.csv", 1, 0, "10001")
        with self.assertRaises(ValueError):
            prepare(data)
        data = capture()
        data["tables"]["mp/unlockFlags.csv"] = table([["uf_prestige_rank", "1", "1"]] * 2)
        with self.assertRaises(ValueError):
            prepare(data)


if __name__ == "__main__":
    unittest.main()
