import base64
import copy
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from prepare_game_state import CLIENT_SHA256
from prepare_map_registry import prepare


class NativeMapRegistry(unittest.TestCase):
    def capture(self):
        rows = []
        for name, native_id in [('mp_afghan', 1), ('za_coliseum', 83)]:
            row = [''] * 37
            row[:4] = ['1', '1', '1', str(native_id)]
            row[4], row[5], row[8], row[9] = name, 'MPUI_TEST', 'pvp', 'war;dom'
            row[10], row[11] = ('1', '4') if name == 'za_coliseum' else ('4', '18')
            rows.extend(base64.b64encode(c.encode()).decode() for c in row)
        return dict(schema_version=1, read_only=True, client_sha256=CLIENT_SHA256,
                    tables={'mp/coMapList.csv': dict(found=True, columns=37, rows=2, cells_base64=rows)})

    def test_ids_and_modes_are_preserved_without_claiming_asset_preparation(self):
        registry = prepare(self.capture())
        self.assertEqual([(m['name'], m['native_id']) for m in registry['maps']],
                         [('mp_afghan', 1), ('za_coliseum', 83)])
        self.assertFalse(registry['resource_preparation_claimed'])
        self.assertEqual((registry['maps'][1]['minimum_players'], registry['maps'][1]['maximum_players']), (1, 4))

    def test_native_custom_room_prohibition_and_capacity_are_preserved_and_bounded(self):
        capture = self.capture()
        cells = capture['tables']['mp/coMapList.csv']['cells_base64']
        cells[35] = base64.b64encode(b'1').decode()
        self.assertTrue(prepare(capture)['maps'][0]['no_custom_room'])
        for column, value in [(10, '0'), (10, '19'), (11, '33'), (35, '2')]:
            malformed = self.capture()
            malformed['tables']['mp/coMapList.csv']['cells_base64'][column] = base64.b64encode(value.encode()).decode()
            with self.subTest(column=column, value=value), self.assertRaises(ValueError):
                prepare(malformed)

    def test_wrong_capture_layout_paths_and_ambiguous_names_or_ids_are_rejected(self):
        for change in ('client', 'read_only', 'layout', 'length', 'path', 'duplicate_id', 'duplicate_name'):
            capture = self.capture()
            table = capture['tables']['mp/coMapList.csv']
            cells = table['cells_base64']
            if change == 'client':
                capture['client_sha256'] = 'other'
            elif change == 'read_only':
                capture['read_only'] = False
            elif change == 'layout':
                table['columns'] = 36
            elif change == 'length':
                cells.pop()
            elif change == 'path':
                cells[4] = base64.b64encode(b'../mp_afghan').decode()
            elif change == 'duplicate_id':
                cells[40] = cells[3]
            elif change == 'duplicate_name':
                cells[41] = cells[4]
            with self.subTest(change=change), self.assertRaises(ValueError):
                prepare(capture)

    def test_other_modes_and_categories_do_not_expand_the_tdm_registry(self):
        for column, value in [(8, 'pve'), (9, 'sd;dom')]:
            capture = copy.deepcopy(self.capture())
            capture['tables']['mp/coMapList.csv']['cells_base64'][column] = base64.b64encode(value.encode()).decode()
            self.assertEqual([m['native_id'] for m in prepare(capture)['maps']], [83])


if __name__ == '__main__':
    unittest.main()
