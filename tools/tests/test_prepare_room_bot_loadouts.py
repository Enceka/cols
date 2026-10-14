import base64
import copy
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from prepare_room_bot_loadouts import bind, CLIENT_SHA256


def inputs():
    character = dict(groups=[7] * 5 + [0] * 6, bits=[3] * 5 + [0] * 6,
                     gender=0, faction=1, skin=0)
    bot = dict(class_name='unused', weapon_name='rifle_mp',
               outfits=[dict(character=character),
                        dict(character=dict(character, faction=0))])
    state = dict(client_sha256=CLIENT_SHA256, ncs_crc=99,
                 match_config=dict(player_count=4, roster=[dict(copy.deepcopy(bot), **{'class': 'botclass' + str(i)})
                                                          for i in range(3)]))
    appearance = dict(schema_version=1, client_sha256=CLIENT_SHA256, packing_bits=character['bits'],
        items=[dict(id=100 + kind, kind=kind, native_index=7,
                    variants=[dict(mask=0xb4000000, model='body' + str(kind))]) for kind in range(1, 6)],
        packages=[dict(id=500, gender=0, skin=0, defaults=[0] + list(range(101, 106)) + [0] * 6,
                       allowed=[])])
    row = ['10001', 'rifle_mp', 'wea'] + ['0'] * 9
    arsenal = dict(client_sha256=CLIENT_SHA256, read_only=True, ncs_crc=99,
        marketplace=dict(found=True, columns=12, rows=1,
                         cells_base64=[base64.b64encode(v.encode()).decode() for v in row]),
        weapons=[dict(name='rifle_mp', item_ids=[10001])])
    return state, appearance, arsenal


class RoomBotLoadouts(unittest.TestCase):
    def test_variant_indices_become_owned_package_and_weapon_items_without_changing_gameplay(self):
        state, appearance, arsenal = inputs()
        before = copy.deepcopy(state)
        result = bind(state, appearance, arsenal)
        self.assertEqual(result['bots'], 3)
        for old, bot in zip(before['match_config']['roster'], state['match_config']['roster']):
            self.assertEqual({k: v for k, v in bot.items() if k != 'room_loadouts'}, old)
            self.assertEqual(len(bot['room_loadouts']), 2)
            for loadout in bot['room_loadouts']:
                self.assertEqual(loadout['clothing'], [500, 101, 102, 103, 104, 105] + [0] * 6)
                self.assertEqual(loadout['weapons'], [[10001] + [0] * 6, [0] * 7])

    def test_wrong_package_skin_gender_membership_and_ambiguous_items_fail_atomically(self):
        for issue in ['gender', 'skin', 'membership', 'package', 'wearable', 'weapon', 'market', 'map', 'faction', 'model']:
            state, appearance, arsenal = inputs()
            if issue == 'gender': appearance['packages'][0]['gender'] = 1
            if issue == 'skin': appearance['packages'][0]['skin'] = 1
            if issue == 'membership': appearance['packages'][0]['defaults'][3] = 999
            if issue == 'package': appearance['packages'].append(dict(appearance['packages'][0], id=501))
            if issue == 'wearable': appearance['items'].append(dict(appearance['items'][0], id=999))
            if issue == 'weapon': arsenal['weapons'][0]['item_ids'] = [700]
            if issue == 'market': arsenal['marketplace']['cells_base64'][1] = base64.b64encode(b'other_mp').decode()
            if issue == 'map': arsenal['ncs_crc'] += 1
            if issue == 'faction': state['match_config']['roster'][1]['outfits'][1]['character']['faction'] = 1
            if issue == 'model': appearance['items'][0]['variants'][0]['mask'] = 0x94000000
            before = copy.deepcopy(state)
            with self.subTest(issue=issue), self.assertRaises(ValueError):
                bind(state, appearance, arsenal)
            self.assertEqual(state, before)


if __name__ == '__main__':
    unittest.main()
