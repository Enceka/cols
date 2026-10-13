#!/usr/bin/env python3
"""Bind nuclear reward resources to an initialized native map capture."""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import struct

from clientlib import manifest
from prepare_game_state import compute


def prepare(assets_raw, ncs_raw):
    a, n = json.loads(assets_raw), json.loads(ncs_raw)
    client = next(f['sha256'] for f in manifest()['files'] if f['path'] == 'CODOL/codoMP_client_shipRetail.exe')
    if any(c.get('schema_version') != 1 or c.get('read_only') is not True or c.get('client_sha256') != client for c in (a, n)):
        raise ValueError('expected target read-only captures')
    crc = compute(n)['ncs_crc']
    if a['connection_state'] != 10 or n['connection_state'] != 10 or n['initialized'] != 1 or a['ncs_crc'] != crc:
        raise ValueError('nuclear resources require initialized matching map CRC')
    def bank(category):
        return [base64.b64decode(s, validate=True).decode('ascii') for r in n['linked']
                if r['category'] == category for s in r['strings_base64']]
    def slot(category, name):
        values = bank(category)
        if values.count(name) != 1:
            raise ValueError('nuclear resource absent or ambiguous: ' + name)
        return values.index(name) + 1
    def weapon(name):
        values = [w for w in a['weapons'] if w['name'] == name]
        if len(values) != 1:
            raise ValueError('nuclear weapon absent or ambiguous: ' + name)
        return values[0]
    w, damage = weapon('killstreak_nuke_mp'), weapon('nuke_mp')
    if w['index'] != slot(13, w['name']) or damage['index'] != slot(13, damage['name']):
        raise ValueError('native weapon registration differs from NCS order')
    raw = base64.b64decode(w['definition_base64'], validate=True)
    if len(raw) != 0xe28:
        raise ValueError('incomplete nuclear weapon definition')
    word = lambda off: struct.unpack_from('<I', raw, off)[0]
    effects = {e['name']: e['selector'] for e in a['effects']}
    if len(effects) != len(a['effects']):
        raise ValueError('ambiguous nuclear FX')
    for name, selector in effects.items():
        if selector != slot(3, name):
            raise ValueError('native FX selector differs from NCS order')
    if not a['map'].startswith('maps/mp/mp_') or not a['map'].endswith('.d3dbsp'):
        raise ValueError('invalid nuclear map identity')
    map_name = a['map'][8:-7]
    return dict(schema_version=1, client_sha256=client,
                sources_sha256=[hashlib.sha256(r).hexdigest() for r in (assets_raw, ncs_raw)],
                map=map_name, ncs_crc=crc, weapon=w['index'], damage_weapon=damage['index'],
                switch=dict(drop_msec=word(0x74c), raise_msec=word(0x750),
                            empty_drop_msec=word(0x768), empty_raise_msec=word(0x764)),
                fire_interval_msec=w['runtime_fields']['fire_interval_msec'],
                flash_fx=effects['explosions/player_death_nuke_flash'],
                blast_fx=effects['explosions/player_death_nuke'], aftermath_fx=effects['dust/nuke_aftermath_mp'],
                tick_sound=slot(5, 'ui_mp_nukebomb_timer') + 511,
                music_sound=slot(5, 'nuke_music') + 511,
                earned_text=slot(4, 'MP_EARNED_NUKE'), called_text=slot(4, 'MP_TACTICAL_NUKE_CALLED'))


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--assets', type=Path, required=True)
    p.add_argument('--ncs', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    args = p.parse_args()
    result = prepare(args.assets.read_bytes(), args.ncs.read_bytes())
    with args.output.open('x') as output:
        json.dump(result, output, indent=2); output.write('\n')
    print(json.dumps({'output': str(args.output), 'map': result['map'], 'weapon': result['weapon'], 'ncs_crc': result['ncs_crc']}))


if __name__ == '__main__':
    main()
