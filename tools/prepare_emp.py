#!/usr/bin/env python3
"""Verify this client's EMP HUD bit and source the reference player lifecycle."""
import argparse
import hashlib
import json
from pathlib import Path
import struct

import pefile
from clientlib import manifest


def prepare(client_path, reference_dir):
    raw = client_path.read_bytes()
    expected = next(f['sha256'] for f in manifest()['files']
                    if f['path'] == 'CODOL/codoMP_client_shipRetail.exe')
    if hashlib.sha256(raw).hexdigest() != expected:
        raise ValueError('expected unmodified current client')
    pe = pefile.PE(data=raw)
    image = pe.get_memory_mapped_image()
    base = pe.OPTIONAL_HEADER.ImageBase
    word = lambda at: struct.unpack_from('<I', image, at - base)[0]
    case = word(0x578090 + (154 - 30) * 4)
    block = image[case - base:case - base + 20]
    calls = [case + i + 5 + struct.unpack_from('<i', block, i + 1)[0]
             for i in range(len(block) - 4) if block[i] == 0xe8]
    if 0x574130 not in calls:
        raise ValueError('EMP HUD operator dispatch differs')
    getter = image[0x574130 - base:0x57416c - base]
    if bytes.fromhex('8b4a10c1e90af6c101') not in getter:
        raise ValueError('EMP otherFlags consumer differs')
    sources = []
    for filename, checks in [
        ('maps/mp/_empgrenade.gsc', ['self.empEndTime = getTime() + 10000;',
                                    'self setEMPJammed( true );', 'self shellshock( "flashbang_mp", 1 );',
                                    'self _hasPerk( "specialty_empimmune" )', 'attacker.team == self.team',
                                    'self clearPersonalEMP();']),
        ('maps/mp/gametypes/_damage.gsc', ['sWeapon == "emp_grenade_mp" && sMeansOfDeath != "MOD_IMPACT"',
                                          'victim notify( "emp_grenaded", eAttacker );'])]:
        data = (reference_dir / filename).read_bytes()
        if not all(c in data.decode('utf-8') for c in checks):
            raise ValueError('reference player EMP rules differ')
        sources.append({'path': filename, 'sha256': hashlib.sha256(data).hexdigest()})
    return {'schema_version': 1, 'client_sha256': expected,
            'native': {'hud_operator': 154, 'getter_va': 0x574130,
                       'getter_sha256': hashlib.sha256(getter).hexdigest(),
                       'player_state_offset': 0x10, 'field': 54, 'bits': 28, 'mask': 0x400},
            'server_rules': {'source': 'Alpha/LAN IW5 reference player EMP port; current production rules unverified',
                             'sources': sources, 'duration_msec': 10000, 'flash_msec': 1000,
                             'immunity_ability': 'modEmpProtectionScale'}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--client', type=Path, required=True)
    parser.add_argument('--reference-dir', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = prepare(args.client, args.reference_dir)
    with args.output.open('x') as output:
        json.dump(result, output, indent=2, allow_nan=False)
        output.write('\n')
    print(json.dumps({'output': str(args.output), 'hud_operator': result['native']['hud_operator']}))


if __name__ == '__main__':
    main()
