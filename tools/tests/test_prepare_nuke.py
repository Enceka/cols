import base64
import copy
import json
from pathlib import Path
import struct
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from prepare_nuke import prepare

CLIENT = 'a' * 64
FX = ['explosions/player_death_nuke_flash', 'explosions/player_death_nuke', 'dust/nuke_aftermath_mp']


def captures():
    raw = bytearray(0xe28)  # Synthetic timing bytes, not a client definition.
    for off, value in [(0x74c, 1000), (0x750, 833), (0x768, 1000), (0x764, 833)]:
        struct.pack_into('<I', raw, off, value)
    identity = dict(schema_version=1, read_only=True, client_sha256=CLIENT, connection_state=10)
    a = dict(identity, map='maps/mp/mp_afghan.d3dbsp', ncs_crc=123,
             weapons=[dict(name='killstreak_nuke_mp', index=2, definition_base64=base64.b64encode(raw).decode(),
                           runtime_fields=dict(fire_interval_msec=800)), dict(name='nuke_mp', index=3)],
             effects=[dict(name=name, selector=i+1) for i, name in enumerate(FX)])
    n = dict(identity, initialized=1, linked=[dict(category=kind,
             strings_base64=[base64.b64encode(s.encode()).decode() for s in names])
             for kind, names in [(13, ['default', 'killstreak_nuke_mp', 'nuke_mp']), (3, FX),
                                 (5, ['ui_mp_nukebomb_timer', 'nuke_music']),
                                 (4, ['MP_EARNED_NUKE', 'MP_TACTICAL_NUKE_CALLED'])]])
    return a, n


class NukePreparation(unittest.TestCase):
    def prepare(self, a, n):
        with patch('prepare_nuke.manifest', return_value=dict(files=[dict(path='CODOL/codoMP_client_shipRetail.exe', sha256=CLIENT)])), \
             patch('prepare_nuke.compute', return_value=dict(ncs_crc=123)):
            return prepare(json.dumps(a).encode(), json.dumps(n).encode())

    def test_map_resources_follow_native_registration_and_keep_diagnostics_disabled(self):
        a, n = captures()
        r = self.prepare(a, n)
        self.assertEqual((r['map'], r['ncs_crc'], r['weapon'], r['damage_weapon']), ('mp_afghan', 123, 2, 3))
        self.assertEqual((r['tick_sound'], r['music_sound']), (512, 513))
        self.assertEqual(r['switch']['raise_msec'], 833)
        self.assertNotIn('acceptance_seed', r)
        self.assertNotIn('acceptance_stationary', r)

    def test_wrong_build_uninitialized_map_and_mismatched_registration_are_rejected(self):
        a, n = captures()
        for key, value in [('schema_version', 2), ('read_only', False), ('client_sha256', 'b'*64),
                           ('connection_state', 0), ('ncs_crc', 124), ('map', 'mp_afghan')]:
            bad = copy.deepcopy(a); bad[key] = value
            with self.assertRaises(ValueError): self.prepare(bad, n)
        for key, value in [('connection_state', 0), ('initialized', 0)]:
            bad = copy.deepcopy(n); bad[key] = value
            with self.assertRaises(ValueError): self.prepare(a, bad)
        bad = copy.deepcopy(a); bad['weapons'][0]['index'] = 60076
        with self.assertRaisesRegex(ValueError, 'registration'): self.prepare(bad, n)

    def test_missing_duplicate_and_truncated_resources_are_rejected(self):
        a, n = captures()
        bad = copy.deepcopy(a); bad['weapons'].append(bad['weapons'][0])
        with self.assertRaisesRegex(ValueError, 'ambiguous'): self.prepare(bad, n)
        bad = copy.deepcopy(a); bad['weapons'][0]['definition_base64'] = base64.b64encode(bytes(32)).decode()
        with self.assertRaisesRegex(ValueError, 'incomplete'): self.prepare(bad, n)
        bad = copy.deepcopy(n); bad['linked'][0]['strings_base64'].append(bad['linked'][0]['strings_base64'][1])
        with self.assertRaisesRegex(ValueError, 'ambiguous'): self.prepare(a, bad)
        bad = copy.deepcopy(n); bad['linked'][2]['strings_base64'].pop()
        with self.assertRaisesRegex(ValueError, 'absent'): self.prepare(a, bad)


if __name__ == '__main__':
    unittest.main()
