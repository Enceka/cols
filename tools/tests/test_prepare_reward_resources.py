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
    a=dict(identity,map='maps/mp/mp_afghan.d3dbsp',ncs_crc=123,weapons=[dict(name=n,index=i+1,definition_base64=base64.b64encode(raw).decode(),runtime_fields=dict(fire_interval_msec=800)) for i,n in enumerate(names)],models=[dict(name='vehicle_test',index=1)],effects=[dict(name='explosions/emp_flash_mp',selector=1)])
    n=dict(identity,initialized=1,linked=[dict(category=k,strings_base64=[base64.b64encode(x.encode()).decode() for x in v]) for k,v in [(13,names),(0,['vehicle_test']),(3,['explosions/emp_flash_mp'])]])
    c=dict(rewards=[dict(native_id=i,weapon_token_c13=name) for i,name in zip([1,3,14,19,25],[*names,'killstreak_locker_mp'])])
    return a,n,c


class RewardResources(unittest.TestCase):
    def run_prepare(self,a,n,c):
        with patch('prepare_reward_resources.compute',return_value=dict(ncs_crc=123)):
            return prepare(json.dumps(a).encode(),json.dumps(n).encode(),c)
    def test_all_positive_bindings_use_actual_registration_and_report_absent_special_assets(self):
        a,n,c=sample();r,missing=self.run_prepare(a,n,c)
        self.assertEqual([(x['native_id'],x['weapon']) for x in r['radios']],[(1,1),(3,2),(14,3),(19,4)])
        self.assertEqual(missing,[25]);self.assertEqual(r['models']['vehicle_test'],1)
        self.assertEqual(r['weapons']['killstreak_emp_mp']['switch']['raise_msec'],1200)
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
