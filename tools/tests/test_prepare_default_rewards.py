import json
from pathlib import Path
import struct
import sys
import unittest
import zlib

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from prepare_default_rewards import seed_empty_rewards, default_items, CATALOG
from prepare_inventory import public_snapshot


def fixture():
    catalog = json.loads(CATALOG.read_bytes())
    fields = {}
    for index, offsets in enumerate(catalog['class_offsets']):
        for slot, offset in enumerate(offsets[:3]):
            fields['weaponClasses[%d].killstreaks[%d]' % (index, slot)] = dict(type=0, width=4, offset=offset, projected=False)
    for i in range(125):
        fields['projection%d' % i] = dict(type=0, width=4, offset=20000+i*4, projected=True)
    return catalog, fields


def encode(snapshot, fields):
    plain = bytearray(snapshot)
    crc = zlib.crc32(plain[4:])
    struct.pack_into('<I', plain, 0, crc)
    blob = zlib.compress(plain, 1)
    u32 = lambda value: b'\x08' + struct.pack('<I', value)
    tail = b''.join(b'\x07' + plain[f['offset']:f['offset']+4] for f in fields.values() if f['projected']) + b'\0'
    return u32(1)+u32(1)+u32(crc)+b'\x13'+u32(len(blob))+blob+tail


class DefaultRewards(unittest.TestCase):
    def test_all_sixty_empty_backpacks_get_target_items_and_other_bytes_survive(self):
        catalog, fields = fixture()
        original = bytearray(32768)
        original[31000] = 93
        data = encode(original, fields)
        result, items, changed = seed_empty_rewards(data, fields, catalog)
        self.assertEqual(items, [60076, 60080, 60083])
        self.assertEqual(changed, list(range(60)))
        before, old_tail = public_snapshot(data)
        after, new_tail = public_snapshot(result)
        self.assertEqual(old_tail, new_tail)
        allowed = set(range(4))
        for offsets in catalog['class_offsets']:
            self.assertEqual([struct.unpack_from('<i', after, o)[0] for o in offsets[:3]], [60077, 60081, 60084])
            for offset in offsets[:3]:
                allowed.update(range(offset, offset+4))
        self.assertTrue(all(a == b or i in allowed for i, (a, b) in enumerate(zip(before, after))))
        self.assertEqual(seed_empty_rewards(result, fields, catalog), (result, items, []))

    def test_existing_and_partial_custom_selections_are_preserved(self):
        catalog, fields = fixture()
        original = bytearray(32768)
        for index, values in [(0, [60077, 60079, 60091]), (1, [60077, 1, 1])]:
            for offset, value in zip(catalog['class_offsets'][index], values):
                struct.pack_into('<i', original, offset, value)
        result, _, changed = seed_empty_rewards(encode(original, fields), fields, catalog)
        snapshot, _ = public_snapshot(result)
        self.assertEqual(changed, list(range(2, 60)))
        for index, values in [(0, [60077, 60079, 60091]), (1, [60077, 1, 1])]:
            self.assertEqual([struct.unpack_from('<i', snapshot, at)[0] for at in catalog['class_offsets'][index][:3]], values)

    def test_projection_and_schema_corruption_are_rejected(self):
        catalog, fields = fixture()
        data = encode(bytes(32768), fields)
        corrupted = bytearray(data)
        corrupted[-3] = 1
        with self.assertRaises(ValueError):
            seed_empty_rewards(bytes(corrupted), fields, catalog)
        fields['weaponClasses[0].killstreaks[0]']['offset'] += 1
        with self.assertRaises(ValueError):
            seed_empty_rewards(data, fields, catalog)

    def test_default_rewards_require_exact_unambiguous_catalog_names(self):
        catalog, _ = fixture()
        catalog['marketplace'] = [entry for entry in catalog['marketplace'] if entry['name'] != 'uav']
        with self.assertRaises(ValueError):
            default_items(catalog)


class ServiceDataRepair(unittest.TestCase):
    def test_repair_is_planned_first_backed_up_idempotent_and_checks_ownership(self):
        import base64
        import tempfile
        from unittest.mock import patch
        from prepare_default_rewards import repair_service_data
        catalog, fields = fixture()
        original = encode(bytes(32768), fields)
        client = 'e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b'
        assets = dict(read_only=True, client_sha256=client, schemas={'mp/playerdata.def':{'definitions':[dict(version=233,formatChecksum=3811397846)]}})
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            template = root/'new-player-template.json'
            template.write_text(json.dumps(dict(schema_version=1,client_sha256=client,public_base64=base64.b64encode(original).decode(),starter_items=[10001],all_items=[10001,60076,60080,60083])))
            paths = []
            for uid, items in [(42,[60076,60080,60083]), (43,[10001])]:
                account = root/'profiles/1002'/str(uid)
                account.mkdir(parents=True)
                (account/'public.bin').write_bytes(original)
                (account/'inventory.json').write_text(json.dumps(dict(items=items)))
                (account/'private.bin').write_bytes(b'private progress remains')
                paths.append(account/'public.bin')
            with patch('schema_fields.flatten', return_value=fields):
                plan = repair_service_data(assets, root)
                self.assertEqual(len(plan['changes']), 2)
                self.assertEqual(len(plan['skipped']), 1)
                self.assertEqual(paths[0].read_bytes(), original)
                applied = repair_service_data(assets, root, True)
                self.assertEqual((Path(applied['backup'])/'profiles/1002/42/public.bin').read_bytes(), original)
                self.assertEqual(repair_service_data(assets, root)['changes'], [])
            self.assertNotEqual(paths[0].read_bytes(), original)
            self.assertEqual(paths[1].read_bytes(), original)
            self.assertEqual((paths[0].parent/'private.bin').read_bytes(), b'private progress remains')
