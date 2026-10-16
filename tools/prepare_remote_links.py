#!/usr/bin/env python3
"""Verify remote camera/input consumers in the unchanged 3.3.3.3 client."""
import argparse
import hashlib
import json
from pathlib import Path
import struct

from clientlib import manifest

# Every instruction starts at a decoded boundary reached from a verified entry.
ANCHORS = [
    (0x52db1e, 'f64010027407e80750ffff', 'camera link flag dispatch'),
    (0x522b4b, '8b47543dff070000', 'camera entity and NONE sentinel'),
    (0x522ba0, '8b4f5881c103150000', 'camera tag configstring5379+index'),
    (0x522c1a, '8b471084c00f8885000000', 'tag orientation bit7 branch'),
    (0x41bcfc, 'f7460c000008000f85f0000000', 'ordinary angle update remote gate'),
    (0x4600b0, 'f74618000008007417', 'snapshot PS remote input mode'),
]
# Keep the signed byte consumers separately.
ANCHORS += [
    (0x45fee4, '814e0400002000', 'remote input command flag'),
    (0x45ffcc, '884627', 'pitch byte command+0x27'),
    (0x460068, '884e28', 'yaw byte command+0x28'),
    (0x414dac, '0fbe4827', 'signed pitch consumer'),
    (0x414dbf, '0fbe4028', 'signed yaw consumer'),
    (0x513848,'51285100','rocket90 native dispatch'),
    (0x5138e8,'79355100','remote boost130 native dispatch'),
    (0x512985,'8b8620090000','rocket weapon explosion sound field920'),
    (0x50e769,'8bbb18090000','projectile explosion FX field918'),
    (0x50e364,'f6401002','boost camera-linked gate'),
    (0x50e36a,'8b48543b8ef00000007537','boost matches camera-linked missile entity'),

    (0x4e63ad, 'd905b455c800', 'remote normal speed3000 default'),
    (0x4e63bb, 'd905045ac800', 'remote boost speed6000 default'),
    (0x4e6419, 'd9059455c800', 'remote acceleration2000 default'),
    (0x4e647a, 'd905b855c800', 'remote deceleration500 default'),

    (0x412397, 'd9052858c800d91c246870d4c500', 'pitch rate35 registration'),
    (0x4123cf, 'd9052858c800d91c246874d4c500', 'yaw rate35 registration'),

]

WEAPON_VIEW_ANCHORS = [
    (0x523893, 'f6461008', 'thermal otherFlags8 getter'),
    (0x52da4a, 'f6401404', 'weapon-view eFlags4 dispatch'),
    (0x52da58, '8b88f0010000', 'weapon-view entity PS1f0'),
    (0x55cc78, '8b5010f6c20275e9', 'camera2 mode blocks native weapon cycling'),
    (0x41b0a1, 'd905c057c800d89cb774010000', 'weapon-view angle clamp ranges PS174/178'),
]

def verify_weapon_view(read):
    checks=[]
    for address, expected, meaning in WEAPON_VIEW_ANCHORS:
        raw=read(address,len(bytes.fromhex(expected)))
        if raw.hex()!=expected:
            raise ValueError('weapon-view consumer differs: '+meaning)
        checks.append(dict(va=address,bytes=expected,meaning=meaning))
    if struct.unpack('<f',read(0xc857c0,4))[0]!=180.0:
        raise ValueError('weapon-view unlimited clamp differs')
    return dict(eflags=4,eflags_index=154,entity_index=68,entity_offset=0x1f0,
                clamp_indices=[120,119],unlimited_clamp=180.0,thermal_flag=8,
                thermal_state_offset=0x16bfd0,camera_mode_blocks_weapon_cycle=True,consumers=checks)


VISION_ANCHORS = [
    (0x57d5af, '83f87d0f87e10b00000fb680a4e25700ff2485ace15700',
     'reliable command bounded two-stage dispatcher'),
    (0x57dd64, '6a406a018db728b41600',
     'missilecam command reads bounded name into cg16b428'),
    (0x57dd80, '6a02e8d9d4ebff83c40450e8d8905700',
     'missilecam transition parses argument2 as integer milliseconds'),
    (0x57dd93, '508bceb802000000e810a3edff',
     'missilecam applies vision slot2 with transition'),
]


NAKED_VISION_ANCHORS = [
    (0x57dcc7, '6a406a018db7a8b31600', 'naked vision bounded name'),
    (0x57dce3, '6a02e876d5ebff83c40450e875915700', 'naked vision integer milliseconds'),
    (0x57dcf6, '508bce33c0e8b0a3edff', 'naked vision slot0'),
]

