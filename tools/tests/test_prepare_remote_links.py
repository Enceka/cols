import copy
from pathlib import Path
import struct
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from prepare_remote_links import NAKED_VISION_ANCHORS, verify_naked_vision, THERMAL_VISION_ANCHORS, verify_thermal_vision, ANCHORS, VISION_ANCHORS, FOF_ANCHORS, STATIC_EXIT_ANCHORS, WEAPON_VIEW_ANCHORS, verify_image, verify_vision, verify_fof, verify_static_exit, verify_weapon_view


def memory():
    data = {address: bytes.fromhex(raw) for address, raw, _ in ANCHORS}
    for address, fmt, value in [(0xc85f58, 'd', -127.0),
                                (0xc85608, 'd', 0.0010000000474974513),
                                (0xb613f0, 'f', 100.0), (0xc85828, 'f',35.0),
                                (0xc5aec8,'f',20.0),(0xc862d8,'f',87.0),(0xc855b4,'f',3000.0),(0xc85a04,'f',6000.0),
                                (0xc85594,'f',2000.0),(0xc855b8,'f',500.0)]:
        data[address] = struct.pack('<'+fmt, value)
    return data


class RemoteLinkConsumerTests(unittest.TestCase):
    def test_naked_vision_command_uses_slot_zero_and_integer_milliseconds(self):
        data = {va: bytes.fromhex(raw) for va, raw, _ in NAKED_VISION_ANCHORS}
        data[0x57e2a4 + ord('N')] = bytes([24])
        data[0x57e1ac + 24 * 4] = struct.pack('<I', 0x57dcc7)
        read = lambda va, n: data[va][:n]
        self.assertEqual(verify_naked_vision(read)['slot'], 0)
        for va, original in list(data.items()):
            data[va] = bytes([original[0] ^ 1]) + original[1:]
            with self.assertRaises(ValueError): verify_naked_vision(read)
            data[va] = original

    def test_thermal_command_and_renderer_use_the_same_slot(self):
        data = {va: bytes.fromhex(raw) for va, raw, _ in THERMAL_VISION_ANCHORS}
        data[0x57e2a4 + ord('Q')] = bytes([27])
        data[0x57e1ac + 27 * 4] = struct.pack('<I', 0x57ddb4)
        read = lambda va, n: data[va][:n]
        self.assertEqual(verify_thermal_vision(read)['slot'], 3)
        for va, original in list(data.items()):
            data[va] = bytes([original[0] ^ 1]) + original[1:]
            with self.assertRaises(ValueError): verify_thermal_vision(read)
            data[va] = original

    def test_weapon_view_and_thermal_are_target_instructions_not_camera_aliases(self):
        data={va:bytes.fromhex(raw) for va,raw,_ in WEAPON_VIEW_ANCHORS}
        data[0xc857c0]=struct.pack('<f',180)
        read=lambda va,n:data[va][:n]
        r=verify_weapon_view(read)
        self.assertEqual((r['eflags'],r['entity_index'],r['thermal_flag']),(4,68,8))
        for va,old in list(data.items()):
            data[va]=bytes([old[0]^1])+old[1:]
            with self.assertRaises(ValueError):verify_weapon_view(read)
            data[va]=old
    def test_static_exit_fullscreen_extents_and_material_lookup_are_target_bound(self):
        data = {va: bytes.fromhex(raw) for va, raw, _ in STATIC_EXIT_ANCHORS}
        read = lambda va, n: data[va][:n]
        result = verify_static_exit(read)
        self.assertEqual((result['width_offset'], result['height_offset']), (68,72))
        self.assertEqual(result['align_screen'], 0x44)
        for address, original in list(data.items()):
            data[address] = bytes([original[0] ^ 1]) + original[1:]
            with self.assertRaises(ValueError): verify_static_exit(read)
            data[address] = original

    def test_fof_uses_target_playerstate_flag_and_native_material_consumers(self):
        data = {va: bytes.fromhex(raw) for va, raw, _ in FOF_ANCHORS}
        data[0xc5f450] = b'hud_fofbox_hostile\0'
        data[0xc5f480] = b'hud_fofbox_self\0'
        read = lambda va, n: data[va][:n]
        self.assertEqual(verify_fof(read)['flag'], 16)
        self.assertEqual(verify_fof(read)['descriptor_index'], 54)
        for address, original in list(data.items()):
            data[address] = bytes([original[0] ^ 1]) + original[1:]
            with self.assertRaises(ValueError):
                verify_fof(read)
            data[address] = original

    def test_missile_vision_checks_both_dispatch_levels_and_consumers(self):
        data = {va: bytes.fromhex(raw) for va, raw, _ in VISION_ANCHORS}
        data[0x57e2a4 + ord('P')] = bytes([26])
        data[0x57e1ac + 26 * 4] = struct.pack('<I', 0x57dd64)
        read = lambda va, n: data[va][:n]
        result = verify_vision(read)
        self.assertEqual((result['command'], result['slot']), ('P', 2))
        self.assertEqual(result['transition_unit'], 'milliseconds')
        for address, original in list(data.items()):
            data[address] = bytes([original[0] ^ 1]) + original[1:]
            with self.assertRaises(ValueError):
                verify_vision(read)
            data[address] = original

    def test_rejects_changed_consumer_or_wrong_sign_and_retains_native_constants(self):
        image = memory()
        code, constants = verify_image(lambda va, n: image[va][:n])
        self.assertEqual(len(code), 23)
        self.assertEqual(constants['remote_byte_divisor']['value'], -127.0)
        for address in image:
            bad = copy.deepcopy(image)
            bad[address] = bytes([bad[address][0] ^ 1]) + bad[address][1:]
            with self.assertRaises(ValueError):
                verify_image(lambda va, n: bad[va][:n])
        image[0xc85f58] = struct.pack('<d', 127.0)
        with self.assertRaises(ValueError):
            verify_image(lambda va, n: image[va][:n])


if __name__ == '__main__':
    unittest.main()
