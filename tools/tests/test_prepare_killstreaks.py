import base64
import copy
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from prepare_killstreaks import MARKET, STATS, catalog, class_layout, identity, native


def table(rows):
    return dict(found=True, rows=len(rows), columns=len(rows[0]),
                cells_base64=[base64.b64encode(cell.encode()).decode() for row in rows for cell in row])


def assets():
    rewards = []
    for native_id, name, cost in [(0, "none", ""), (1, "uav", "3"), (50, "super_katana", "0")]:
        row = [""] * 33
        row[0], row[1], row[4] = str(native_id), name, cost
        rewards.append(row)
    market = []
    for item_id, name in [(60076, "uav"), (60097, "uav_pro")]:
        market.append([str(item_id), name, "kis"] + [""] * 9)
    return dict(tables={STATS: table(rewards), MARKET: table(market)})


def fields():
    # Deliberately irregular records: the layout must come from each leaf.
    return {"weaponClasses[%d].killstreaks[%d]" % (i, s):
            dict(type=0, width=4, projected=False, offset=4000 + i * i * 5 + s * 4)
            for i in range(60) for s in range(4)}


class KillstreakPreparation(unittest.TestCase):
    def test_marketplace_ids_do_not_become_native_ids_and_pro_is_not_aliased(self):
        rewards, items = catalog(assets())
        self.assertEqual([r["native_id"] for r in rewards], [0, 1, 50])
        self.assertEqual(rewards[-1]["physical_row"], 2)
        self.assertEqual(items, [dict(item_id=60076, name="uav", native_id=1),
                                 dict(item_id=60097, name="uav_pro", native_id=None)])

    def test_old_version_columns_truncated_cells_and_duplicate_ids_are_rejected(self):
        original = assets()
        bad = copy.deepcopy(original)
        bad["tables"][STATS] = table([["0", "none"] + [""] * 22])
        with self.assertRaisesRegex(ValueError, "unexpected table"):
            catalog(bad)
        bad = copy.deepcopy(original)
        bad["tables"][STATS]["cells_base64"].pop()
        with self.assertRaisesRegex(ValueError, "incomplete"):
            catalog(bad)
        bad = copy.deepcopy(original)
        bad["tables"][STATS]["cells_base64"][66] = base64.b64encode(b"1").decode()
        with self.assertRaisesRegex(ValueError, "duplicate native"):
            catalog(bad)
        bad = copy.deepcopy(original)
        bad["tables"][MARKET]["cells_base64"][12] = base64.b64encode(b"60076").decode()
        with self.assertRaisesRegex(ValueError, "duplicate marketplace"):
            catalog(bad)

    def test_target_bounds_reject_overflow_without_guessing_another_cost_column(self):
        bad = assets()
        bad["tables"][STATS]["cells_base64"][37] = base64.b64encode(b"32").decode()
        bad["tables"][STATS]["cells_base64"][38] = base64.b64encode(b"3").decode()
        with self.assertRaisesRegex(ValueError, "cost"):
            catalog(bad)

    def test_four_slots_for_all_sixty_classes_use_each_captured_offset(self):
        leaves = fields()
        # Keep the first few irregular records disjoint.
        for i in range(4):
            for s in range(4):
                leaves["weaponClasses[%d].killstreaks[%d]" % (i, s)]["offset"] = 3900 + i * 20 + s * 4
        layout = class_layout(leaves)
        self.assertEqual(len(layout), 60)
        self.assertEqual(layout[0], [3900, 3904, 3908, 3912])
        self.assertEqual(layout[59], [21405, 21409, 21413, 21417])
        for invalid in ["missing", "projected", "overlapping"]:
            bad = copy.deepcopy(leaves)
            name = "weaponClasses[59].killstreaks[3]"
            if invalid == "missing":
                del bad[name]
            elif invalid == "projected":
                bad[name]["projected"] = True
            else:
                bad[name]["offset"] = layout[0][0]
            with self.assertRaises(ValueError):
                class_layout(bad)

    def test_modified_client_and_unmarked_or_other_version_captures_are_rejected(self):
        with self.assertRaisesRegex(ValueError, "unmodified target"):
            native(b"different client", "0" * 64)
        capture = dict(schema_version=1, read_only=True, client_sha256="a" * 64)
        identity(capture, "a" * 64)
        for key, value in [("read_only", False), ("client_sha256", "b" * 64), ("schema_version", 2)]:
            bad = dict(capture)
            bad[key] = value
            with self.assertRaisesRegex(ValueError, "target-client"):
                identity(bad, "a" * 64)


if __name__ == "__main__":
    unittest.main()