def verify_naked_vision(read):
    index = read(0x57e2a4 + ord('N'), 1)[0]
    if index != 24 or struct.unpack('<I', read(0x57e1ac + index * 4, 4))[0] != 0x57dcc7:
        raise ValueError('naked command N dispatcher differs')
    for va, expected, meaning in NAKED_VISION_ANCHORS:
        if read(va, len(bytes.fromhex(expected))).hex() != expected:
            raise ValueError('naked vision consumer differs: ' + meaning)
    return dict(command='N', slot=0, name_storage_offset=0x16b3a8,
                transition_unit='milliseconds', consumers=[dict(va=va, bytes=raw, meaning=meaning)
                    for va, raw, meaning in NAKED_VISION_ANCHORS])


THERMAL_VISION_ANCHORS = [
    (0x57ddb4, '6a406a018db768b41600', 'thermal command reads bounded name into cg16b468'),
    (0x57ddd0, '6a02e889d4ebff83c40450e888905700', 'thermal parses integer milliseconds'),
    (0x57dde3, '508bceb803000000e8c0a2edff', 'thermal applies vision slot3'),
    (0x458573, '80bed0bf1600007407b903000000', 'active thermal renderer selects vision slot3'),
]

def verify_thermal_vision(read):
    index = read(0x57e2a4 + ord('Q'), 1)[0]
    if index != 27 or struct.unpack('<I', read(0x57e1ac + index * 4, 4))[0] != 0x57ddb4:
        raise ValueError('thermal command Q dispatcher differs')
    records = []
    for va, expected, meaning in THERMAL_VISION_ANCHORS:
        if read(va, len(bytes.fromhex(expected))).hex() != expected:
            raise ValueError('thermal vision consumer differs: ' + meaning)
        records.append(dict(va=va, bytes=expected, meaning=meaning))
    return dict(command='Q', slot=3, name_storage_offset=0x16b468,
                transition_unit='milliseconds', consumers=records)


FOF_ANCHORS = [
    (0x53f0b3, 'f647101074098b74241ce8ce3a0200',
     'CG playerState otherFlags16 dispatches FOF drawing'),
    (0x562b90, '5156e839ffffff83c40456e800feffff',
     'FOF draws local marker and other player markers'),
    (0x445a79, '6850f4c500a398f9a101e8f8db6500',
     'hostile FOF material registration'),
    (0x445a9b, '6880f4c500a3a0f9a101e8d6db6500',
     'self FOF material registration'),
    (0x562a4c, 'a19cf9a101', 'hostile FOF drawing material'),
    (0x562b36, '8b35a4f9a101', 'self FOF drawing material'),
]


STATIC_EXIT_ANCHORS = [
    (0x515696, '8b404c', 'MATERIAL HUD material index offset76'),
    (0x5156ab, '8d8803110000e89a41feff', 'MATERIAL HUD reads configstring4355+slot'),
    (0x514528, '8b482c80e1f080f940', 'HUD horizontal fullscreen alignment4'),
    (0x514533, 'd94708da4df8', 'HUD horizontal extent uses fullscreen scale'),
    (0x5145d8, '8b482c80e10f80f904', 'HUD vertical fullscreen alignment4'),
    (0x5145e3, 'd9470c', 'HUD vertical extent uses fullscreen scale'),
]


def verify_static_exit(read):
    records = []
    for address, expected_hex, meaning in STATIC_EXIT_ANCHORS:
        expected = bytes.fromhex(expected_hex)
        if read(address, len(expected)) != expected:
            raise ValueError(f'{meaning} differs at {address:#x}')
        records.append(dict(va=address, bytes=expected_hex, meaning=meaning))
    return dict(reference_duration_msec=500, archived=True, material_type=4,
                align_screen=0x44, width_offset=68, height_offset=72,
                width=640, height=480, sort=[10,20], material_configstring_base=4355,
                consumers=records, actual_material_capture_required=True,
                native_visual_acceptance=False)


def verify_fof(read):
    records = []
    for address, expected_hex, meaning in FOF_ANCHORS:
        expected = bytes.fromhex(expected_hex)
        if read(address, len(expected)) != expected:
            raise ValueError(f'{meaning} differs at {address:#x}')
        records.append(dict(va=address, bytes=expected_hex, meaning=meaning))
    for address, name in [(0xc5f450, b'hud_fofbox_hostile'),
                          (0xc5f480, b'hud_fofbox_self')]:
        if read(address, len(name) + 1) != name + b'\0':
            raise ValueError('native FOF material name differs')
    return dict(player_state_offset=16, descriptor_index=54, flag=16,
                reference_delay_msec=150, consumers=records,
                material_registration='client-owned native HUD registration',
                native_visual_acceptance=False)


