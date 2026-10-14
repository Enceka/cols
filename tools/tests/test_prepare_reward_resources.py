import base64
import copy
import json
from pathlib import Path
import struct
import sys
import unittest
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from prepare_reward_resources import prepare
from prepare_game_state import CLIENT_SHA256


def sample():
    names=['killstreak_uav_mp','killstreak_counter_uav_mp','killstreak_emp_mp','radardirection_mp']
    raw=bytearray(0xe28)
    for off,value in [(0x47c,1),(0x480,6),(0x74c,899),(0x750,1200),(0x768,899),(0x764,1200)]:struct.pack_into('<I',raw,off,value)
    identity=dict(schema_version=1,read_only=True,client_sha256=CLIENT_SHA256,connection_state=10)
    a=dict(identity,map='maps/mp/mp_afghan.d3dbsp',ncs_crc=123,weapons=[dict(name=n,index=i+1,definition_base64=base64.b64encode(raw).decode(),runtime_fields=dict(fire_interval_msec=800)) for i,n in enumerate(names)],models=[dict(name='vehicle_test',index=1,bounds_center=[2,3,4],bounds_half=[5,6,7])],effects=[dict(name='explosions/emp_flash_mp',selector=1)])
    n=dict(identity,initialized=1,linked=[dict(category=k,strings_base64=[base64.b64encode(x.encode()).decode() for x in v]) for k,v in [(13,names),(0,['vehicle_test']),(3,['explosions/emp_flash_mp'])]])
    c=dict(rewards=[dict(native_id=i,weapon_token_c13=name) for i,name in zip([1,3,14,19,25],[*names,'killstreak_locker_mp'])])
    return a,n,c


class RewardResources(unittest.TestCase):
    def run_prepare(self,a,n,c,entities=None):
        with patch('prepare_reward_resources.compute',return_value=dict(ncs_crc=123)):
            return prepare(json.dumps(a).encode(),json.dumps(n).encode(),c,entities_raw=json.dumps(entities).encode() if entities else None)
    def test_air_route_uses_matching_native_map_anchor_and_rejects_missing_or_duplicate_anchors(self):
        a,n,c=sample();raw=b'{"classname" "script_origin" "targetname" "airstrikeheight" "origin" "1875.7 1261.8 1456.6"}'
        capture=dict(schema_version=1,read_only=True,client_sha256=CLIENT_SHA256,maps=[dict(name='maps/mp/mp_afghan.d3dbsp',byte_count=len(raw),entity_string_base64=base64.b64encode(raw).decode())])
        r,_=self.run_prepare(a,n,c,capture);self.assertEqual(r['air_route']['center'],[1875.7,1261.8,1456.6])
        self.assertEqual(len(r['air_route']['sources_sha256'][0]),64)
        bad=copy.deepcopy(capture);bad['maps'][0]['name']='maps/mp/mp_rust.d3dbsp'
        with self.assertRaises(ValueError):self.run_prepare(a,n,c,bad)
        for changed in [raw.replace(b'airstrikeheight',b'other_marker'),raw+raw]:
            bad=copy.deepcopy(capture);bad['maps'][0].update(byte_count=len(changed),entity_string_base64=base64.b64encode(changed).decode())
            with self.assertRaises(ValueError):self.run_prepare(a,n,c,bad)
    def test_all_positive_bindings_use_actual_registration_and_report_absent_special_assets(self):
        a,n,c=sample();r,missing=self.run_prepare(a,n,c)
        self.assertEqual([(x['native_id'],x['weapon']) for x in r['radios']],[(1,1),(3,2),(14,3),(19,4)])
        self.assertEqual(missing,[25]);self.assertEqual(r['models']['vehicle_test'],1)
        self.assertEqual(r['weapons']['killstreak_emp_mp']['switch']['raise_msec'],1200)
        self.assertEqual(r['model_bounds']['vehicle_test'],dict(min=[-3,-3,-3],max=[7,9,11]))
    def test_damage_stages_and_ranges_keep_registered_weapon_values(self):
        a,n,c=sample();raw=bytearray(base64.b64decode(a['weapons'][0]['definition_base64']))
        struct.pack_into('<6I',raw,0xbb4,20,0,0,0,0,10);struct.pack_into('<6f',raw,0xbcc,1200,0,0,0,0,1800)
        a['weapons'][0]['definition_base64']=base64.b64encode(raw).decode()
        r,_=self.run_prepare(a,n,c);w=r['weapons']['killstreak_uav_mp']
        self.assertEqual(w['damage_stages'],[20,0,0,0,0,10]);self.assertEqual(w['damage_ranges'],[1200,0,0,0,0,1800])
    def test_dynamic_character_models_and_composed_guns_do_not_forge_ncs_entries(self):
        a,n,c=sample();a['models'].append(dict(name='dynamic_outfit',index=200));a['weapons'].append(dict(name='** dynamic rifle',index=200))
        r,_=self.run_prepare(a,n,c);self.assertNotIn('dynamic_outfit',r['models'])
    def test_wrong_identity_uninitialized_and_mismatched_ids_are_rejected(self):
        a,n,c=sample()
        for key,value in [('read_only',False),('client_sha256','f'*64),('ncs_crc',124),('connection_state',0)]:
            bad=copy.deepcopy(a);bad[key]=value
            with self.assertRaises(ValueError):self.run_prepare(bad,n,c)
        for group,key in [('weapons','index'),('models','index'),('effects','selector')]:
            bad=copy.deepcopy(a);bad[group][0][key]=700
            with self.assertRaises(ValueError):self.run_prepare(bad,n,c)
    def test_none_ammunition_sentinel_stays_absent_and_truncated_definitions_fail(self):
        a,n,c=sample();raw=bytearray(base64.b64decode(a['weapons'][0]['definition_base64']));struct.pack_into('<I',raw,0x6c0,0xffffffff);a['weapons'][0]['definition_base64']=base64.b64encode(raw).decode()
        r,_=self.run_prepare(a,n,c);self.assertIsNone(r['weapons']['killstreak_uav_mp']['clip_key'])
        a['weapons'][0]['definition_base64']=base64.b64encode(bytes(30)).decode()
        with self.assertRaises(ValueError):self.run_prepare(a,n,c)


if __name__=='__main__':unittest.main()
