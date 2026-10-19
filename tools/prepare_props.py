#!/usr/bin/env python3
"""Bind real map barrels and vehicles to native registrations and model bounds."""
import argparse
import base64
import hashlib
import json
import math
from pathlib import Path

from prepare_game_state import CLIENT_SHA256, compute
from prepare_map_hud import map_entities


def wreck_for(model_name, destructible_type, models):
    """Resolve a native wreck model using the map's registered naming variants."""
    candidates = []

    def add(value):
        if value and value != model_name and value not in candidates:
            candidates.append(value)

    # Most stock props use one of these direct suffix pairs.
    add(model_name.replace('_destructible_mp', '_dsr'))
    add(model_name.replace('_destructible_mp', '_destroyed_mp'))
    add(model_name.replace('_destructible_', '_destroyed_'))
    add(model_name.replace('_destructible', '_destroyed'))
    add(model_name.replace('_destructible', '_destroy'))
    add(model_name.replace('_destructible', '_dst'))
    # Some map entities use a short `veh_` model while the registered wreck
    # keeps the full `vehicle_` prefix.
    if model_name.startswith('veh_'):
        add('vehicle_' + model_name[4:] + '_destroyed')
        add('vehicle_' + model_name[4:] + '_dsr')
    # `destructible_type` is the stable semantic name for variants such as
    # vehicle_uaz_hardtop_destructible_mp.
    if destructible_type.startswith('vehicle_'):
        add(destructible_type + '_destroyed')
        add(destructible_type + '_dsr')
        add(destructible_type + '_destroyed_mp')
    return next((models[name] for name in candidates if name in models), None)


def prepare(assets_raw, entities_raw, constants_raw):
    assets, entities, constants = [json.loads(raw) for raw in (assets_raw, entities_raw, constants_raw)]
    for capture in (assets, entities, constants):
        if capture.get('schema_version') != 1 or capture.get('read_only') is not True or capture.get('client_sha256') != CLIENT_SHA256:
            raise ValueError('expected target read-only map prop captures')
    crc = compute(constants)['ncs_crc']
    if assets['connection_state'] != 10 or constants['initialized'] != 1 or assets['ncs_crc'] != crc:
        raise ValueError('prop resources require initialized matching map CRC')
    full_map = assets['map']
    if not full_map.startswith('maps/mp/') or not full_map.endswith('.d3dbsp'):
        raise ValueError('invalid prop map identity')
    name = full_map[8:-7]
    def bank(category):
        return [base64.b64decode(s, validate=True).decode('ascii') for a in constants['linked']
                if a['category'] == category for s in a['strings_base64']]
    def slot(category, token):
        values = bank(category)
        if values.count(token) != 1:
            raise ValueError('prop registration absent or ambiguous: ' + token)
        return values.index(token) + 1
    models = {}
    for model in assets['models']:
        if model['name'] in models or model['index'] != slot(0, model['name']):
            raise ValueError('prop model registration differs from native constants')
        low, high = model['bounds_min'], model['bounds_max']
        if len(low) != 3 or len(high) != 3 or any(not math.isfinite(v) or abs(v) > 2048 for v in low + high) or any(a >= b for a,b in zip(low,high)):
            raise ValueError('invalid native model bounds')
        models[model['name']] = model
    effects = {}
    for fx in assets['effects']:
        if fx['name'] in effects or fx['selector'] != slot(3,fx['name']):
            raise ValueError('prop effect registration differs from native constants')
        effects[fx['name'].lower()] = fx['selector']
    weapon = next((w for w in assets['weapons'] if w['name'] == 'destructible_car'), None)
    if weapon is None or weapon['index'] != slot(13,'destructible_car'):
        raise ValueError('missing native destructible damage identity')
    sounds = {s['name']:s for s in assets['sounds']}
    for alias in ('explo_metal_rand','car_explode'):
        if not sounds.get(alias,{}).get('found'):
            raise ValueError('missing native prop sound alias: '+alias)
    props, unsupported, seen = [], [], set()
    for fields in map_entities(entities,name):
        if fields.get(b'classname') != b'script_model': continue
        target = fields.get(b'targetname', b'')
        barrel = target == b'explodable_barrel' or fields.get(b'script_noteworthy') == b'explodable_barrel'
        vehicle = target == b'destructible_vehicle'
        if not (barrel or vehicle): continue
        model_name = fields[b'model'].decode('ascii')
        model = models.get(model_name)
        if model is None:
            unsupported.append(model_name); continue
        origin = [float(v) for v in fields[b'origin'].split()]
        angles = [float(v) for v in fields.get(b'angles',b'0 0 0').split()]
        if len(origin)!=3 or len(angles)!=3 or any(not math.isfinite(v) or abs(v)>32767 for v in origin) or any(not math.isfinite(v) or abs(v)>360000 for v in angles):
            raise ValueError('invalid map prop pose')
        key = (model_name, tuple(origin), tuple(angles))
        if key in seen: continue
        seen.add(key)
        wreck = None
        if vehicle:
            wreck = wreck_for(
                model_name,
                fields.get(b'destructible_type', b'').decode('ascii', errors='strict'),
                models,
            )
        props.append(dict(entity_number=256+len(props),kind='barrel' if barrel else 'vehicle',model=model['index'],
                          destroyed_model=None if wreck is None else wreck['index'],origin=[round(v) for v in origin],
                          angles=[int(v*65536/360)&65535 for v in angles],bounds_min=model['bounds_min'],bounds_max=model['bounds_max'],
                          destroyed_bounds=None if wreck is None else [wreck['bounds_min'],wreck['bounds_max']]))
    if len(props)>128: raise ValueError('map prop count exceeds native snapshot budget')
    if not props: raise ValueError('no supported real map barrels or vehicles')
    required=['props/barrel_ignite','props/barrel_fire_top','props/barrelexp','explosions/small_vehicle_explosion','smoke/car_damage_blacksmoke']
    needs_barrel=any(p['kind']=='barrel' for p in props)
    needs_vehicle=any(p['kind']=='vehicle' for p in props)
    needed=(required[:3] if needs_barrel else [])+(required[3:] if needs_vehicle else [])
    if any(f not in effects for f in needed): raise ValueError('missing registered prop effects')
    return dict(schema_version=1,client_sha256=CLIENT_SHA256,map=name,ncs_crc=crc,
                sources_sha256=[hashlib.sha256(raw).hexdigest() for raw in (assets_raw,entities_raw,constants_raw)],
                damage_weapon=weapon['index'],barrel_ignite_fx=effects.get(required[0],0),barrel_burn_fx=effects.get(required[1],0),barrel_explosion_fx=effects.get(required[2],0),
                vehicle_explosion_fx=effects.get(required[3],0),vehicle_smoke_fx=effects.get(required[4],0),
                barrel_sound_slot=480,vehicle_sound_slot=481,
                sound_aliases=[[480,'explo_metal_rand'],[481,'car_explode']],props=props,unsupported_models=sorted(set(unsupported)))


