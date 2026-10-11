import base64
import json
import struct
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from prepare_player_appearance import prepare, native_hash, CLIENT_SHA256, GROUPS


def inputs():
    layout = json.loads((Path(__file__).resolve().parents[2] / "fixtures/protocol/character-slots-3.3.3.3.json").read_text())
    fields = {"currentCharacterIndex":dict(offset=layout["current_character_offset"], type=1, width=1)}
    for c, row in enumerate(layout["slot_offsets"]):
        for s, offset in enumerate(row):
            fields[f"characterClasses[{c}].slots[{s}]"] = dict(offset=offset, type=0, width=4)
    package = bytearray(40)
    package[2] = 1
    package[3] = 2
    struct.pack_into("<12H", package, 8, 0,101,102,103,104,105,0,0,0,0,0,0)
    capture = dict(client_sha256=CLIENT_SHA256,read_only=True,
                   items=[dict(item_id=792,name="cha_test",kind=0,type="cha",package_base64=base64.b64encode(package).decode(),members=[106])],
                   variants=[])
    records = bytearray()
    groups = bytearray()
    for i, group in enumerate(GROUPS):
        groups += struct.pack("<HH", len(records)//8, int(i<5))
        if i<5:
            name=group+"_test"
            capture["items"].append(dict(item_id=101+i,name=name,type="cat",kind=i+1))
            records += struct.pack("<HHI",1,0xffff,native_hash(name))
            capture["variants"].append(dict(group=group,index=1,model=name+"_tf",selectors_base64=base64.b64encode(struct.pack("<HI",0,0x62000000)).decode()))
    capture.update(variant_items_base64=base64.b64encode(records).decode(),
                   variant_groups_base64=base64.b64encode(groups).decode(),
                   selector_hashes_base64=base64.b64encode(struct.pack("<9I",*[native_hash(n) for n in ["m","f","tf","sc","zm","wht","yel","blk","cau"]])).decode())
    assets=dict(client_sha256=CLIENT_SHA256,read_only=True,schemas={"mp/playerdata.def":{"definitions":[{"version":233,"formatChecksum":3811397846}]}})
    return capture,assets,fields


class PlayerAppearancePreparation(unittest.TestCase):
    def test_native_hand_composition_keeps_group_indices_and_checks_selector_mapping(self):
        c,a,f=inputs()
        offsets=[0x18,0x1c,0x20,0x14,0x24,0x28,0x2c,0x44,0x48,0x4c,0x54,0x40,0x50,0x30,0x34,0x38,0x3c,0x58,0x5c,0x60,0x64]
        enc=lambda v:base64.b64encode(struct.pack('<'+'I'*len(v),*v)).decode()
        c['packing_bits_base64']=enc([7,4,8,8,6,7,8,5,0,8,0])
        c['packing_fields_base64']=enc([v for o in offsets for v in (o,10 if o<0x40 else 6,0)])
        c['gender_selectors_base64']=enc([2,3,4]);c['faction_selectors_base64']=enc([0,1]);c['skin_selectors_base64']=enc([5,6,7,8])
        with patch('prepare_player_appearance.flatten',return_value=f):
            result=prepare(c,a)
            self.assertEqual(result['items'][0]['native_index'],1)
            self.assertEqual(result['packing_bits'][:3],[7,4,8])
            c['faction_selectors_base64']=enc([1,0])
            with self.assertRaises(ValueError):prepare(c,a)

    def test_package_and_variant_index_are_separate_and_profile_slot_order_is_preserved(self):
        capture,assets,fields=inputs()
        with patch("prepare_player_appearance.flatten", return_value=fields):
            result=prepare(capture,assets)
        self.assertEqual(result["packages"][0]["gender"],1)
        self.assertEqual(result["packages"][0]["skin"],2)
        self.assertEqual(result["packages"][0]["defaults"][1:6],[101,102,103,104,105])
        self.assertEqual(result["items"][0]["variants"],[dict(mask=0x62000000,model="fpa_test_tf")])
        self.assertEqual(result["slot_offsets"][0][6],8513)

    def test_wrong_build_selector_order_and_ambiguous_item_hash_are_rejected(self):
        for issue in ["identity","selector","ambiguous","schema"]:
            capture,assets,fields=inputs()
            if issue=="identity":capture["read_only"]=False
            if issue=="selector":capture["selector_hashes_base64"]=base64.b64encode(bytes(36)).decode()
            if issue=="ambiguous":
                records=base64.b64decode(capture["variant_items_base64"])
                capture["variant_items_base64"]=base64.b64encode(records+records[:8]).decode()
                groups=bytearray(base64.b64decode(capture["variant_groups_base64"]))
                struct.pack_into("<HH",groups,0,0,6)
                capture["variant_groups_base64"]=base64.b64encode(groups).decode()
            if issue=="schema":fields["characterClasses[0].slots[0]"]["type"]=4
            with self.subTest(issue=issue), patch("prepare_player_appearance.flatten",return_value=fields), self.assertRaises(ValueError):
                prepare(capture,assets)


if __name__ == "__main__":
    unittest.main()
