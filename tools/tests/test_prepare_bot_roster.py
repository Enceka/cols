import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from prepare_bot_roster import animation_config, rows, variant, model_registry, NativeBotCharacters
from test_prepare_bot_locomotion import inputs


class BotRosterPreparation(unittest.TestCase):
    def test_native_bot_composition_uses_one_skin_and_preserves_item_identity(self):
        import base64
        import struct
        from test_prepare_player_appearance import inputs as appearance_inputs
        capture,_,_ = appearance_inputs()
        enc = lambda values: base64.b64encode(struct.pack('<'+'I'*len(values),*values)).decode()
        capture['packing_bits_base64'] = enc([7,4,8,8,6,7,8,5,0,8,0])
        offsets=[0x18,0x1c,0x20,0x14,0x24,0x28,0x2c,0x44,0x48,0x4c,0x54,0x40,0x50,0x30,0x34,0x38,0x3c,0x58,0x5c,0x60,0x64]
        capture['packing_fields_base64']=enc([v for o in offsets for v in (o,10 if o<0x40 else 6,0)])
        capture['gender_selectors_base64']=enc([2,3,4])
        capture['faction_selectors_base64']=enc([0,1])
        capture['skin_selectors_base64']=enc([5,6,7,8])
        for v in capture['variants']:
            mask=0x60800000 if v['group']=='hea' else 0x64000000 if v['group']=='han' else 0x67800000
            v['selectors_base64']=base64.b64encode(struct.pack('<HI',0,mask)).decode()
        capture['variants'].append(dict(group='han',index=1,model='han_test_cau',selectors_base64=base64.b64encode(struct.pack('<HI',0,0x60800000)).decode()))
        items={g:g+'_test' for g in ['fpa','boo','fja','han','hea']}
        models=[g+'_test_tf' for g in items]
        bank=NativeBotCharacters(capture)
        character,chosen=bank.compose(items,'f',2,models)
        self.assertEqual(character['groups'],[1]*5+[0]*6)
        self.assertEqual((character['gender'],character['faction'],character['skin']),(1,0,3))
        self.assertIn('han_test_cau',chosen)
        self.assertNotIn('han_test_tf',chosen)
        with self.assertRaises(ValueError): bank.compose(items,'f',1,models)
        with self.assertRaises(ValueError): bank.compose(dict(items,han='unknown'),'f',2,models)
        capture['read_only']=False
        with self.assertRaises(ValueError): NativeBotCharacters(capture)

    def test_repreparation_reuses_explicit_models_but_not_unlisted_runtime_player_slots(self):
        state={"match_config":{"models":[[193,"bot_body"]]}}
        capture={"models":[{"model":4,"name":"inherited_body"},{"model":193,"name":"bot_body"},{"model":243,"name":"old_player"}]}
        models, registrations=model_registry(state,capture)
        self.assertEqual(models,{"inherited_body":4,"bot_body":193})
        self.assertEqual(registrations,[[193,"bot_body"]])
        state["match_config"]["models"].append([194,"bot_body"])
        with self.assertRaises(ValueError):model_registry(state,capture)
    def test_outfit_variant_matches_gender_faction_and_shared_parts(self):
        import base64
        values = [["coat", "model", "m sc sss", "coat_sc"],
                  ["coat", "model", "m tf sss", "coat_tf"],
                  ["coat", "model", "f ss sss", "coat_f"]]
        table = dict(found=True, rows=3, columns=4,
                     cells_base64=[base64.b64encode(x.encode()).decode() for row in values for x in row])
        self.assertEqual(variant(table, "coat", "m", "tf"), "coat_tf")
        self.assertEqual(variant(table, "coat", "f", "sc"), "coat_f")
        with self.assertRaises(ValueError):
            variant(table, "missing", "m", "tf")
        table["rows"] += 1
        with self.assertRaises(ValueError):
            rows(table)

    def test_each_weapon_uses_its_own_native_animation_branch(self):
        import base64
        import struct
        state, capture, arsenal = inputs()
        definition = bytearray(base64.b64decode(arsenal["weapons"][0]["definition_base64"]))
        rifle = animation_config({}, capture, definition)
        self.assertEqual(rifle["reload_animation"], 13)
        struct.pack_into("<I", definition, 0x480, 2)
        machinegun = animation_config({}, capture, definition)
        self.assertNotEqual(rifle["locomotion"]["run_forward"], machinegun["locomotion"]["run_forward"])
        struct.pack_into("<I", definition, 0x478, 3)
        struct.pack_into("<I", definition, 0x480, 3)
        smg = animation_config({}, capture, definition)
        self.assertEqual(smg["reload_animation"], 14)
        struct.pack_into("<I", definition, 0x480, 12)
        with self.assertRaises(ValueError):
            animation_config({}, capture, definition)

    def test_reload_binding_rejects_missing_or_ambiguous_selected_animation(self):
        import base64
        for missing in (True, False):
            state, capture, arsenal = inputs()
            definition = base64.b64decode(arsenal["weapons"][0]["definition_base64"])
            if missing:
                capture["animations"] = [a for a in capture["animations"] if a["name"] != "reload_rifle"]
            else:
                capture["animations"].append({"source":0x07817b80,"name":"reload_rifle","index":90})
            with self.assertRaises(ValueError):
                animation_config({}, capture, definition)


if __name__ == "__main__":
    unittest.main()
