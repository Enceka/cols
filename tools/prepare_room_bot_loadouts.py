#!/usr/bin/env python3
"""Bind existing Bot identities to native frontend item IDs without changing gameplay."""
import argparse
import copy
import hashlib
import json
from pathlib import Path

from prepare_bot_roster import rows
from prepare_game_state import CLIENT_SHA256

# The room has a cha package and a pat slot; clientState instead has a bar group.
GROUP_KINDS = [1, 2, 3, 4, 5, 6, 7, 8, 9, 11, 12]
ROOM_SLOTS = {kind: kind for kind in range(1, 10)} | {11: 11}
SKINS = [0, 3, 1, 2]


def bind(state, appearance, arsenal):
    for value in (state, appearance, arsenal):
        if value.get('client_sha256') != CLIENT_SHA256:
            raise ValueError('room avatar client identity differs')
    if (appearance.get('schema_version') != 1 or arsenal.get('read_only') is not True
            or arsenal.get('ncs_crc') != state['ncs_crc']):
        raise ValueError('room avatar appearance/arsenal identity differs')
    market = {}
    for row in rows(arsenal['marketplace']):
        if len(row) != 12:
            raise ValueError('room avatar marketplace layout differs')
        identifier = int(row[0])
        if identifier in market:
            raise ValueError('duplicate room avatar marketplace ID')
        market[identifier] = row
    weapons = {w['name']: w for w in arsenal['weapons']}
    if len(weapons) != len(arsenal['weapons']):
        raise ValueError('duplicate room avatar weapon')
    items = {}
    for item in appearance['items']:
        key = (item['kind'], item['native_index'])
        if key in items:
            raise ValueError('ambiguous room avatar wearable index')
        items[key] = item
    roster = copy.deepcopy(state['match_config']['roster'])
    if len(roster) != state['match_config'].get('player_count', 12) - 1 or not roster:
        raise ValueError('room avatar roster count differs')
    result = []
    for bot in roster:
        weapon = weapons.get(bot['weapon_name'])
        if weapon is None:
            raise ValueError('room avatar weapon not captured: ' + bot['weapon_name'])
        weapon_ids = [i for i in weapon['item_ids'] if i in market
                      and market[i][1] == bot['weapon_name'] and market[i][2] == 'wea']
        if len(weapon_ids) != 1 or not 0 < weapon_ids[0] <= 65535:
            raise ValueError('room avatar weapon item absent/ambiguous')
        if len(bot.get('outfits', [])) != 2:
            raise ValueError('room avatar requires both native faction outfits')
        loadouts = []
        for faction, outfit in enumerate(bot['outfits']):
            character = outfit.get('character')
            if (not character or len(character['groups']) != 11 or character['faction'] != 1 - faction
                    or character['gender'] not in (0, 1) or character['skin'] not in range(4)
                    or character['bits'] != appearance['packing_bits'] or character['groups'][10]):
                raise ValueError('room avatar native character identity differs')
            selected = {}
            for kind, index in zip(GROUP_KINDS, character['groups']):
                if index:
                    item = items.get((kind, index))
                    if item is None or not 0 < item['id'] <= 32767:
                        raise ValueError('room avatar wearable item absent')
                    selected[kind] = item
            if any(kind not in selected for kind in range(1, 6)):
                raise ValueError('room avatar body is incomplete')
            selected_ids = {i['id'] for i in selected.values()}
            packages = [p for p in appearance['packages']
                        if p['gender'] == character['gender'] and SKINS[p['skin']] == character['skin']
                        and selected_ids <= set(p['allowed']) | set(p['defaults'])]
            if len(packages) != 1 or not 0 < packages[0]['id'] <= 32767:
                raise ValueError('room avatar package absent/ambiguous for existing Bot identity')
            mask = ((0x80000000 >> character['gender']) | (0x80000000 >> (3 - faction))
                    | (0x80000000 >> (5 + character['skin'])))
            for item in selected.values():
                if not any(v['mask'] & mask == mask for v in item['variants']):
                    raise ValueError('room avatar package has no matching native model variant')
            clothing = [0] * 12
            clothing[0] = packages[0]['id']
            for kind, item in selected.items():
                clothing[ROOM_SLOTS[kind]] = item['id']
            # Prepared bots currently carry their class primary without explicit
            # attachments or a secondary. The frontend fills its native defaults.
            loadouts.append(dict(clothing=clothing,
                                 weapons=[[weapon_ids[0]] + [0] * 6, [0] * 7]))
        bot['room_loadouts'] = loadouts
        result.append(dict(class_name=bot['class'], weapon_item=weapon_ids[0],
                           character_packages=[v['clothing'][0] for v in loadouts]))
    state['match_config']['roster'] = roster
    return dict(bots=len(roster), identities=result)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--game-state', type=Path, required=True)
    parser.add_argument('--appearance', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    state = json.loads(args.game_state.read_text())
    appearance_path = args.appearance or args.game_state.parent / state['appearance_file']
    arsenal_path = args.game_state.parent / state['arsenal_file']
    result = bind(state, json.loads(appearance_path.read_text()), json.loads(arsenal_path.read_text()))
    # A new output may live elsewhere; preserve every referenced preparation.
    for name, value in list(state.items()):
        if name.endswith('_file') and isinstance(value, str):
            state[name] = str((args.game_state.parent / value).resolve())
    with args.output.open('x') as target:
        json.dump(state, target, separators=(',', ':'))
        target.write('\n')
    result['sources_sha256'] = [hashlib.sha256(p.read_bytes()).hexdigest()
                                for p in (args.game_state, appearance_path, arsenal_path)]
    print(json.dumps(result))


if __name__ == '__main__':
    main()