def verify_vision(read):
    # Decode both dispatch levels: checking a case body alone does not prove
    # that command P can reach it in the target client.
    index = read(0x57e2a4 + ord('P'), 1)[0]
    if index != 26:
        raise ValueError('missilecam command P dispatch index differs')
    target = struct.unpack('<I', read(0x57e1ac + index * 4, 4))[0]
    if target != 0x57dd64:
        raise ValueError('missilecam command P handler differs')
    records = []
    for address, expected_hex, meaning in VISION_ANCHORS:
        expected = bytes.fromhex(expected_hex)
        if read(address, len(expected)) != expected:
            raise ValueError(f'{meaning} differs at {address:#x}')
        records.append(dict(va=address, bytes=expected_hex, meaning=meaning))
    return dict(command='P', slot=2, name_limit_bytes=64,
                transition_unit='milliseconds', name_storage_offset=0x16b428,
                handler_va=target, consumers=records,
                resource_binding_required=True, native_visual_acceptance=False)


def verify_image(read):
    code = []
    for address, expected_hex, meaning in ANCHORS:
        expected = bytes.fromhex(expected_hex)
        actual = read(address, len(expected))
        if actual != expected:
            raise ValueError(f'{meaning} differs at {address:#x}')
        code.append(dict(va=address, bytes=expected_hex, meaning=meaning,
                         sha256=hashlib.sha256(actual).hexdigest()))
    constants = {}
    for address, fmt, expected, name in [
        (0xc85f58, 'd', -127.0, 'remote_byte_divisor'),
        (0xc85608, 'd', 0.0010000000474974513, 'msec_to_seconds'),
        (0xb613f0, 'f', 100.0, 'linked_control_fov'),
        (0xc85828, 'f', 35.0, 'default_pitch_yaw_rate'),
        (0xc5aec8, 'f', 20.0, 'default_min_pitch'),
        (0xc862d8, 'f', 87.0, 'default_max_pitch'),
        (0xc855b4,'f',3000.0,'default_normal_speed'),
        (0xc85a04,'f',6000.0,'default_boost_speed'),
        (0xc85594,'f',2000.0,'default_acceleration'),
        (0xc855b8,'f',500.0,'default_deceleration'),

    ]:
        value = struct.unpack('<'+fmt, read(address, struct.calcsize(fmt)))[0]
        if value != expected:
            raise ValueError(f'{name} differs')
        constants[name] = dict(va=address, value=value)
    return code, constants


def prepare(client):
    import pefile
    raw = client.read_bytes()
    expected = next(f['sha256'] for f in manifest()['files']
                    if f['path'] == 'CODOL/codoMP_client_shipRetail.exe')
    if hashlib.sha256(raw).hexdigest() != expected:
        raise ValueError('expected unchanged target client')
    pe = pefile.PE(data=raw, fast_load=True)
    read = lambda address, size: pe.get_data(address - pe.OPTIONAL_HEADER.ImageBase, size)
    code, constants = verify_image(read)
    layout = json.loads((Path(__file__).resolve().parents[1] /
                         'fixtures/protocol/player-state-fields-3.3.3.3.json').read_bytes())
    table_bytes = read(layout['table_va'], len(layout['fields']) * 12)
    if hashlib.sha256(table_bytes).hexdigest() != layout['table_sha256']:
        raise ValueError('playerState descriptor differs')
    names = {12:'pm_flags', 16:'link_flags', 84:'camera_entity', 88:'camera_tag',
             92:'controls_entity', 512:'camera_reference_distinct'}
    fields = [dict(name=names[row[0]], offset=row[0], index=i, codec_bits=row[1])
              for i, row in enumerate(layout['fields']) if row[0] in names]
    return dict(schema_version=1, client_sha256=expected, native_gameplay_acceptance=False,
                evidence='Fixed target instruction checks; native link/payload gameplay remains unverified',
                player_state_descriptor_sha256=layout['table_sha256'], fields=fields,
                consumers=code, constants=constants, weapon_view=verify_weapon_view(read), missile_vision=verify_vision(read), thermal_vision=verify_thermal_vision(read), naked_vision=verify_naked_vision(read), fof_overlay=verify_fof(read), static_exit=verify_static_exit(read), camera=dict(flag=2,tag_angles_flag=128,
                tag_configstring_base=5379, tag_bank='immutable NCS category9', tag_slot_must_be_captured=True, configstring_override=False, entity_none=2047),
                controls=dict(pm_flag=0x80000, input_flag=0x200000,
                input_bytes=[0x27,0x28], signed=True, divisor=-127.0, entity_none=2047),
                unverified=['PS+0x5c controls entity routing semantics',
                            'native camera enter/exit and body restoration',
                            'missile/RC/gunner/AC130 payload authority and acceptance'])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--client', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = prepare(args.client)
    with args.output.open('x') as output:
        json.dump(result, output, indent=2)
        output.write('\n')
    print(json.dumps(dict(output=str(args.output), instructions=len(result['consumers']),
                         native_gameplay_acceptance=False)))


if __name__ == '__main__':
    main()
