#!/usr/bin/env python3
"""Prepare minimal registered reward resources from a matching initialized map."""
import argparse
import base64
import hashlib
import json
import math
import re
from pathlib import Path
import struct
import zlib
from prepare_game_state import compute, CLIENT_SHA256
from prepare_map_hud import map_entities
from prepare_flight_nodes import prepare as prepare_flight_nodes

CATALOG = Path(__file__).resolve().parents[1] / 'fixtures/protocol/killstreaks-3.3.3.3.json'


def captured_rawfile(record, name, limit):
    if record.get('found') is not True or record.get('name') != name:
        raise ValueError('invalid remote vision rawfile identity')
    size, compressed = record.get('length'), record.get('compressed_length')
    if type(size) is not int or not 1 <= size <= limit or type(compressed) is not int or not 0 <= compressed <= limit:
        raise ValueError('remote vision rawfile exceeds budget')
    payload = base64.b64decode(record['buffer_base64'], validate=True)
    if len(payload) != (compressed or size):
        raise ValueError('truncated remote vision rawfile')
    if compressed:
        decoder = zlib.decompressobj()
        payload = decoder.decompress(payload, size + 1)
        if not decoder.eof or decoder.unused_data or decoder.unconsumed_tail:
            raise ValueError('invalid remote vision compression stream')
    if len(payload) != size or not payload.rstrip(b'\0').strip():
        raise ValueError('invalid remote vision content length')
    payload.rstrip(b'\0').decode('ascii')
    return payload


def arena_thermal(arena, map_name):
    # Alpha getMapCustom reads mp/basemaps.arena. Require the actual map record;
    # never infer inversion from a map name or from assets merely being loaded.
    pattern = r'\s+|//[^\n]*|/\*[\s\S]*?\*/|"([^"\r\n]*)"|([{}])|((?!//|/\*)[^\s{}"]+)'
    text = arena.rstrip(b'\0').decode('ascii')
    if '\0' in text:
        raise ValueError('embedded NUL in arena')
    tokens, pos = [], 0
    for match in re.finditer(pattern, text):
        if match.start() != pos:
            raise ValueError('invalid arena token')
        pos = match.end()
        token = next((x for x in match.groups() if x is not None), None)
        if token is not None:
            tokens.append(token)
    if pos != len(text):
        raise ValueError('invalid arena tail')
    rows, i = [], 0
    while i < len(tokens):
        if tokens[i] != '{':
            raise ValueError('arena block expected')
        i += 1
        row = {}
        while i < len(tokens) and tokens[i] != '}':
            if i + 1 >= len(tokens) or tokens[i] in ('{', '}') or tokens[i+1] in ('{', '}'):
                raise ValueError('invalid arena pair')
            key, value = tokens[i].lower(), tokens[i+1]
            if key in row:
                raise ValueError('duplicate arena key')
            row[key] = value
            i += 2
        if i == len(tokens):
            raise ValueError('unterminated arena block')
        i += 1
        if row.get('map', '').lower() == map_name.lower():
            rows.append(row)
    if len(rows) != 1:
        raise ValueError('remote vision requires unique captured arena map')
    return rows[0].get('thermal', '') == 'invert'


