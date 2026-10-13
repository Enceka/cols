import base64
import copy
import json
from pathlib import Path
import struct
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from prepare_radars import prepare

CLIENT = 'a' * 64


def captures():
    raw = bytearray(0xe28)
    for offset, value in [(0x74c, 1000), (0x750, 833), (0x768, 1000), (0x764, 833)]:
        struct.pack_into('<I', raw, offset, value)
    identity = dict(schema_version=1, read_only=True, client_sha256=CLIENT, connection_state=10)
    names = ['killstreak_uav_mp', 'killstreak_counter_uav_mp']
    assets = dict(identity, map='maps/mp/mp_afghan.d3dbsp', ncs_crc=123,
                  weapons=[dict(name=name, index=i+1, definition_base64=base64.b64encode(raw).decode(),
                                runtime_fields=dict(fire_interval_msec=800)) for i, name in enumerate(names)])
    ncs = dict(identity, initialized=1, linked=[dict(category=13,
               strings_base64=[base64.b64encode(n.encode()).decode() for n in names])])
    return assets, ncs


class RadarPreparation(unittest.TestCase):
    def prepare(self, assets, ncs):
        with patch('prepare_radars.manifest', return_value=dict(files=[dict(path='CODOL/codoMP_client_shipRetail.exe', sha256=CLIENT)])), \
             patch('prepare_radars.compute', return_value=dict(ncs_crc=123)):
            return prepare(json.dumps(assets).encode(), json.dumps(ncs).encode())

    def test_registration_and_timing_are_independent_of_marketplace_ids(self):
        assets, ncs = captures()
        result = self.prepare(assets, ncs)
        self.assertEqual(result['map'], 'mp_afghan')
        self.assertEqual([(r['native_id'], r['weapon']) for r in result['radios']], [(1, 1), (3, 2)])
        self.assertEqual(result['radios'][0]['switch']['raise_msec'], 833)

    def test_wrong_build_crc_and_uninitialized_registry_are_rejected(self):
        assets, ncs = captures()
        for key, value in [('client_sha256', 'b'*64), ('read_only', False), ('connection_state', 0), ('ncs_crc', 124)]:
            bad = copy.deepcopy(assets); bad[key] = value
            with self.assertRaises(ValueError): self.prepare(bad, ncs)
        ncs['initialized'] = 0
        with self.assertRaises(ValueError): self.prepare(assets, ncs)

    def test_missing_duplicate_registration_and_truncated_definition_are_rejected(self):
        assets, ncs = captures()
        for mutate in [lambda a: a['weapons'].pop(), lambda a: a['weapons'].append(a['weapons'][0]),
                       lambda a: a['weapons'][0].update(index=60076),
                       lambda a: a['weapons'][0].update(definition_base64=base64.b64encode(bytes(32)).decode())]:
            bad = copy.deepcopy(assets); mutate(bad)
            with self.assertRaises(ValueError): self.prepare(bad, ncs)


if __name__ == '__main__':
    unittest.main()
