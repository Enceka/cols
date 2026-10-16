import base64
import copy
import json
from pathlib import Path
import struct
import sys
import unittest
import zlib
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from prepare_reward_resources import prepare, remote_vision, arena_thermal
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
    def test_animal_capture_keeps_parent_chains_and_rejects_a_cycle(self):
        a,n,c=sample();raw=bytearray(8+8*10);struct.pack_into('<I',raw,0,10)
        names=['german_shepherd_idle','german_shepherd_run','german_shepherd_attack_player','german_shepherd_death_front']
        parts=[dict(name=name,node=i+1,parent=5 if i==2 else 0,frames=30,frame_rate=30) for i,name in enumerate(names)]
        parents=[0]*10;parents[3]=5;parents[5]=6
        a.update(actor_animlist_mask=2,actor_trees=[dict(selector=1,registered_tree_ids=[10],tree_layout=dict(hex=raw.hex()),parts=parts,parents=parents)],
            vehicle_animtrees=[dict(index=10,layout=dict(hex=raw.hex()),parts=parts)],actor_rawfiles={})
        for name in ['animlists/common_dog.animlist','animtrees/dog.atr']:
            data=b'captured content';a['actor_rawfiles'][name]=dict(name=name,found=True,length=len(data),compressed_length=0,buffer_base64=base64.b64encode(data).decode())
        result,_=self.run_prepare(a,n,c)
        self.assertEqual(result['animals']['ancestors']['bite'],[6,5])
        self.assertEqual(result['animals']['durations']['death'],1000)
        a['actor_trees'][0]['parents'][6]=5
        with self.assertRaises(ValueError):self.run_prepare(a,n,c)

    def test_armored_operator_exports_rotary_trigger_reload_and_move_facts(self):
        a,n,c=sample();name='minigungunship_mp';raw=bytearray(0xe28)
        for off,value in [(0x48c,6),(0x71c,9699),(0x728,9699),(0x730,7130),(0x738,7130),(0xd08,300)]:struct.pack_into('<I',raw,off,value)
        struct.pack_into('<ff',raw,0x7c8,1.0,0.75)
        a['weapons'].append(dict(name=name,index=5,definition_base64=base64.b64encode(raw).decode(),runtime_fields=dict(fire_interval_msec=70,fire_delay_msec=0)))
        next(x for x in n['linked'] if x['category']==13)['strings_base64'].append(base64.b64encode(name.encode()).decode())
        r,_=self.run_prepare(a,n,c);w=r['weapons'][name]['operator']
        self.assertEqual((w['fire_type'],w['move_scale'],w['ads_move_scale']),(6,1.0,0.75))
        self.assertEqual(w['reload']['empty_add_msec'],7130)
        a['weapons'][-1]['runtime_fields']['fire_delay_msec']=20
        with self.assertRaises(ValueError):self.run_prepare(a,n,c)

    def test_camera_mount_and_gun_magazine_come_from_capture(self):
        a,n,c=sample();a['models'][0]['attachment_bind_matrices']={'tag_player':[0,0,0,1,10,20,-30,2]}
        raw=bytearray(base64.b64decode(a['weapons'][0]['definition_base64']));struct.pack_into('<I',raw,0xd08,20);struct.pack_into('<I',raw,0x71c,1500)
        a['weapons'][0]['definition_base64']=base64.b64encode(raw).decode()
        r,_=self.run_prepare(a,n,c)
        self.assertEqual(r['camera_mounts']['vehicle_test'],{'rotation':[0,0,0,1],'translation':[10,20,-30]})
        self.assertEqual((r['weapons']['killstreak_uav_mp']['clip_size'],r['weapons']['killstreak_uav_mp']['reload_msec']),(20,1500))
        a['models'][0]['attachment_bind_matrices']['tag_player'][0]=float('nan')
        with self.assertRaises(ValueError):self.run_prepare(a,n,c)
    def run_prepare(self,a,n,c,entities=None):
        with patch('prepare_reward_resources.compute',return_value=dict(ncs_crc=123)):
            return prepare(json.dumps(a).encode(),json.dumps(n).encode(),c,entities_raw=json.dumps(entities).encode() if entities else None)
    def test_remote_vision_requires_map_arena_and_both_real_rawfiles(self):
        def record(name, content, compressed=False):
            payload = zlib.compress(content) if compressed else content
            return dict(found=True, name=name, length=len(content),
                        compressed_length=len(payload) if compressed else 0,
                        buffer_base64=base64.b64encode(payload).decode())
        arena = b'// thermal selection\n{ map mp_afghan thermal invert } /* next */ { map mp_subbase_sh }'
        raw = {'mp/basemaps.arena': record('mp/basemaps.arena', arena, True)}
        for name in ['black_bw', 'thermal_mp', 'thermal_snowlevel_mp']:
            path = 'vision/' + name + '.vision'
            raw[path] = record(path, b'"r_filmEnable" "1"\n', name == 'thermal_mp')
        self.assertEqual(remote_vision({}, 'mp_afghan'), None)
        self.assertEqual(remote_vision(raw, 'mp_afghan')['flight'], 'thermal_snowlevel_mp')
        # The map name does not imply inversion; only the captured arena does.
        self.assertEqual(remote_vision(raw, 'mp_subbase_sh')['flight'], 'thermal_mp')
        self.assertEqual(len(remote_vision(raw, 'mp_afghan')['rawfiles_sha256']), 3)
        absent = copy.deepcopy(raw)
        absent['mp/basemaps.arena'] = dict(found=False)
        result = remote_vision(absent, 'mp_afghan')
        self.assertEqual(result['flight'], 'thermal_mp')
        self.assertIs(result['arena_absent'], True)
        self.assertEqual(len(result['rawfiles_sha256']), 2)
        del absent['mp/basemaps.arena']
        self.assertIsNone(remote_vision(absent, 'mp_afghan'))

        missing = copy.deepcopy(raw)
        missing['vision/black_bw.vision']['found'] = False
        self.assertIsNone(remote_vision(missing, 'mp_afghan'))
        for broken in [arena + b'{map mp_afghan}', b'{map mp_afghan thermal invert thermal other}', b'{map mp_afghan', b'"unterminated', b'{map mp_afghan /*unterminated value}']:
            with self.assertRaises(ValueError): arena_thermal(broken, 'mp_afghan')
        with self.assertRaises(ValueError): arena_thermal(arena, 'mp_missing')
        self.assertFalse(arena_thermal(b'{map mp_afghan image mp/path.tga description "literal // text"}', 'mp_afghan'))
        with self.assertRaises(ValueError): arena_thermal(b'{map mp_afghan}\0{map other}', 'mp_afghan')

        bad = copy.deepcopy(raw)
        bad['mp/basemaps.arena']['buffer_base64'] = base64.b64encode(zlib.compress(arena) + b'extra').decode()
        bad['mp/basemaps.arena']['compressed_length'] += 5
        with self.assertRaises(ValueError): remote_vision(bad, 'mp_afghan')
        bad = copy.deepcopy(raw)
        bad['vision/thermal_snowlevel_mp.vision']['length'] += 1
        with self.assertRaises(ValueError): remote_vision(bad, 'mp_afghan')

    def test_static_exit_requires_both_loaded_materials_and_free_dynamic_slots(self):
        a,n,c = sample()
        r,_ = self.run_prepare(a,n,c)
        self.assertNotIn('remote_exit', r)
        a['remote_material_assets'] = [dict(name=name, found=True, loaded=True) for name in ['white','ac130_overlay_grain']]
        r,_ = self.run_prepare(a,n,c)
        self.assertEqual(r['remote_exit'], dict(white=1000,grain=1001,material_bank_count=0))
        a['remote_material_assets'][1]['loaded'] = False
        r,_ = self.run_prepare(a,n,c)
        self.assertNotIn('remote_exit', r)
        a['remote_material_assets'][1]['loaded'] = True
        n['linked'].append(dict(category=8,strings_base64=[base64.b64encode(('material_'+str(i)).encode()).decode() for i in range(1000)]))
        with self.assertRaises(ValueError): self.run_prepare(a,n,c)

    def test_selector_bank_preserves_actual_native_slots_and_rejects_aliases(self):
        a,n,c=sample()
        n['linked'].append(dict(category=12,strings_base64=[base64.b64encode(x.encode()).decode() for x in ['map_artillery_selector','map_mortar_selector']]))
        r,_=self.run_prepare(a,n,c)
        self.assertEqual(r['selectors'],dict(map_artillery_selector=1,map_mortar_selector=2))
        n['linked'][-1]['strings_base64']*=2
        with self.assertRaises(ValueError):self.run_prepare(a,n,c)
    def test_cobra_loop_requires_captured_alias_and_uses_eight_bit_dynamic_slot(self):
        a,n,c=sample()
        r,_=self.run_prepare(a,n,c)
        self.assertEqual(r['loop_sounds'],{})
        a['sounds']=[dict(name='weap_cobra_20mm_fire_npc',found=False)]
        r,_=self.run_prepare(a,n,c)
        self.assertEqual(r['loop_sounds'],{})
        a['sounds'][0]['found']=True
        r,_=self.run_prepare(a,n,c)
        self.assertEqual(r['loop_sounds'],{'weap_cobra_20mm_fire_npc':240})

    def test_remote_missile_spawns_keep_real_target_links_and_provenance(self):
        a,n,c=sample()
        raw=b'{"classname" "script_origin" "targetname" "airstrikeheight" "origin" "0 0 2000"}'
        raw+=b'{"targetname" "remoteMissileSpawn" "origin" "-1000 0 2000" "target" "remote_target"}'
        raw+=b'{"targetname" "remote_target" "origin" "500 10 0"}'
        def capture(data):
            return dict(schema_version=1,read_only=True,client_sha256=CLIENT_SHA256,maps=[dict(name='maps/mp/mp_afghan.d3dbsp',byte_count=len(data),entity_string_base64=base64.b64encode(data).decode())])
        r,_=self.run_prepare(a,n,c,capture(raw))
        self.assertEqual(r['remote_spawns']['points'],[dict(origin=[-1000,0,2000],target=[500,10,0])])
        self.assertEqual(len(r['remote_spawns']['sources_sha256'][0]),64)
        for bad in [raw.replace(b'"target" "remote_target"',b'"target" "missing"'),raw+b'{"targetname" "remote_target" "origin" "1 2 3"}',raw.replace(b'500 10 0',b'-1000 0 2000')]:
            with self.assertRaises(ValueError):self.run_prepare(a,n,c,capture(bad))

    def test_projectile_lifetime_and_explosion_kind_preserve_target_data(self):
        a,n,c=sample();raw=bytearray(base64.b64decode(a['weapons'][0]['definition_base64']))
        struct.pack_into('<I',raw,0x47c,2);struct.pack_into('<f',raw,0x904,12.0);struct.pack_into('<I',raw,0x914,1)
        a['weapons'][0]['definition_base64']=base64.b64encode(raw).decode()
        r,_=self.run_prepare(a,n,c);self.assertEqual(r['weapons']['killstreak_uav_mp']['projectile'],dict(lifetime_msec=12000,explosion_kind=1))
        struct.pack_into('<f',raw,0x904,200.0);a['weapons'][0]['definition_base64']=base64.b64encode(raw).decode()
        r,_=self.run_prepare(a,n,c);self.assertEqual(r['weapons']['killstreak_uav_mp']['projectile']['lifetime_msec'],200000)
        for lifetime in [float('nan'),-1.0]:
            struct.pack_into('<f',raw,0x904,lifetime);a['weapons'][0]['definition_base64']=base64.b64encode(raw).decode()
            with self.assertRaises(ValueError):self.run_prepare(a,n,c)

    def test_camera_tags_use_immutable_ncs_slots_and_reject_duplicates(self):
        a,n,c=sample()
        n['linked'].append(dict(category=9,strings_base64=[base64.b64encode(x.encode()).decode() for x in ['j_clavicle_le','tag_origin']]))
        r,_=self.run_prepare(a,n,c);self.assertEqual(r['tags'],{'j_clavicle_le':1,'tag_origin':2})
        n['linked'][-1]['strings_base64'].append(base64.b64encode(b'tag_origin').decode())
        with self.assertRaises(ValueError):self.run_prepare(a,n,c)

    def test_vehicle_default_slot_and_rotor_tree_use_separate_registries(self):
        a,n,c=sample()
        n['linked'].append(dict(category=2,strings_base64=[base64.b64encode(x.encode()).decode() for x in ['defaultvehicle','cobra_mp','pavelow_mp']]))
        a['vehicles']=[dict(name='defaultvehicle',index=1),dict(name='cobra_mp',index=2),dict(name='pavelow_mp',index=3)]
        raw=bytearray(104);struct.pack_into('<I',raw,0,12);struct.pack_into('<H',raw,94,10)
        tree=dict(index=3,layout=dict(hex=raw.hex(),pointers=[dict(offset=96,target=dict(pointers=[dict(offset=0,string='bh_rotors')]))]))
        a['vehicle_animtrees']=[tree]
        r,_=self.run_prepare(a,n,c)
        self.assertEqual(r['vehicles'],dict(cobra_mp=2,pavelow_mp=3))
        self.assertEqual(r['animations']['bh_rotors'],dict(tree=3,node=11,parent=10))
        bad=copy.deepcopy(a);bad['vehicles'][1]['index']=3
        with self.assertRaises(ValueError):self.run_prepare(bad,n,c)
        bad=copy.deepcopy(a);bad['vehicle_animtrees'].append(tree)
        with self.assertRaises(ValueError):self.run_prepare(bad,n,c)
    def test_dynamic_copy_of_registered_model_does_not_override_its_ncs_slot(self):
        a,n,c=sample();clone=copy.deepcopy(a['models'][0]);clone['index']=500;a['models'].insert(0,clone)
        r,_=self.run_prepare(a,n,c);self.assertEqual(r['models']['vehicle_test'],1)
        a['models'].pop()
        with self.assertRaises(ValueError):self.run_prepare(a,n,c)
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
        struct.pack_into('<III',raw,0x8f0,17,3000,23)
        a['weapons'][0]['definition_base64']=base64.b64encode(raw).decode()
        r,_=self.run_prepare(a,n,c);w=r['weapons']['killstreak_uav_mp']
        self.assertEqual(w['damage_stages'],[20,0,0,0,0,10]);self.assertEqual(w['damage_ranges'],[1200,0,0,0,0,1800])
        self.assertEqual(w['projectile_speed'],3000)
    def test_airdrop_markers_export_authored_pull_throw_and_bounce_parameters(self):
        a,n,c=sample();name='airdrop_marker_mp';raw=bytearray(0xe28)
        for off,value in [(0x47c,1),(0x480,6),(0x6c0,5),(0x6b8,5),(0x70c,600),(0xd10,360),(0x6f0,129),(0x7ac,1000),(0x8f4,960),(0x8f8,120)]:struct.pack_into('<I',raw,off,value)
        struct.pack_into('<31f',raw,0x934,*([.5]*31));struct.pack_into('<31f',raw,0x9b0,*([.25]*31))
        a['weapons'].append(dict(name=name,index=5,definition_base64=base64.b64encode(raw).decode(),runtime_fields=dict(fire_interval_msec=360)))
        next(row for row in n['linked'] if row['category']==13)['strings_base64'].append(base64.b64encode(name.encode()).decode())
        c['rewards'].append(dict(native_id=2,weapon_token_c13=name))
        r,_=self.run_prepare(a,n,c);m=r['weapons'][name]['marker']
        self.assertEqual((m['prepare_msec'],m['throw_msec'],m['throw_delay_msec'],m['fuse_msec']),(600,360,129,1000))
        self.assertEqual(m['parallel'],[.5]*31);self.assertEqual(m['perpendicular'],[.25]*31);self.assertEqual(m['speed_up'],120)
        struct.pack_into('<I',raw,0x490,1);a['weapons'][-1]['definition_base64']=base64.b64encode(raw).decode()
        with self.assertRaises(ValueError):self.run_prepare(a,n,c)
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
