import copy
import struct
import sys
import unittest
import zlib
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from prepare_player_loadout import selection, set_parts


def profile(selected, primary=10001):
    plain = bytearray(32768)
    struct.pack_into('<7I', plain, 1000, primary, *selected)
    crc = zlib.crc32(plain[4:])
    struct.pack_into('<I', plain, 0, crc)
    return (b'\x08' + struct.pack('<I', 0) + b'\x08' + struct.pack('<I', 1)
            + b'\x08' + struct.pack('<I', crc) + b'\x13\x08'
            + struct.pack('<I', len(plain)) + plain + (b'\x07' + bytes(4))*125 + b'\0')


class PlayerLoadout(unittest.TestCase):
    def setUp(self):
        self.config = dict(primary_offset=1000, attachment_offsets=list(range(1004,1028,4)),
                           projection_offsets=list(range(12,512,4)), weapon_item_id=10001,
                           defaults=[dict(item_id=20032, kind=4, wire_index=1690),
                                     dict(item_id=20031, kind=9, wire_index=9641),
                                     dict(item_id=10587, kind=10, wire_index=13370)],
                           compatible=[dict(item_id=20659, kind=4, wire_index=11226),
                                       dict(item_id=21237, kind=9, wire_index=12223)])

    def test_saved_items_replace_defaults_and_keep_unselected_base_parts(self):
        original=copy.deepcopy(self.config)
        attachments, selected = selection(self.config, profile([21237,20659,0,0,0,0]))
        self.assertEqual(attachments, [11226,12223,13370])
        self.assertEqual(selected, [21237,20659])
        weapon=dict(base_index=163)
        set_parts(weapon,attachments,'a'*64)
        self.assertEqual(weapon['composed_name'],'** 163 0 0 11226 12223 13370')
        self.assertEqual(self.config,original)
        self.assertEqual(selection(self.config,profile([0]*6))[0],[1690,9641,13370])

    def test_conflicts_missing_mapping_wrong_weapon_and_projection_are_rejected(self):
        for ids in ([20659,20659,0,0,0,0],[99999,0,0,0,0,0]):
            with self.assertRaises(ValueError):selection(self.config,profile(ids))
        with self.assertRaises(ValueError):selection(self.config,profile([0]*6,10010))
        bad=bytearray(profile([0]*6));bad[-625]=1
        with self.assertRaises(ValueError):selection(self.config,bad)

class CatalogueMapping(unittest.TestCase):
    def inputs(self):
        import base64
        import json
        from prepare_game_state import CLIENT_SHA256
        property = lambda name, kind, index, leaf, extra=0: dict(name=name,type=kind,index=index,leaf=leaf,extra4=extra)
        definition=dict(version=233,formatChecksum=3811397846,size=2000,leafCount=132,
            rootType=5,rootIndex=0,enums=[],enumedArrays=[],indexedArrays=[
                dict(count=1,type=5,index=1),dict(count=6,type=0,index=0),dict(count=125,type=0,index=0)],
            structs=[dict(leafCount=132,properties=[property('weaponClasses',6,0,0),property('projection',6,2,7,32)]),
                     dict(leafCount=7,properties=[property('primaryBase',0,0,0),property('primaryAttach',6,1,1)])],
            offsets=[o-4 for o in list(range(1000,1028,4))+list(range(12,512,4))])
        rows=[['10001','m4_mp','wea','wea'],['20659','muz_slncarm4','wat','muz']]
        cells=[base64.b64encode(s.encode()).decode() for row in rows for s in row+['']*8]
        assets=dict(client_sha256=CLIENT_SHA256,read_only=True,schemas={'mp/playerdata.def':dict(definitions=[definition])},
                    tables={'mp/coMarketplaceItems.csv':dict(found=True,columns=12,rows=2,cells_base64=cells)})
        part=lambda id: dict(item_id=id,kind=4,name='muz_slncarm4',wire_index=11226)
        capture=dict(schema_version=1,read_only=True,client_sha256=CLIENT_SHA256,connection_state=10,ncs_crc=123,
            m4_default_parts=dict(weapon='m4_mp',parts=[dict(item_id=20032,kind=4,name='muz_default',wire_index=1690)]),
            compatible_attachments=[dict(index=11226,name='muz_slncarm4',items=[part(13666),part(20659)])])
        return assets,json.dumps(capture).encode(),dict(ncs_crc=123)

    def test_compact_id_aliases_are_filtered_by_independent_marketplace_identity(self):
        import json
        from prepare_player_loadout import catalogue
        assets, parts, state=self.inputs()
        result=catalogue(json.dumps(assets).encode(),parts,state,0)
        self.assertEqual([p['item_id'] for p in result['compatible']],[20659])
        self.assertEqual(result['primary_offset'],1000)
        self.assertEqual(result['attachment_offsets'],list(range(1004,1028,4)))

    def test_schema_overlap_and_wrong_version_are_rejected(self):
        import json
        from prepare_player_loadout import catalogue
        assets, parts, state=self.inputs()
        for key, val in [('version',220),('offsets',[996]*132)]:
            bad=copy.deepcopy(assets)
            bad['schemas']['mp/playerdata.def']['definitions'][0][key]=val
            with self.assertRaises(ValueError):catalogue(json.dumps(bad).encode(),parts,state,0)
