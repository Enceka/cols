#!/usr/bin/env python3
"""Bind two ordinary radio rewards to an initialized target map capture."""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import struct

from clientlib import manifest
from prepare_game_state import compute


def prepare(assets_raw, ncs_raw):
    assets, ncs = json.loads(assets_raw), json.loads(ncs_raw)
    client = next(f['sha256'] for f in manifest()['files']
                  if f['path'] == 'CODOL/codoMP_client_shipRetail.exe')
    if any(c.get('schema_version') != 1 or c.get('read_only') is not True
           or c.get('client_sha256') != client for c in (assets, ncs)):
        raise ValueError('expected target read-only captures')
    crc = compute(ncs)['ncs_crc']
    if (assets['connection_state'] != 10 or ncs['connection_state'] != 10
            or ncs['initialized'] != 1 or assets['ncs_crc'] != crc):
        raise ValueError('radio resources require initialized matching map CRC')
    names = [base64.b64decode(s, validate=True).decode('ascii')
             for bank in ncs['linked'] if bank['category'] == 13 for s in bank['strings_base64']]
    radios = []
    for native_id, name in [(1, 'killstreak_uav_mp'), (3, 'killstreak_counter_uav_mp')]:
        matches = [w for w in assets['weapons'] if w['name'] == name]
        if len(matches) != 1 or names.count(name) != 1:
            raise ValueError('radio absent or ambiguous: ' + name)
        weapon = matches[0]
        if weapon['index'] != names.index(name) + 1:
            raise ValueError('radio registration differs from NCS order')
        raw = base64.b64decode(weapon['definition_base64'], validate=True)
        if len(raw) != 0xe28:
            raise ValueError('incomplete radio definition')
        word = lambda off: struct.unpack_from('<I', raw, off)[0]
        switch = dict(drop_msec=word(0x74c), raise_msec=word(0x750),
                      empty_drop_msec=word(0x768), empty_raise_msec=word(0x764))
        interval = weapon['runtime_fields']['fire_interval_msec']
        if any(not 1 <= t <= 15000 for t in [*switch.values(), interval]):
            raise ValueError('invalid radio timing')
        radios.append(dict(native_id=native_id, weapon=weapon['index'], switch=switch,
                           fire_interval_msec=interval))
    map_path = assets['map']
    if not map_path.startswith('maps/mp/mp_') or not map_path.endswith('.d3dbsp'):
        raise ValueError('invalid radio map identity')
    return dict(schema_version=1, client_sha256=client,
                sources_sha256=[hashlib.sha256(r).hexdigest() for r in (assets_raw, ncs_raw)],
                map=map_path[8:-7], ncs_crc=crc, radios=radios)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--assets', type=Path, required=True)
    parser.add_argument('--ncs', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = prepare(args.assets.read_bytes(), args.ncs.read_bytes())
    with args.output.open('x') as output:
        json.dump(result, output, indent=2); output.write('\n')
    print(json.dumps({'output': str(args.output), 'map': result['map'],
                      'radios': result['radios'], 'ncs_crc': result['ncs_crc']}))


if __name__ == '__main__':
    main()
