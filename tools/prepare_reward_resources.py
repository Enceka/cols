#!/usr/bin/env python3
"""Prepare minimal registered reward resources from a matching initialized map."""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import struct
from prepare_game_state import compute, CLIENT_SHA256

CATALOG = Path(__file__).resolve().parents[1] / 'fixtures/protocol/killstreaks-3.3.3.3.json'


def prepare(assets_raw, ncs_raw, catalog=None):
    assets, ncs = json.loads(assets_raw), json.loads(ncs_raw)
    catalog = catalog or json.loads(CATALOG.read_bytes())
    for capture in [assets, ncs]:
        if capture.get('schema_version') != 1 or capture.get('read_only') is not True or capture.get('client_sha256') != CLIENT_SHA256:
            raise ValueError('expected read-only target-client capture')
    crc = compute(ncs)['ncs_crc']
    if assets.get('connection_state') != 10 or ncs.get('connection_state') != 10 or ncs.get('initialized') != 1 or assets.get('ncs_crc') != crc:
        raise ValueError('reward resources require initialized matching CRC')
    def bank(kind):
        return [base64.b64decode(s, validate=True).decode('ascii') for row in ncs['linked'] if row['category'] == kind for s in row['strings_base64']]
    def slot(kind, name):
        names = bank(kind)
        if names.count(name) != 1:
            raise ValueError('missing or ambiguous registered resource: ' + name)
        return names.index(name) + 1
    words = {}
    for weapon in assets['weapons']:
        name = weapon['name']
        if name in words:
            raise ValueError('duplicate weapon registration')
        if name.startswith('** '): continue
        if weapon['index'] != slot(13, name):
            # Composed backpack forms are dynamically registered after NCS.
            if not name.startswith('** '):
                raise ValueError('weapon/NCS registration mismatch')
            continue
        raw = base64.b64decode(weapon['definition_base64'], validate=True)
        if len(raw) != 0xe28:
            raise ValueError('incomplete registered weapon definition')
        u = lambda offset: struct.unpack_from('<I', raw, offset)[0]
        words[name] = dict(weapon=weapon['index'], weapon_type=u(0x47c), weapon_class=u(0x480),
                          clip_key=None if u(0x6c0)==0xffffffff else u(0x6c0), ammo_key=None if u(0x6b8)==0xffffffff else u(0x6b8),
                          switch=dict(drop_msec=u(0x74c), raise_msec=u(0x750), empty_drop_msec=u(0x768), empty_raise_msec=u(0x764)),
                          fire_interval_msec=weapon['runtime_fields']['fire_interval_msec'],
                          radius=u(0x8d8), inner_damage=u(0x8e0), outer_damage=u(0x8e4))
    radios, unavailable = [], []
    for reward in catalog['rewards']:
        if reward['native_id'] == 0:
            continue
        definition = words.get(reward['weapon_token_c13'])
        if definition is None:
            unavailable.append(reward['native_id']); continue
        radios.append(dict(native_id=reward['native_id'], **{k: definition[k] for k in ['weapon','switch','fire_interval_msec']}))
    models = {}
    for model in assets['models']:
        if model['name'] not in bank(0): continue
        if model['name'] in models or model['index'] != slot(0, model['name']):
            raise ValueError('model registration mismatch')
        models[model['name']] = model['index']
    effects = {}
    for effect in assets['effects']:
        if effect['name'] in effects or effect['selector'] != slot(3, effect['name']):
            raise ValueError('effect registration mismatch')
        effects[effect['name']] = effect['selector']
    path = assets['map']
    if not path.startswith('maps/mp/mp_') or not path.endswith('.d3dbsp'):
        raise ValueError('invalid map identity')
    result = dict(schema_version=1, client_sha256=CLIENT_SHA256,
                  sources_sha256=[hashlib.sha256(r).hexdigest() for r in [assets_raw, ncs_raw]],
                  map=path[8:-7], ncs_crc=crc, radios=radios, models=models, effects=effects)
    # Companion weapon facts remain separate from activator timings.
    relevant = {r['weapon_token_c13'] for r in catalog['rewards']}
    prefixes = ('ac130_', 'cobra_', 'harrier_', 'pavelow_', 'sentry_', 'tow_turret_', 'remotemissile_', 'remote_missile', 'rcbomb_', 'missile_drone_', 'mg4juggernaut', 'minigungunship', 'kingpin_spawn', 'artillery_', 'airstrike_', 'stealth_bomb_', 'dog_bite', 'littlebird_')
    result['weapons'] = {name: data for name, data in words.items() if name in relevant or name.startswith(prefixes)}
    return result, unavailable


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--assets', type=Path, required=True); p.add_argument('--ncs', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True); args = p.parse_args()
    result, unavailable = prepare(args.assets.read_bytes(), args.ncs.read_bytes())
    with args.output.open('x') as f: json.dump(result, f, indent=2); f.write('\n')
    print(json.dumps(dict(map=result['map'], activators=len(result['radios']), unavailable=unavailable)))


if __name__ == '__main__': main()
