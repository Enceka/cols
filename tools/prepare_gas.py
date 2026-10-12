#!/usr/bin/env python3
"""Bind captured native Nova FX to explicitly sourced reference gas area rules."""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import struct

from clientlib import manifest


def prepare(registry_path, reference_dir):
    raw = registry_path.read_bytes()
    registry = json.loads(raw)
    expected = next(f['sha256'] for f in manifest()['files']
                    if f['path'] == 'CODOL/codoMP_client_shipRetail.exe')
    if registry.get('client_sha256') != expected or registry.get('read_only') is not True:
        raise ValueError('expected fixed-client read-only weapon capture')
    weapon = next(w for w in registry['offhand_definitions'] if w['name'] == 'tabun_center_mp')
    definition = base64.b64decode(weapon['definition_base64'], validate=True)
    effect = weapon['explosion_effect']
    elements = base64.b64decode(effect['elements_base64'], validate=True)
    if (len(definition) != 0xe28 or weapon['index'] != 250 or
            struct.unpack_from('<I', definition, 0x918)[0] == 0 or
            effect['name'] != 'weapon/grenade/fx_gas_nova_mp' or
            effect['element_stride'] != 260 or effect['looping_life_msec'] != 3600 or
            (effect['looping_count'], effect['one_shot_count'], effect['emission_count']) != (3, 8, 3) or
            len(elements) != 14 * 260):
        raise ValueError('native Nova effect binding differs')
    # Reconstruction cutoff for the captured parent emitters. Emission children
    # are left to the client's native FX runtime; this is not an optical model.
    lifetime = max((effect['looping_life_msec'] if i < effect['looping_count'] else 0) +
                   sum(struct.unpack_from('<I', elements, i * 260 + o)[0]
                       for o in (0x28, 0x2c, 0x30, 0x34)) for i in range(14))
    source = reference_dir / 'maps/mp/equipment/_tabun.gsc'
    data = source.read_bytes()
    checks = ['setting( "scr_tabunGasDuration", 8 )', 'setting( "scr_poisonDuration", 8 )',
              'setting( "scr_poisonDamage", 13 )', 'setting( "scr_tabun_effect_radius", 185 )',
              'setting( "scr_tabun_shock_height", 20 )', 'dirs[i]*220',
              'self hasAbilityModifier( "modGasMask" )', 'nextDamage=getTime()+1000;',
              'clearAt=getTime()+1000;', 'self.team==cloud.owner.team',
              'cloud.killcam, 0, "MOD_GAS", 0, "tabun_gas_mp"']
    if not all(c in data.decode('utf-8') for c in checks):
        raise ValueError('reference gas rules differ')
    return {'schema_version': 1, 'client_sha256': expected,
            'capture_sha256': hashlib.sha256(raw).hexdigest(),
            'native_effect': {'weapon': weapon['index'], 'name': weapon['name'],
                              'effect': effect['name'], 'lifetime_msec': lifetime,
                              'definition_sha256': hashlib.sha256(definition).hexdigest(),
                              'elements_sha256': hashlib.sha256(elements).hexdigest(),
                              'source': 'captured Nova center binding; reference poison FX is a different resource'},
            'server_rules': {'source': 'Alpha/LAN reference script port; current client production rules unverified',
                             'script': 'maps/mp/equipment/_tabun.gsc',
                             'sha256': hashlib.sha256(data).hexdigest(),
                             'cloud_msec': 8000, 'tick_msec': 1000, 'damage': 13,
                             'clear_msec': 1000, 'radius': 185.0, 'height': 20.0,
                             'spoke_length': 220.0, 'radii': [55.0, 55.0, 50.0, 25.0]}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--registry', type=Path, required=True)
    parser.add_argument('--reference-dir', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = prepare(args.registry, args.reference_dir)
    with args.output.open('x') as output:
        json.dump(result, output, indent=2, allow_nan=False)
        output.write('\n')
    print(json.dumps({'output': str(args.output), 'effect': result['native_effect']['effect']}))


if __name__ == '__main__':
    main()
