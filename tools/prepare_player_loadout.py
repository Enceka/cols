#!/usr/bin/env python3
"""Prepare target-derived M4 backpack mappings; the Rust server reads live saves.

A preview is an explicit model-only diagnostic until its weapon timings have
been captured. It never edits profiles, inventory, or the source configuration.
"""
import argparse
import base64
import copy
import hashlib
import json
from pathlib import Path

from prepare_game_state import CLIENT_SHA256, weapon_ammo, weapon_parts
from prepare_inventory import public_snapshot
from schema_fields import flatten


def catalogue(assets_raw, parts_raw, state, class_index):
    assets = json.loads(assets_raw)
    if (assets.get('client_sha256') != CLIENT_SHA256 or assets.get('read_only') is not True
            or not 0 <= class_index < 60):
        raise ValueError('invalid schema capture or backpack index')
    definition = assets['schemas']['mp/playerdata.def']['definitions'][0]
    if (definition['version'], definition['formatChecksum']) != (233, 3811397846):
        raise ValueError('unsupported public definition')
    fields = flatten(definition)
    prefix = 'weaponClasses[%d].' % class_index
    names = [prefix + 'primaryBase'] + [prefix + 'primaryAttach[%d]' % i for i in range(6)]
    occupied = set()
    for name in names:
        field = fields[name]
        if field['type'] != 0 or field['width'] != 4:
            raise ValueError('expected Int32 backpack field')
        bits = set(range(field['offset'] * 8, (field['offset'] + 4) * 8))
        for other, f in fields.items():
            if other == name:
                continue
            start = f['offset'] * 8 + f['bit'] if f['type'] == 2 else f['offset'] * 8
            if bits.intersection(range(start, start + (1 if f['type'] == 2 else f['width'] * 8))):
                raise ValueError('backpack field overlaps ' + other)
        if bits & occupied:
            raise ValueError('overlapping backpack fields')
        occupied.update(bits)
    projected = [f for f in fields.values() if f['projected']]
    if len(projected) != 125 or any(f['type'] != 0 for f in projected):
        raise ValueError('unsupported public projections')
    weapon_parts(parts_raw, state['ncs_crc'])
    parts = json.loads(parts_raw)
    table = assets['tables']['mp/coMarketplaceItems.csv']
    if not table.get('found') or table['columns'] != 12 or len(table['cells_base64']) != table['rows'] * 12:
        raise ValueError('missing target marketplace table')
    market = {}
    for i in range(table['rows']):
        row = [base64.b64decode(v, validate=True).decode('ascii') for v in table['cells_base64'][i*12:(i+1)*12]]
        if int(row[0]) in market:
            raise ValueError('duplicate marketplace item')
        market[int(row[0])] = row
    if market[10001][1] != 'm4_mp':
        raise ValueError('M4 item mismatch')
    defaults = parts['m4_default_parts']['parts']
    compatible = []
    seen = set()
    for attachment in parts['compatible_attachments']:
        for item in attachment['items']:
            # The compact registry can alias unused ID ranges; accept only IDs
            # independently present under the same name in the marketplace.
            row = market.get(item['item_id'])
            if item['item_id'] == 20008 or not row or row[1] != attachment['name'] or row[2] != 'wat':
                continue
            if (item['name'] != row[1] or item['wire_index'] != attachment['index']
                    or not 0 <= item['kind'] < 16 or item['item_id'] in seen):
                raise ValueError('inconsistent attachment mapping')
            seen.add(item['item_id'])
            compatible.append(item)
    if not compatible:
        raise ValueError('no verified compatible attachments')
    return dict(class_index=class_index, primary_offset=fields[names[0]]['offset'],
                attachment_offsets=[fields[n]['offset'] for n in names[1:]],
                projection_offsets=[f['offset'] for f in projected],
                schema_source_sha256=hashlib.sha256(assets_raw).hexdigest(),
                parts_source_sha256=hashlib.sha256(parts_raw).hexdigest(),
                weapon_item_id=10001, defaults=defaults, compatible=compatible, variants=[])


def selection(config, profile):
    snapshot, tail = public_snapshot(profile)
    expected = b''.join(b'\x07' + snapshot[o:o+4] for o in config['projection_offsets']) + b'\0'
    if expected != tail:
        raise ValueError('public projections disagree with snapshot')
    integer = lambda o: int.from_bytes(snapshot[o:o+4], 'little')
    if integer(config['primary_offset']) != config['weapon_item_id']:
        raise ValueError('backpack primary is not M4')
    parts = {p['kind']: p for p in config['defaults']}
    selected = []
    kinds = set()
    for offset in config['attachment_offsets']:
        item = integer(offset)
        if item in (0, 20008):
            continue
        found = [p for p in config['compatible'] if p['item_id'] == item]
        if len(found) != 1:
            raise ValueError('unsupported M4 attachment: %d' % item)
        part = found[0]
        if part['kind'] in kinds:
            raise ValueError('conflicting backpack attachment slots')
        kinds.add(part['kind'])
        parts[part['kind']] = part
        selected.append(item)
    return [p['wire_index'] for _, p in sorted(parts.items())], selected


