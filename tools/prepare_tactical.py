#!/usr/bin/env python3
"""Prepare native shock bindings and explicitly sourced reference server rules."""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import re
import struct
import zlib

from clientlib import manifest


def prepare(capture_path, reference_dir):
    raw = capture_path.read_bytes()
    capture = json.loads(raw)
    expected = next(f['sha256'] for f in manifest()['files'] if f['path'] == 'CODOL/codoMP_client_shipRetail.exe')
    if capture.get('client_sha256') != expected or capture.get('read_only') is not True or capture.get('connection_state') != 10:
        raise ValueError('expected a loaded current-client read-only tactical capture')
    names = []
    for bank in capture['shock_ncs']:
        if bank['category'] != 7:
            raise ValueError('unexpected shock NCS category')
        names.extend(base64.b64decode(s, validate=True).decode('ascii') for s in bank['strings_base64'])
    if len(names) > 15 or len(set(names)) != len(names):
        raise ValueError('native shock binding budget or duplicates')
    bindings = []
    for kind, name in [('flash', 'flashbang_mp'), ('concussion', 'concussion_grenade_mp')]:
        index = names.index(name) + 1
        record = capture['rawfiles']['shock/' + name + '.shock']
        payload = base64.b64decode(record['buffer_base64'], validate=True)
        if not record.get('found') or len(payload) != (record['compressed_length'] or record['length']):
            raise ValueError('missing or incomplete native shock rawfile')
        if record['compressed_length']:
            decoder = zlib.decompressobj()
            payload = decoder.decompress(payload, record['length'] + 1)
            if not decoder.eof or decoder.unused_data or decoder.unconsumed_tail:
                raise ValueError('invalid shock compression stream')
        if len(payload) != record['length']:
            raise ValueError('native shock length differs')
        text = payload.rstrip(b'\0').decode('ascii')
        text = re.sub(r'//[^\n]*', '', text)
        settings = dict(re.findall(r'([^\s"]+)\s+"([^"]*)"', text))
        params = base64.b64decode(capture['shocks'][index]['parameters_base64'], validate=True)
        if len(params) != 0x268 or capture['shocks'][index]['index'] != index:
            raise ValueError('native shock block layout differs')
        word = lambda o: struct.unpack_from('<I', params, o)[0]
        number = lambda o: struct.unpack_from('<f', params, o)[0]
        screen = {'blurred': 0, 'flashed': 1, 'none': 2}[settings['66']]
        if word(0x10) != screen or bool(params[0x264]) != bool(int(settings['92'])):
            raise ValueError('shock file and registered parameters disagree')
        bindings.append({'kind': kind, 'name': name, 'index': index, 'configstring': 4275 + index,
                         'screen_type': screen, 'movement': bool(params[0x264]),
                         'look_control': bool(params[0x250]), 'max_pitch_speed': number(0x25c),
                         'max_yaw_speed': number(0x260), 'mouse_sensitivity_scale': number(0x258),
                         'rawfile_sha256': hashlib.sha256(payload).hexdigest(),
                         'parameter_sha256': hashlib.sha256(params).hexdigest()})
    sources = []
    for filename, checks in [
        ('maps/mp/_flashgrenades.gsc', ['duration = amount_distance * amount_angle * 6;', 'if ( duration < 0.25 )', 'if ( amount_angle < 0.5 )', 'else if ( amount_angle > 0.8 )', 'duration *= protectionScale;']),
        ('maps/mp/gametypes/_weapons.gsc', ['time = 2 + ( 4 * scale );', 'time *= protectionScale;'])]:
        p = reference_dir / filename
        data = p.read_bytes()
        text = data.decode('utf-8')
        if not all(c in text for c in checks):
            raise ValueError('reference duration script differs from recovered source')
        sources.append({'path': filename, 'sha256': hashlib.sha256(data).hexdigest()})
    return {'schema_version': 1, 'client_sha256': expected, 'capture_sha256': hashlib.sha256(raw).hexdigest(),
            'bindings': bindings,
            'server_rules': {'source': 'Alpha/LAN reference script port; current client production rules unverified',
                             'sources': sources, 'flash_max_msec': 6000, 'flash_min_msec': 250,
                             'flash_angle_floor': 0.5, 'flash_angle_full_threshold': 0.8,
                             'concussion_min_msec': 2000, 'concussion_distance_msec': 4000}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--capture', type=Path, required=True)
    parser.add_argument('--reference-dir', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = prepare(args.capture, args.reference_dir)
    with args.output.open('x') as output:
        json.dump(result, output, indent=2, allow_nan=False)
        output.write('\n')
    print(json.dumps({'output': str(args.output), 'bindings': len(result['bindings'])}))


if __name__ == '__main__':
    main()
