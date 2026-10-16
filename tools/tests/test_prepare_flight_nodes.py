import base64
import copy
import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from prepare_flight_nodes import prepare
from prepare_game_state import CLIENT_SHA256


def capture(text):
    raw = text.encode('ascii')
    data = dict(schema_version=1, read_only=True, client_sha256=CLIENT_SHA256,
                maps=[dict(name='maps/mp/mp_afghan.d3dbsp', byte_count=len(raw),
                           entity_string_base64=base64.b64encode(raw).decode())])
    return data, json.dumps(data).encode()


class FlightNodes(unittest.TestCase):
    def test_heading_is_preserved_without_inventing_missing_angles(self):
        data, raw = capture('''
        {"targetname" "heli_start" "target" "turn" "origin" "0 0 500" "angles" "0 135 5"}
        {"targetname" "turn" "target" "last" "origin" "100 0 500" "angle" "270"}
        {"targetname" "last" "origin" "200 0 500"}
        ''')
        nodes = prepare(data, 'mp_afghan', raw)['nodes']
        self.assertEqual(nodes[0]['angles'], [0, 135, 5])
        self.assertEqual(nodes[1]['angles'], [0, 270, 0])
        self.assertNotIn('angles', nodes[2])
        for field, value in [('angles', '0 nan 0'), ('angles', '0 90'),
                             ('angles', '0 4000 0'), ('angle', 'inf')]:
            data, raw = capture('{"targetname" "heli_start" "origin" "0 0 500" "'+field+'" "'+value+'"}')
            with self.assertRaises(ValueError):
                prepare(data, 'mp_afghan', raw)

    def test_shared_destinations_loop_prefix_and_authored_values_are_preserved(self):
        data, raw = capture('''
        {"targetname" "heli_start" "target" "dest" "origin" "0 0 500"}
        {"targetname" "heli_start" "target" "dest" "origin" "0 50 500"}
        {"targetname" "dest" "origin" "100 100 500" "script_airspeed" "40" "script_accel" "15"}
        {"targetname" "heli_loop_start" "target" "auto1" "origin" "0 0 500"}
        {"targetname" "auto1" "target" "auto2" "origin" "100 0 500" "script_delay" "2"}
        {"targetname" "auto2" "target" "auto1" "origin" "100 100 500"}
        {"targetname" "unrelated" "origin" "0 0 0"}
        ''')
        paths = prepare(data, 'mp_afghan', raw)
        self.assertEqual(len(paths['nodes']), 6)
        starts = paths['entries']['heli_start']
        self.assertEqual(len(starts), 2)
        self.assertEqual(paths['nodes'][starts[0]]['target'], paths['nodes'][starts[1]]['target'])
        dest = paths['nodes'][paths['nodes'][starts[0]]['target']]
        self.assertEqual((dest['script_airspeed'], dest['script_accel']), (40, 15))
        self.assertNotIn('script_airspeed', paths['nodes'][starts[0]])
        self.assertNotIn('unrelated', [node['name'] for node in paths['nodes']])
        self.assertEqual(len(paths['sources_sha256'][0]), 64)

    def test_missing_ambiguous_or_invalid_targets_do_not_become_synthetic_paths(self):
        root = '{"targetname" "heli_start" "target" "dest" "origin" "0 0 500"}'
        dest = '{"targetname" "dest" "origin" "100 100 500"}'
        for text in [root, root+dest+dest, root+dest.replace('100 100 500', 'nan 100 500'),
                     root+dest[:-1]+' "script_accel" "-1"}', root+dest[:-1]+' "script_delay" "601"}']:
            data, raw = capture(text)
            with self.assertRaises(ValueError): prepare(data, 'mp_afghan', raw)

    def test_maps_without_flight_roots_and_mismatching_capture_are_distinct(self):
        data, raw = capture('{"targetname" "other" "origin" "0 0 0"}')
        self.assertIsNone(prepare(data, 'mp_afghan', raw))
        for mutate in [lambda d: d.update(read_only=False),
                       lambda d: d.update(client_sha256='0'*64),
                       lambda d: d['maps'][0].update(name='maps/mp/mp_rust.d3dbsp')]:
            bad = copy.deepcopy(data); mutate(bad)
            with self.assertRaises(ValueError): prepare(bad, 'mp_afghan', raw)


if __name__ == '__main__': unittest.main()
