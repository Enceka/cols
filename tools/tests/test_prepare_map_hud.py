import base64
import copy
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from prepare_map_hud import CLIENT_SHA256, minimap, minimap_geometry


class MinimapPreparation(unittest.TestCase):
    def capture(self, yaw=90, corners=((4621, -1427), (2468, 1665))):
        data = ('{"classname" "worldspawn" "northyaw" "%s"}' % yaw).encode()
        for x, y in corners:
            data += ('{"classname" "script_origin" "targetname" "minimap_corner" "origin" "%s %s 200"}' % (x, y)).encode()
        return dict(schema_version=1, read_only=True, client_sha256=CLIENT_SHA256,
                    maps=[dict(name="maps/mp/mp_killhouse_sh.d3dbsp", byte_count=len(data),
                               entity_string_base64=base64.b64encode(data).decode())])

    def test_rotated_corners_preserve_the_captured_map_orientation(self):
        result = minimap(self.capture(), "mp_killhouse_sh")
        self.assertEqual(result["upper_left"], [1998.5, 1665])
        self.assertEqual(result["lower_right"], [5090.5, -1427])
        self.assertEqual(result["north_yaw"], 90)
        self.assertEqual(result, minimap(self.capture(corners=((2468, 1665), (4621, -1427))), "mp_killhouse_sh"))
        result = minimap(self.capture(yaw=0), "mp_killhouse_sh")
        self.assertEqual(result["upper_left"], [5090.5, 1665])
        self.assertEqual(result["lower_right"], [1998.5, -1427])

    def test_square_image_projection_keeps_the_map_centre_and_contains_both_corners(self):
        for corners in [((2048,4864),(-4192,7936)), ((-1798,1958),(2820,-4164))]:
            with self.subTest(corners=corners):
                result=minimap_geometry(self.capture(yaw=0,corners=corners),'mp_killhouse_sh')
                upper,lower=result['upper_left'],result['lower_right']
                self.assertEqual(upper[0]-lower[0],upper[1]-lower[1])
                for axis in range(2):
                    self.assertEqual(upper[axis]+lower[axis],sum(p[axis] for p in corners))
                    for point in corners:self.assertLessEqual(lower[axis],point[axis]);self.assertGreaterEqual(upper[axis],point[axis])
        result=minimap_geometry(self.capture(), 'mp_killhouse_sh', required_aspect_ratio=0)
        self.assertEqual(result['upper_left'],[2468,1665])
        self.assertEqual(result['lower_right'],[4621,-1427])
        for invalid in [-1,float('nan'),float('inf'),17]:
            with self.assertRaises(ValueError):minimap_geometry(self.capture(),'mp_killhouse_sh',invalid)

    def test_bad_identity_duplicate_resource_degenerate_and_missing_corners_fail(self):
        for capture in [self.capture(yaw="nan"), self.capture(corners=((0, 0), (0, 10))),
                        self.capture(corners=((0, 0),)), self.capture(corners=((0, 0), (10, 10), (20, 20))),
                        self.capture(corners=((0, 0), ("nan", 10)))]:
            with self.assertRaises(ValueError):
                minimap(capture, "mp_killhouse_sh")
        capture = self.capture()
        capture["maps"].append(copy.deepcopy(capture["maps"][0]))
        with self.assertRaises(ValueError):
            minimap(capture, "mp_killhouse_sh")
        with self.assertRaises(ValueError):
            minimap(self.capture(), "mp_afghan")

    def test_crash_uses_its_loaded_dlc_image_with_its_own_bounds(self):
        capture = self.capture(yaw=0, corners=((1976, 2528), (-1200, -2166)))
        capture["maps"][0]["name"] = "maps/mp/mp_crash_tropical.d3dbsp"
        result = minimap(capture, "mp_crash_tropical")
        self.assertEqual(result["material"], "compass_map_mp_crash_dlc")
        self.assertEqual(result["upper_left"], [2735, 2528])
        self.assertEqual(result["lower_right"], [-1959, -2166])

    def test_new_map_requires_its_captured_material_and_rejects_default_or_unrelated_images(self):
        capture=self.capture(yaw=0,corners=((100,100),(-100,-100)))
        capture['maps'][0]['name']='maps/mp/mp_bog_sh.d3dbsp'
        material=dict(schema_version=1,read_only=True,client_sha256=CLIENT_SHA256,
            map='maps/mp/mp_bog_sh.d3dbsp',ncs_crc=123,
            map_hud_materials={'compass_map_mp_bog_sh':dict(found=True,render_default=False,
                textures=[dict(width=512,height=512)])})
        result=minimap(capture,'mp_bog_sh',material)
        self.assertEqual(result['material'],'compass_map_mp_bog_sh')
        self.assertEqual(result['material_proof']['ncs_crc'],123)
        self.assertEqual(result['material_proof']['texture_dims'],[[512,512]])
        for change in ('missing','map','client','default','small'):
            altered=copy.deepcopy(material)
            if change=='missing':altered['map_hud_materials'].clear()
            elif change=='map':altered['map']='maps/mp/mp_afghan.d3dbsp'
            elif change=='client':altered['client_sha256']='other'
            elif change=='default':altered['map_hud_materials']['compass_map_mp_bog_sh']['render_default']=True
            else:altered['map_hud_materials']['compass_map_mp_bog_sh']['textures']=[dict(width=16,height=16)]
            with self.subTest(change=change),self.assertRaises(ValueError):minimap(capture,'mp_bog_sh',altered)

    def test_overgrown_dlc_alias_still_requires_its_own_map_and_texture_proof(self):
        capture=self.capture(yaw=0,corners=((100,100),(-100,-100)))
        capture['maps'][0]['name']='maps/mp/mp_overgrown.d3dbsp'
        material=dict(schema_version=1,read_only=True,client_sha256=CLIENT_SHA256,
            map='maps/mp/mp_overgrown.d3dbsp',ncs_crc=456,
            map_hud_materials={'compass_map_mp_overgrown_dlc':dict(found=True,render_default=False,
                textures=[dict(width=1024,height=1024)])})
        result=minimap(capture,'mp_overgrown',material)
        self.assertEqual(result['material'],'compass_map_mp_overgrown_dlc')
        self.assertEqual(result['material_proof']['map'],'maps/mp/mp_overgrown.d3dbsp')
        material['map']='maps/mp/mp_crash_tropical.d3dbsp'
        with self.assertRaises(ValueError):minimap(capture,'mp_overgrown',material)

    def test_ambush_uses_captured_convoy_texture_with_ambush_corners(self):
        capture=self.capture(yaw=0,corners=((180,220),(-140,-100)))
        capture['maps'][0]['name']='maps/mp/mp_ambush_sh.d3dbsp'
        material=dict(schema_version=1,read_only=True,client_sha256=CLIENT_SHA256,
            map='maps/mp/mp_ambush_sh.d3dbsp',ncs_crc=789,
            map_hud_materials={'compass_map_mp_convoy':dict(found=True,render_default=False,
                textures=[dict(width=1024,height=1024)])})
        result=minimap(capture,'mp_ambush_sh',material)
        self.assertEqual(result['material'],'compass_map_mp_convoy')
        self.assertEqual(result['upper_left'],[180,220])
        self.assertEqual(result['lower_right'],[-140,-100])
        material['map_hud_materials']['compass_map_mp_convoy']['render_default']=True
        with self.assertRaises(ValueError):minimap(capture,'mp_ambush_sh',material)


if __name__ == "__main__":
    unittest.main()