def prepare_cached(characters_raw, entities_raw, constants_raw, common_assets_raw, common_constants_raw):
    """Use retained native model bounds and the same verified common alias bank.

    The result cites all original captures; the temporary binding object is
    derived data and is never presented as a new process-memory capture.
    """
    characters, constants, shared, common = [json.loads(raw) for raw in
        (characters_raw, constants_raw, common_assets_raw, common_constants_raw)]
    if any(c.get('read_only') is not True or c.get('client_sha256') != CLIENT_SHA256 for c in (characters,constants,shared,common)):
        raise ValueError('cached prop bindings require target read-only originals')
    crc=compute(constants)['ncs_crc']
    if characters['ncs_crc']!=crc or constants['common_checksum']!=common['common_checksum']:
        raise ValueError('cached model/common prop resources differ')
    def bank(category):
        return [base64.b64decode(s,validate=True).decode('ascii') for a in constants['linked']
                if a['category']==category for s in a['strings_base64']]
    weapons=bank(13);effects=bank(3)
    names={e['name'].lower() for e in shared['effects']}
    derived=dict(schema_version=1,read_only=True,client_sha256=CLIENT_SHA256,connection_state=10,
                 ncs_crc=crc,map=characters.get('map',json.loads(entities_raw)['maps'][0]['name']),
                 models=[dict(name=m['name'],index=m['model'],bounds_min=m['bounds_min'],bounds_max=m['bounds_max'])
                         for m in characters['models'] if 'barrel' in m['name'] or m['name'].startswith(('vehicle_','veh_'))],
                 weapons=[dict(name='destructible_car',index=weapons.index('destructible_car')+1)],
                 effects=[dict(name=name,selector=i+1) for i,name in enumerate(effects) if name.lower() in names],
                 sounds=shared['sounds'])
    result=prepare(json.dumps(derived).encode(),entities_raw,constants_raw)
    result['sources_sha256']=[hashlib.sha256(raw).hexdigest() for raw in
                             (characters_raw,entities_raw,constants_raw,common_assets_raw,common_constants_raw)]
    return result


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ('assets','entities','constants','output'):parser.add_argument('--'+name,type=Path,required=True)
    args=parser.parse_args()
    result=prepare(args.assets.read_bytes(),args.entities.read_bytes(),args.constants.read_bytes())
    with args.output.open('x') as file:json.dump(result,file,indent=2);file.write('\n')
    print(json.dumps(dict(map=result['map'],props=len(result['props']),unsupported_models=result['unsupported_models'])))


if __name__=='__main__': main()