def remote_vision(rawfiles, map_name):
    arena_name = 'mp/basemaps.arena'
    record = rawfiles.get(arena_name)
    if record is None:
        return None
    arena_absent = record.get('found') is False
    if arena_absent:
        # Reference getMapCustom returns an empty value when the arena asset
        # is absent. This is a captured negative lookup, not an uncaptured map.
        arena = None
        flight = 'thermal_mp'
    else:
        arena = captured_rawfile(record, arena_name, 2 * 1024 * 1024)
        flight = 'thermal_snowlevel_mp' if arena_thermal(arena, map_name) else 'thermal_mp'
    required = ['vision/black_bw.vision', 'vision/' + flight + '.vision']
    records = [rawfiles.get(name) for name in required]
    if any(record is None or record.get('found') is False for record in records):
        return None
    hashes = {} if arena is None else {arena_name: hashlib.sha256(arena).hexdigest()}
    for name, record in zip(required, records):
        payload = captured_rawfile(record, name, 256 * 1024)
        hashes[name] = hashlib.sha256(payload).hexdigest()
    inverted = rawfiles.get('vision/missilecam.vision')
    if inverted is not None and inverted.get('found') is True:
        payload = captured_rawfile(inverted, 'vision/missilecam.vision', 256 * 1024)
        hashes['vision/missilecam.vision'] = hashlib.sha256(payload).hexdigest()
    return dict(flight=flight, arena_absent=arena_absent, rawfiles_sha256=hashes)


