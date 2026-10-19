import base64
import copy
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from prepare_props import prepare, prepare_cached, wreck_for
from prepare_game_state import CLIENT_SHA256


def captures():
    identity=dict(schema_version=1,read_only=True,client_sha256=CLIENT_SHA256)
    names=['com_barrel_russian_fuel_dirt','vehicle_hummer_destructible','vehicle_hummer_destroyed']
    fx=['props/barrel_ignite','props/barrel_fire_top','props/barrelexp','explosions/small_vehicle_explosion','smoke/car_damage_blacksmoke']
    a=dict(identity,map='maps/mp/mp_afghan.d3dbsp',connection_state=10,ncs_crc=123,
           models=[dict(name=n,index=i+1,bounds_min=[-10,-10,0],bounds_max=[10,10,40]) for i,n in enumerate(names)],
           effects=[dict(name=n,selector=i+1) for i,n in enumerate(fx)],weapons=[dict(name='destructible_car',index=1)],
           sounds=[dict(name=n,found=True,loaded=False) for n in ['explo_metal_rand','car_explode']])
    text=b'{"classname" "script_model" "model" "com_barrel_russian_fuel_dirt" "targetname" "explodable_barrel" "origin" "123.1 10 0" "angles" "0 270 90"}'
    text+=b'{"classname" "script_model" "model" "vehicle_hummer_destructible" "targetname" "destructible_vehicle" "destructible_type" "vehicle_hummer" "origin" "300 0 0"}'
    text+=text # Native map variants can repeat an identical model/pose.
    e=dict(identity,maps=[dict(name='maps/mp/mp_afghan.d3dbsp',byte_count=len(text),entity_string_base64=base64.b64encode(text).decode())])
    n=dict(identity,initialized=1,linked=[dict(category=cat,strings_base64=[base64.b64encode(s.encode()).decode() for s in values]) for cat,values in [(0,names),(3,fx),(13,['destructible_car'])]])
    return a,e,n