def set_parts(weapon, attachments, source):
    weapon.update(attachments=attachments, attachment_source_sha256=source, camo=0,
                  composed_name='** %d 0 0%s' % (weapon['base_index'], ''.join(' %d' % a for a in attachments)))


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--state', type=Path, required=True)
    p.add_argument('--assets', type=Path, required=True)
    p.add_argument('--parts', type=Path)
    p.add_argument('--arsenal', type=Path, help='general weapon definitions and attributes; replaces measured variants')
    p.add_argument('--collision', type=Path, help='actual training map brushes and terrain')
    p.add_argument('--backpack', type=int, default=1, help='training backpack number, 1..60 (default 1)')
    p.add_argument('--runtime', type=Path, action='append', default=[])
    p.add_argument('--preview-profile', type=Path, help='model-only preview for measuring this saved combination')
    p.add_argument('--output', type=Path, required=True)
    args = p.parse_args()
    for path, budget in [(args.state, 65536), (args.assets, 16*1024*1024), (args.parts, 8*1024*1024), (args.arsenal, 16*1024*1024), (args.collision,64*1024*1024)]:
        if path is None:continue
        if path.stat().st_size > budget:
            p.error('input exceeds capture budget')
    if args.output.exists():
        p.error('choose a new output; existing configuration is preserved')
    state = json.loads(args.state.read_bytes())
    if state.get('schema_version') != 1 or state.get('client_sha256') != CLIENT_SHA256:
        p.error('invalid prepared state identity')
    if args.arsenal:
        if args.preview_profile or args.runtime:
            p.error('arsenal mode resolves live saves; preview/runtime presets are not used')
        assets_raw=args.assets.read_bytes();assets=json.loads(assets_raw)
        definition=assets['schemas']['mp/playerdata.def']['definitions'][0]
        if assets.get('read_only') is not True or assets.get('client_sha256')!=CLIENT_SHA256 or (definition['version'],definition['formatChecksum'])!=(233,3811397846):
            p.error('unsupported schema evidence')
        if not 1<=args.backpack<=60:p.error('backpack range')
        fields=flatten(definition);prefix='weaponClasses[%d].'%(args.backpack-1)
        projected=[f for f in fields.values() if f['projected']]
        names=[prefix+w+s for w in ['primary','secondary'] for s in ['Base']+['Attach[%d]'%i for i in range(6)]]
        offsets=[]
        for name in names:
            f=fields[name]
            if f['type']!=0 or f['width']!=4:p.error('backpack field is not Int32')
            own=set(range(f['offset']*8,(f['offset']+4)*8))
            for other,v in fields.items():
                if other==name:continue
                at=v['offset']*8+(v['bit'] if v['type']==2 else 0)
                if own.intersection(range(at,at+(1 if v['type']==2 else v['width']*8))):p.error('overlapping backpack schema: '+name)
            offsets.append(f['offset'])
        if len(projected)!=125 or any(f['type']!=0 for f in projected):p.error('public projection schema')
        arsenal_raw=args.arsenal.read_bytes();arsenal=json.loads(arsenal_raw)
        if arsenal.get('client_sha256')!=CLIENT_SHA256 or arsenal.get('read_only') is not True or arsenal.get('ncs_crc')!=state['ncs_crc']:p.error('arsenal identity/CRC')
        state['player_loadout']=dict(class_index=args.backpack-1,primary_offset=offsets[0],attachment_offsets=offsets[1:7],secondary_offset=offsets[7],secondary_attachment_offsets=offsets[8:],projection_offsets=[f['offset'] for f in projected],schema_source_sha256=hashlib.sha256(assets_raw).hexdigest(),parts_source_sha256=hashlib.sha256(arsenal_raw).hexdigest())
        state['arsenal_file']=str(args.arsenal.resolve())
        if args.collision:state['collision_file']=str(args.collision.resolve())
        args.output.write_text(json.dumps(state,indent=2)+'\n')
        print(json.dumps(dict(mode='dynamic_resource_loadout',backpack=args.backpack,weapons=len(arsenal['weapons']),collision=bool(args.collision))))
        return
    if not args.parts:p.error('provide arsenal or legacy parts capture')
    config = catalogue(args.assets.read_bytes(), args.parts.read_bytes(), state, args.backpack-1)
    weapon = state['bootstrap_weapon']
    if args.preview_profile:
        if args.preview_profile.stat().st_size > 61440:
            p.error('profile exceeds capture budget')
        attachments, selected = selection(config, args.preview_profile.read_bytes())
        set_parts(weapon, attachments, config['parts_source_sha256'])
        weapon.pop('ammo', None)
        state.pop('player_loadout', None)
        print(json.dumps(dict(mode='model_only_preview', selected_items=selected, composed_name=weapon['composed_name'])))
    else:
        if weapon.get('ammo'):
            config['variants'].append(dict(attachments=weapon['attachments'], ammo=weapon['ammo']))
        for path in args.runtime:
            if path.stat().st_size > 4096:
                p.error('runtime capture exceeds budget')
            raw = path.read_bytes()
            name = json.loads(raw)['composed_weapon']['name']
            tokens = name.split(' ')
            if tokens[:4] != ['**', str(weapon['base_index']), '0', '0']:
                p.error('runtime is not this uncamoed M4')
            attachments = [int(n) for n in tokens[4:]]
            candidate = copy.deepcopy(weapon)
            set_parts(candidate, attachments, config['parts_source_sha256'])
            ammo = weapon_ammo(raw, state['ncs_crc'], candidate)
            if any(v['attachments'] == attachments for v in config['variants']):
                p.error('duplicate runtime variant')
            config['variants'].append(dict(attachments=attachments, ammo=ammo))
        state['player_loadout'] = config
        print(json.dumps(dict(mode='saved_player_backpack', backpack=args.backpack, compatible_items=len(config['compatible']), measured_variants=len(config['variants']))))
    args.output.write_text(json.dumps(state, indent=2) + '\n')


if __name__ == '__main__':
    main()