def prepare(assets_raw, ncs_raw, catalog=None, entities_raw=None):
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
                          fire_delay_msec=weapon['runtime_fields'].get('fire_delay_msec', 0),
                          clip_size=u(0xd08), reload_msec=u(0x71c),
                          radius=u(0x8d8), inner_damage=u(0x8e0), outer_damage=u(0x8e4),
                          projectile_speed=u(0x8f4),
                          damage_stages=list(struct.unpack_from('<6I', raw, 0xbb4)),
                          damage_ranges=list(struct.unpack_from('<6f', raw, 0xbcc)))
        if name in ('mg4juggernaut_mp', 'minigungunship_mp'):
            runtime = weapon['runtime_fields']
            normal, empty = u(0x71c), u(0x728)
            ads = None
            if raw[0xd76]:
                ads = dict(in_rate=struct.unpack_from('<f', raw, 0xbac)[0],
                           out_rate=struct.unpack_from('<f', raw, 0xbb0)[0],
                           reload_allowed=bool(raw[0xd78]), reload_interruptible=bool(raw[0xd86]),
                           reload_end_msec=u(0xb38))
            if runtime['fire_delay_msec'] != 0 or u(0x48c) not in (0, 6):
                raise ValueError('unsupported armored weapon trigger')
            words[name]['operator'] = dict(fire_type=u(0x48c),
                move_scale=struct.unpack_from('<f', raw, 0x7c8)[0],
                ads_move_scale=struct.unpack_from('<f', raw, 0x7cc)[0],
                reload=dict(normal_msec=normal, empty_msec=empty,
                    add_msec=min(u(0x730) or normal, normal),
                    empty_add_msec=min(u(0x738) or empty, empty)), ads=ads,
                location_multipliers=list(struct.unpack_from('<20f', raw, 0xbf0)))
        if u(0x47c)==2:
            seconds=struct.unpack_from('<f',raw,0x904)[0]
            if not math.isfinite(seconds) or seconds<0 or seconds>3600 or u(0x914)>7:
                raise ValueError('invalid captured projectile lifetime or explosion kind')
            if seconds>0:
                words[name]['projectile']=dict(lifetime_msec=max(1,round(seconds*1000)),explosion_kind=u(0x914))
        if name in ['airdrop_marker_mp', 'airdrop_mega_marker_mp', 'airdrop_sentry_marker_mp', 'airdrop_sam_marker_mp']:
            if u(0x47c) != 1 or u(0x480) != 6 or u(0x490) != 0 or u(0x8d8) != 0 or raw[0xd7a] not in (0, 1):
                raise ValueError('unexpected native marker definition: ' + name)
            words[name]['marker'] = dict(prepare_msec=u(0x70c), throw_msec=u(0xd10), throw_delay_msec=u(0x6f0),
                fuse_msec=u(0x7ac), cook=bool(raw[0xd7a]), speed_up=u(0x8f8), speed_forward=u(0x8fc), stickiness=u(0x928),
                parallel=list(struct.unpack_from('<31f',raw,0x934)), perpendicular=list(struct.unpack_from('<31f',raw,0x9b0)))
    radios, unavailable = [], []
    for reward in catalog['rewards']:
        if reward['native_id'] == 0:
            continue
        definition = words.get(reward['weapon_token_c13'])
        if definition is None:
            unavailable.append(reward['native_id']); continue
        radios.append(dict(native_id=reward['native_id'], **{k: definition[k] for k in ['weapon','switch','fire_interval_msec']}))
    models, model_bounds, camera_mounts = {}, {}, {}
    model_names = bank(0)
    captured_registered = {(m['name'], m['index']) for m in assets['models']}
    for model in assets['models']:
        if model['name'] not in model_names: continue
        expected = slot(0, model['name'])
        # Outfit instances may copy an already registered model name into a
        # dynamic slot. Accept only when its actual NCS binding was captured too.
        if len(model_names) < model['index'] < 1024 and (model['name'], expected) in captured_registered:
            continue
        if model['name'] in models or model['index'] != slot(0, model['name']):
            raise ValueError('model registration mismatch')
        models[model['name']] = model['index']
        mount = model.get('attachment_bind_matrices', {}).get('tag_player')
        if mount is not None:
            if (len(mount) != 8 or not all(math.isfinite(v) for v in mount)
                    or not .5 <= sum(v*v for v in mount[:4]) <= 2
                    or any(abs(v)>30000 for v in mount[4:7])):
                raise ValueError('invalid captured camera mount')
            camera_mounts[model['name']] = dict(rotation=mount[:4], translation=mount[4:7])
        center, half = model['bounds_center'], model['bounds_half']
        if len(center) != 3 or len(half) != 3 or not all(math.isfinite(x) for x in center + half) or not all(h >= 0 for h in half):
            raise ValueError('invalid registered model bounds')
        low, high = [c-h for c,h in zip(center, half)], [c+h for c,h in zip(center, half)]
        if all(h > 0 for h in half) and all(abs(x) <= 4096 for x in low + high):
            model_bounds[model['name']] = dict(min=low, max=high)
    vehicles = {}
    for vehicle in assets.get('vehicles', []):
        name, index = vehicle['name'], vehicle['index']
        if name == 'defaultvehicle':
            if index != 1 or index != slot(2, name): raise ValueError('invalid default vehicle slot')
            continue
        # Category2 includes the code-owned defaultvehicle at slot1 before
        # the common multiplayer definitions. Do not count just one NCS file.
        if name in vehicles or index != slot(2, name) or not 2 <= index < 32:
            raise ValueError('vehicle registration mismatch')
        vehicles[name] = index
    tag_names=bank(9)
    if len(tag_names)>63 or len(set(tag_names))!=len(tag_names):
        raise ValueError('invalid native camera tag bank')
    tags={name:i+1 for i,name in enumerate(tag_names)}
    animations = {}
    for tree in assets.get('vehicle_animtrees', []):
        index = tree['index']
        raw = bytes.fromhex(tree['layout']['hex'])
        if not 1 <= index < 16 or len(raw) < 8:
            raise ValueError('invalid vehicle animation tree')
        count = struct.unpack_from('<I', raw)[0]
        if not 1 <= count <= 2048: raise ValueError('invalid vehicle animation node count')
        for entry in tree['layout'].get('pointers', []):
            offset = entry['offset']
            if offset < 8 or offset % 8 or offset >= 8 + count * 8: continue
            names = [p['string'] for p in entry.get('target', {}).get('pointers', []) if p['offset'] == 0 and 'string' in p]
            if names != ['bh_rotors']: continue
            if 'bh_rotors' in animations: raise ValueError('ambiguous helicopter rotor animation')
            node = (offset - 8) // 8
            if offset + 4 > len(raw): raise ValueError('truncated helicopter rotor animation entry')
            parent = struct.unpack_from('<H', raw, offset - 2)[0]
            if parent >= count or parent == node:
                raise ValueError('invalid helicopter rotor animation parent')
            animations['bh_rotors'] = dict(tree=index, node=node, parent=parent)
    animal_resources = None
    if assets.get('actor_animlist_mask', 0) & 2:
        selectors = [a for a in assets.get('actor_trees', []) if a['selector'] == 1]
        if len(selectors) == 1 and len(selectors[0]['registered_tree_ids']) == 1:
            tree_id = selectors[0]['registered_tree_ids'][0]
            trees = [t for t in assets.get('vehicle_animtrees', []) if t['index'] == tree_id]
            if len(trees) != 1: raise ValueError('animal composition tree absent')
            tree = trees[0]
            actor_tree = selectors[0]
            raw = bytes.fromhex(actor_tree['tree_layout']['hex'])
            count = struct.unpack_from('<I', raw)[0]
            parents = actor_tree.get('parents')
            if (not isinstance(parents, list) or len(parents) != count or not 1 < count <= 2048
                    or any(type(v) is not int or not 0 <= v < count for v in parents)):
                raise ValueError('animal ancestor capture missing or invalid')
            needed = {'idle':'german_shepherd_idle', 'run':'german_shepherd_run',
                      'bite':'german_shepherd_attack_player', 'death':'german_shepherd_death_front'}
            clips, ancestors, durations = {}, {}, {}
            for key, name in needed.items():
                entries = [p for p in actor_tree.get('parts', []) if p['name'] == name]
                if len(entries) != 1: raise ValueError('animal clip absent or ambiguous: ' + name)
                node = entries[0]['node']
                if not 0 < node < count: raise ValueError('animal tree entry outside capture')
                if not any(p['name'] == name and p['node'] == node for p in tree.get('parts', [])):
                    raise ValueError('animal tree registry mismatch')
                parent = entries[0]['parent']
                if parent >= count or parent == node: raise ValueError('animal parent invalid')
                if parent != parents[node]: raise ValueError('animal parent capture mismatch')
                chain, cursor = [], parent
                while cursor:
                    if cursor == node or cursor in chain or len(chain) == 4:
                        raise ValueError('animal ancestor cycle or depth')
                    chain.append(cursor)
                    cursor = parents[cursor]
                ancestors[key] = list(reversed(chain))
                frames, rate = entries[0]['frames'], entries[0]['frame_rate']
                if (type(frames) is not int or frames <= 0 or not math.isfinite(rate) or rate <= 0
                        or not 1 <= math.ceil(frames / rate * 1000) <= 15000):
                    raise ValueError('invalid animal animation duration')
                durations[key] = math.ceil(frames / rate * 1000)
                clips[key] = dict(tree=tree_id,node=node,parent=parent)
            files = {}
            for name in ['animlists/common_dog.animlist', 'animtrees/dog.atr']:
                payload = captured_rawfile(assets.get('actor_rawfiles', {}).get(name, {}), name, 256*1024)
                files[name] = hashlib.sha256(payload).hexdigest()
            animal_resources = dict(mask=2, selector=1, clips=clips, ancestors=ancestors,
                durations=durations, rawfiles_sha256=files)
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
                  map=path[8:-7], ncs_crc=crc, radios=radios, models=models, model_bounds=model_bounds, effects=effects,
                  vehicles=vehicles, animations=animations, tags=tags, camera_mounts=camera_mounts)
    vision = remote_vision(assets.get('remote_vision_rawfiles', {}), result['map'])
    if vision is not None:
        result['remote_vision'] = vision
    exit_materials = {}
    for material in assets.get('remote_material_assets', []):
        name = material.get('name')
        if name not in ('white', 'ac130_overlay_grain'):
            continue
        if name in exit_materials:
            raise ValueError('duplicate remote exit material capture')
        exit_materials[name] = material
    if all(exit_materials.get(name, {}).get('found') is True and
           exit_materials[name].get('loaded') is True for name in ('white', 'ac130_overlay_grain')):
        count = len(bank(8))
        if count >= 1000:
            raise ValueError('remote exit material slots overlap immutable NCS')
        result['remote_exit'] = dict(white=1000, grain=1001, material_bank_count=count)
    selectors = bank(12)
    if len(selectors) > 3 or len(set(selectors)) != len(selectors):
        raise ValueError('invalid native location selector bank')
    result['selectors'] = {name: i + 1 for i, name in enumerate(selectors)}
    # type13 loopSound is an eight-bit dynamic selector, not NCS512+index.
    # Publish only aliases actually found by the matching read-only sampler.
    result['loop_sounds'] = {'weap_cobra_20mm_fire_npc': 240} if any(
        sound.get('name') == 'weap_cobra_20mm_fire_npc' and sound.get('found') is True
        for sound in assets.get('sounds', [])) else {}

    if entities_raw is not None:
        capture = json.loads(entities_raw)
        anchors = [e for e in map_entities(capture, result['map']) if e.get(b'targetname') == b'airstrikeheight']
        if len(anchors) != 1:
            raise ValueError('expected one captured air-support height anchor')
        center = [float(x) for x in anchors[0][b'origin'].split()]
        if len(center) != 3 or not all(math.isfinite(x) and abs(x) <= 30000 for x in center):
            raise ValueError('invalid captured air-support anchor')
        result['air_route'] = dict(center=center, sources_sha256=[hashlib.sha256(entities_raw).hexdigest()])
        entities = map_entities(capture, result['map'])
        def position(entity):
            point=[float(x) for x in entity.get(b'origin',b'').split()]
            if len(point)!=3 or not all(math.isfinite(x) and abs(x)<=30000 for x in point):
                raise ValueError('invalid remote missile spawn position')
            return point
        remote=[]
        for spawn in entities:
            if spawn.get(b'targetname')!=b'remoteMissileSpawn': continue
            targets=[e for e in entities if e.get(b'targetname')==spawn.get(b'target') and spawn.get(b'target')]
            if len(targets)!=1: raise ValueError('remote missile spawn requires one captured target')
            origin,target=position(spawn),position(targets[0])
            if sum((a-b)**2 for a,b in zip(origin,target))<=1: raise ValueError('remote missile spawn direction is degenerate')
            remote.append(dict(origin=origin,target=target))
        if len(remote)>64: raise ValueError('remote missile spawn budget')
        result['remote_spawns']=dict(sources_sha256=[hashlib.sha256(entities_raw).hexdigest()],points=remote)
        paths = prepare_flight_nodes(capture, result['map'], entities_raw)
        if paths is not None:
            result['air_paths'] = paths
    if animal_resources is not None:
        result['animals'] = animal_resources
    # Companion weapon facts remain separate from activator timings.
    relevant = {r['weapon_token_c13'] for r in catalog['rewards']}
    prefixes = ('ac130_', 'cobra_', 'harrier_', 'pavelow_', 'sentry_', 'tow_turret_', 'remotemissile_', 'remote_missile', 'rcbomb_', 'missile_drone_', 'mg4juggernaut', 'minigungunship', 'kingpin_spawn', 'artillery_', 'airstrike_', 'stealth_bomb_', 'dog_bite', 'littlebird_')
    result['weapons'] = {name: data for name, data in words.items() if name in relevant or name.startswith(prefixes)}
    return result, unavailable


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--assets', type=Path, required=True); p.add_argument('--ncs', type=Path, required=True)
    p.add_argument('--entities', type=Path, help='matching captured map entities with the native air-support anchor')
    p.add_argument('--output', type=Path, required=True); args = p.parse_args()
    result, unavailable = prepare(args.assets.read_bytes(), args.ncs.read_bytes(), entities_raw=args.entities.read_bytes() if args.entities else None)
    with args.output.open('x') as f: json.dump(result, f, indent=2); f.write('\n')
    print(json.dumps(dict(map=result['map'], activators=len(result['radios']), unavailable=unavailable)))


if __name__ == '__main__': main()
