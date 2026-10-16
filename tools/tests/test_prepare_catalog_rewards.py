import base64
import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from prepare_catalog_rewards import bind, prepare, CLIENT_SHA256


def sample():
    names = ['killstreak_helicopter_mp', 'killstreak_nuke_mp', 'nuke_mp', 'killstreak_uav_mp',
             'killstreak_counter_uav_mp', 'killstreak_predator_missile_mp', 'killstreak_sentry_mp']
    banks = {13: names, 0: ['padding', 'vehicle_mi24p_hind_mp', 'projectile_hellfire_missile'],
             3: ['dust/nuke_aftermath_mp', 'explosions/player_death_nuke_flash', 'explosions/player_death_nuke'],
             4: ['MP_TACTICAL_NUKE_CALLED', 'MP_EARNED_NUKE'],
             5: ['nuke_music', 'ui_mp_nukebomb_timer'], 8: ['menu'], 9: ['tag_origin'],
             2: ['defaultvehicle', 'cobra_mp'], 12: ['map_artillery_selector']}
    ncs = dict(schema_version=1, read_only=True, client_sha256=CLIENT_SHA256, initialized=1,
               linked=[dict(category=category, strings_base64=[base64.b64encode(n.encode()).decode() for n in values]) for category, values in banks.items()])
    identity = dict(schema_version=1, client_sha256=CLIENT_SHA256)
    shared = dict(identity, map='mp_afghan', ncs_crc=99,
        radios=[dict(native_id=i, weapon=77+i, switch={}, fire_interval_msec=800) for i in [1,3,5,8,16]],
        models={'vehicle_mi24p_hind_mp': 100, 'projectile_hellfire_missile': 101},
        model_bounds={'vehicle_mi24p_hind_mp': {'min':[-1,-1,-1], 'max':[1,1,1]}}, camera_mounts={},
        effects={}, tags={'tag_origin':18}, vehicles={'cobra_mp':3}, selectors={},
        weapons={'killstreak_uav_mp':dict(weapon=78, clip_key=78, ammo_key=78)},
        air_route={'center':[9999,9999,9999]}, remote_spawns={'points':[{'origin':[9999,9999,9999]}]},
        remote_exit=dict(white=1000, grain=1001, material_bank_count=27))
    nuclear = dict(identity, map='mp_afghan', ncs_crc=99, weapon=323)
    state = dict(identity, map='mp_nuked', mode='war', ncs_crc=123,
                 match_config={'spawns': [[[0,0,10]], [[100,100,20]]]})
    return state, ncs, shared, nuclear


class CatalogRewards(unittest.TestCase):
    def bind(self, state, ncs, shared, nuclear, **options):
        with patch('prepare_catalog_rewards.compute', return_value={'ncs_crc':123}):
            return bind(state, *[json.dumps(value).encode() for value in [ncs, shared, nuclear]], **options)

    def test_rebinding_uses_map_ncs_slots_and_generated_geometry_is_explicit(self):
        state, ncs, shared, nuclear = sample()
        radar, nuke, report = self.bind(state, ncs, shared, nuclear)
        self.assertEqual(radar['models']['vehicle_mi24p_hind_mp'], 2)
        self.assertEqual(next(r for r in radar['radios'] if r['native_id']==1)['weapon'], 4)
        self.assertEqual(radar['weapons']['killstreak_uav_mp']['ammo_key'], 4)
        self.assertEqual(radar['tags']['tag_origin'], 1)
        self.assertEqual(radar['vehicles']['cobra_mp'], 2)
        self.assertEqual((nuke['weapon'], nuke['damage_weapon'], nuke['flash_fx']), (2,3,2))
        self.assertEqual((nuke['tick_sound'], nuke['earned_text']), (513,2))
        self.assertEqual(radar['map'], 'mp_nuked')
        self.assertEqual(radar['ncs_crc'], 123)
        self.assertFalse(report['authored_flight_graph'])
        self.assertTrue(report['experimental'])
        self.assertFalse(report['native_acceptance_claimed'])
        self.assertNotEqual(radar['air_route']['center'], shared['air_route']['center'])
        self.assertEqual(radar['remote_spawns']['points'], [])
        self.assertFalse(radar['experimental_air_routes'])
        self.assertTrue(self.bind(state, ncs, shared, nuclear, unchecked=True)[0]['experimental_air_routes'])

    def test_missing_resources_wrong_crc_and_diagnostic_sources_are_rejected(self):
        state, ncs, shared, nuclear = sample()
        wrong = copy.deepcopy(state)
        wrong['ncs_crc'] = 124
        with self.assertRaises(ValueError): self.bind(wrong, ncs, shared, nuclear)
        nuke = dict(nuclear, acceptance_seed=6)
        with self.assertRaises(ValueError): self.bind(state, ncs, shared, nuke)
        broken = copy.deepcopy(ncs)
        weapons = next(row for row in broken['linked'] if row['category']==13)['strings_base64']
        weapons.append(base64.b64encode(b'killstreak_predator_missile_mp').decode())
        with self.assertRaises(ValueError): self.bind(state, broken, shared, nuclear)

    def test_catalog_output_is_new_atomic_and_keeps_inputs_intact(self):
        state, ncs, shared, nuclear = sample()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            inputs = dict(state=state, ncs=ncs, shared=shared, nuclear=nuclear,
                          catalog={'schema_version':1,'entries':[{'map':'mp_nuked','mode':'war','file':'state.json'}]},
                          index=[{'map':'mp_nuked','mode':'war','ncs':'ncs.json'}])
            for name, value in inputs.items(): (root/(name+'.json')).write_text(json.dumps(value))
            before = (root/'state.json').read_bytes()
            with patch('prepare_catalog_rewards.compute', return_value={'ncs_crc':123}):
                result = prepare(root/'catalog.json',root/'index.json',root/'shared.json',root/'nuclear.json',root/'result')
            self.assertTrue(result.is_file())
            prepared = json.loads((root/'result/maps/mp_nuked-war.json').read_text())
            self.assertTrue((root/'result/maps'/prepared['radar_file']).is_file())
            self.assertEqual((root/'state.json').read_bytes(), before)
            with self.assertRaises(ValueError): prepare(root/'catalog.json',root/'index.json',root/'shared.json',root/'nuclear.json',root/'result')
            bad = copy.deepcopy(inputs['catalog'])
            bad['entries'].append({'map':'mp_missing','mode':'war','file':'missing.json'})
            (root/'bad.json').write_text(json.dumps(bad))
            with patch('prepare_catalog_rewards.compute', return_value={'ncs_crc':123}):
                with self.assertRaises(ValueError): prepare(root/'bad.json',root/'index.json',root/'shared.json',root/'nuclear.json',root/'failed')
            self.assertFalse((root/'failed').exists())
            self.assertEqual(list(root.glob('.reward-preparation-*')), [])
