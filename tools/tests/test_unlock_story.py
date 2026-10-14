import base64
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
sys.path.insert(0,str(Path(__file__).resolve().parent))
from test_unlock_all_items import UID, fields, profile
from unlock_all_items import checked_profile
import unlock_story


class StoryUnlock(unittest.TestCase):
    def data(self):
        layout=fields(36,True)
        layout["unlockFlagsNew[150]"]=dict(type=9,width=2,offset=8595,projected=False)
        rows=[["cf_ch_coop_training_mp_cargoship","150",""],["uf_m4_mp_rank","754","1"],["","",""]]
        assets=dict(schema_version=1,read_only=True,client_sha256=unlock_story.CLIENT_SHA256,
                    tables={"mp/unlockFlags.csv":dict(found=True,columns=3,rows=3,
                    cells_base64=[base64.b64encode(v.encode()).decode() for r in rows for v in r])})
        original={"private.bin":profile(layout,True),"public.bin":b"retained-public",
                  "inventory.json":b"retained-items","wallet.json":b"retained-wallet"}
        return layout,assets,original

    def test_native_story_grant_preserves_unrelated_progress_and_is_idempotent(self):
        layout,assets,original=self.data()
        with patch.object(unlock_story,"schema",return_value=layout):
            replacements,report=unlock_story.prepare(assets,UID,original)
            self.assertEqual(set(replacements),{"private.bin"})
            before=checked_profile(original["private.bin"],layout,36)
            after=checked_profile(replacements["private.bin"],layout,36)
            self.assertEqual([i for i in range(4,32768) if before[i]!=after[i]],[8595])
            self.assertFalse(report["authoritative_completion"])
            self.assertFalse(report["rewards_granted"])
            self.assertEqual(unlock_story.prepare(assets,UID,dict(original,**replacements))[0],{})
            with self.assertRaises(ValueError):unlock_story.prepare(assets,UID+1,original)

    def test_wrong_client_or_missing_first_flag_is_rejected(self):
        layout,assets,original=self.data()
        with patch.object(unlock_story,"schema",return_value=layout):
            assets["client_sha256"]="other"
            with self.assertRaises(ValueError):unlock_story.prepare(assets,UID,original)
            assets["client_sha256"]=unlock_story.CLIENT_SHA256
            assets["tables"]["mp/unlockFlags.csv"]["cells_base64"][0]=base64.b64encode(b"other").decode()
            with self.assertRaises(ValueError):unlock_story.prepare(assets,UID,original)
