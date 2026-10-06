import base64
import copy
import struct
import sys
from pathlib import Path
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from prepare_shop import offers_from_assets


class ShopPreparation(unittest.TestCase):
    def fixture(self):
        return {"runtime_marketplace": {
            "products": [{"index": 1, "header_base64": base64.b64encode(struct.pack("<5HBB3I", 2, 0, 0, 0, 0, 1, 0, 123456, 0, 0)).decode(), "item_ids": [100]}],
            "skus": [{"index": 1, "fields": [3, 5, 2, 0, 2600, 0, 0, 0, 0], "byte14": 0, "byte15": 0}]
        }, "tables": {"mp/coMarketplaceItems.csv": {"found": True, "rows": 1, "columns": 12, "cells_base64": [base64.b64encode(b"100").decode(), "", base64.b64encode(b"wat").decode()] + [""] * 9}}}

    def test_import_uses_sku_product_relation_and_filters_rental_or_timed_offers(self):
        data = self.fixture()
        self.assertEqual(offers_from_assets(data), [{"sku_id": 3, "item_id": 100, "payment": "gp", "price": 2600}])
        for field in (5, 6, 7, 8):
            extra = copy.deepcopy(data["runtime_marketplace"]["skus"][0])
            extra["fields"][0] = 4
            extra["fields"][field] = 1
            data["runtime_marketplace"]["skus"].append(extra)
            self.assertEqual(len(offers_from_assets(data)), 1)
            data["runtime_marketplace"]["skus"].pop()

    def test_rejects_unknown_item_duplicate_sku_and_corrupt_product_count(self):
        data = self.fixture()
        data["runtime_marketplace"]["products"][0]["item_ids"] = [200]
        with self.assertRaises(ValueError): offers_from_assets(data)
        data = self.fixture()
        data["runtime_marketplace"]["skus"] *= 2
        with self.assertRaises(ValueError): offers_from_assets(data)
        data = self.fixture()
        data["runtime_marketplace"]["products"][0]["item_ids"] = []
        with self.assertRaises(ValueError): offers_from_assets(data)