class PropPreparation(unittest.TestCase):
    def test_wreck_model_variants_follow_native_registration_names(self):
        models = {
            'veh_80s_sedan1_nt': {'index': 189},
            'vehicle_80s_sedan1_nt_destroyed': {'index': 192},
            'vehicle_uaz_hardtop_destructible_mp': {'index': 198},
            'vehicle_uaz_hardtop_dsr': {'index': 199},
            'vehicle_hummer_destructible': {'index': 192},
            'vehicle_hummer_destroyed': {'index': 191},
            'vehicle_uaz_winter_destructible': {'index': 187},
            'vehicle_uaz_winter_destroy': {'index': 186},
            'vehicle_moving_truck_destructible': {'index': 253},
            'vehicle_moving_truck_dst': {'index': 254},
        }
        self.assertEqual(
            wreck_for('veh_80s_sedan1_nt', 'vehicle_80s_sedan1_nt', models)['index'],
            192,
        )
        self.assertEqual(
            wreck_for('vehicle_uaz_hardtop_destructible_mp', 'vehicle_uaz_hardtop', models)['index'],
            199,
        )
        self.assertEqual(
            wreck_for('vehicle_hummer_destructible', 'vehicle_hummer', models)['index'],
            191,
        )
        self.assertEqual(
            wreck_for('vehicle_uaz_winter_destructible', 'vehicle_uaz_winter', models)['index'],
            186,
        )
        self.assertEqual(
            wreck_for('vehicle_moving_truck_destructible', 'vehicle_moving_truck', models)['index'],
            254,
        )

    def test_real_map_variants_bind_sedan_and_uaz_wrecks(self):
        identity = dict(schema_version=1, read_only=True, client_sha256=CLIENT_SHA256)
        names = [
            'veh_80s_sedan1_nt',
            'vehicle_80s_sedan1_nt_destroyed',
            'vehicle_uaz_hardtop_destructible_mp',
            'vehicle_uaz_hardtop_dsr',
        ]
        effects = [
            'explosions/small_vehicle_explosion',
            'smoke/car_damage_blacksmoke',
        ]
        assets = dict(
            identity,
            map='maps/mp/mp_nuked.d3dbsp',
            connection_state=10,
            ncs_crc=123,
            models=[dict(name=n, index=i + 1, bounds_min=[-10, -10, 0], bounds_max=[10, 10, 40]) for i, n in enumerate(names)],
            effects=[dict(name=n, selector=i + 1) for i, n in enumerate(effects)],
            weapons=[dict(name='destructible_car', index=1)],
            sounds=[dict(name=n, found=True, loaded=False) for n in ['explo_metal_rand', 'car_explode']],
        )
        text = b''.join([
            b'{"classname" "script_model" "model" "veh_80s_sedan1_nt" "targetname" "destructible_vehicle" "destructible_type" "vehicle_80s_sedan1_nt" "origin" "0 0 0"}',
            b'{"classname" "script_model" "model" "vehicle_uaz_hardtop_destructible_mp" "targetname" "destructible_vehicle" "destructible_type" "vehicle_uaz_hardtop" "origin" "100 0 0"}',
        ])
        entities = dict(identity, maps=[dict(name='maps/mp/mp_nuked.d3dbsp', byte_count=len(text), entity_string_base64=base64.b64encode(text).decode())])
        linked = [(0, names), (3, effects), (13, ['destructible_car'])]
        constants = dict(identity, initialized=1, linked=[dict(category=cat, strings_base64=[base64.b64encode(s.encode()).decode() for s in values]) for cat, values in linked])
        with patch('prepare_props.compute', return_value=dict(ncs_crc=123)):
            result = prepare(json.dumps(assets).encode(), json.dumps(entities).encode(), json.dumps(constants).encode())
        self.assertEqual([p['destroyed_model'] for p in result['props']], [2, 4])

    def test_cached_models_use_original_capture_digests_and_reject_a_different_common_bank(self):
        a,e,n=captures();n['common_checksum']=77
        characters=dict(schema_version=1,read_only=True,client_sha256=CLIENT_SHA256,ncs_crc=123,
                        map=a['map'],models=[dict(name=m['name'],model=m['index'],bounds_min=m['bounds_min'],bounds_max=m['bounds_max']) for m in a['models']])
        raws=[json.dumps(x).encode() for x in [characters,e,n,a,n]]
        with patch('prepare_props.compute',return_value=dict(ncs_crc=123)):
            result=prepare_cached(*raws)
            self.assertEqual(len(result['props']),2)
            import hashlib
            self.assertEqual(result['sources_sha256'],[hashlib.sha256(raw).hexdigest() for raw in raws])
            bad=copy.deepcopy(n);bad['common_checksum']=78
            with self.assertRaises(ValueError): prepare_cached(raws[0],raws[1],json.dumps(bad).encode(),raws[3],raws[4])
    def prepare(self,a,e,n):
        with patch('prepare_props.compute',return_value=dict(ncs_crc=123)):
            return prepare(*[json.dumps(c).encode() for c in [a,e,n]])
    def test_real_instances_use_matching_registrations_and_deduplicate_variant_duplicates(self):
        a,e,n=captures();p=self.prepare(a,e,n)
        self.assertEqual(len(p['props']),2)
        self.assertEqual(p['props'][0]['origin'],[123,10,0])
        self.assertEqual(p['props'][0]['angles'],[0,49152,16384])
        self.assertEqual(p['props'][1]['destroyed_model'],3)
        self.assertEqual(p['sound_aliases'],[[480,'explo_metal_rand'],[481,'car_explode']])
    def test_wrong_build_crc_registration_and_resource_truncation_are_rejected(self):
        a,e,n=captures()
        for key,value in [('read_only',False),('client_sha256','other'),('connection_state',0),('ncs_crc',124)]:
            bad=copy.deepcopy(a);bad[key]=value
            with self.assertRaises(ValueError):self.prepare(bad,e,n)
        bad=copy.deepcopy(a);bad['models'][0]['index']=99
        with self.assertRaises(ValueError):self.prepare(bad,e,n)
        bad=copy.deepcopy(e);bad['maps'][0]['byte_count']+=1
        with self.assertRaises(ValueError):self.prepare(a,bad,n)
        bad=copy.deepcopy(a);bad['sounds'][0]['found']=False
        with self.assertRaises(ValueError):self.prepare(bad,e,n)
        bad=copy.deepcopy(a);bad['models'][0]['bounds_max'][0]=-10
        with self.assertRaises(ValueError):self.prepare(bad,e,n)


if __name__=='__main__':unittest.main()
