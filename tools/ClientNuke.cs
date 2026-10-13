// Fixed read-only nuke weapon/layout capture for the marked 3.3.3.3 client.
using System;
using System.Collections.Generic;
using System.Web.Script.Serialization;

partial class ClientAssets {
    public static void ExportNukeAssets(IntPtr process) {
        var r=new ClientAssets(process);int count=Count(r.Word(0x07817b88),899);
        var weapons=new List<object>();
        for(uint i=1;i<=count;i++) {
            uint def=r.Word(0x017f5a40+i*4);string name=r.Name(r.Word(def));
            if(name!="killstreak_nuke_mp" && name!="nuke_mp")continue;
            weapons.Add(Obj("name",name,"index",i,"definition_base64",Convert.ToBase64String(r.Read(def,0xe28)),
                "runtime_fields",r.WeaponRuntimeFields(def)));
        }
        var effects=new List<object>();
        for(uint i=1;i<=399;i++) {
            uint fx=r.Word(0x01a1f9bc+i*4);if(fx==0)continue;
            string name=r.Name(r.Word(fx));
            if(name=="explosions/player_death_nuke"||name=="explosions/player_death_nuke_flash"||name=="dust/nuke_aftermath_mp")
                effects.Add(Obj("name",name,"selector",i));
        }
        var sounds=new List<object>();
        foreach(string name in new string[]{"nuke_incoming","nuke_explosion","nuke_wave","ui_mp_nukebomb_timer","nuke_music"}) {
            uint entry;uint asset=r.Find(11,name,out entry,false);
            sounds.Add(Obj("name",name,"found",asset!=0,"loaded",asset!=0&&(r.Read(entry,28)[17]&5)!=0));
        }
        var output=Obj("schema_version",1,"read_only",true,
            "client_sha256","e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b",
            "connection_state",r.Word(0x01a2e7ec),"map",r.Name(r.Word(0x02dfb000)),
            "ncs_crc",r.Word(0x02df7d90),"weapons",weapons,"effects",effects,"sounds",sounds);
        Console.WriteLine(new JavaScriptSerializer().Serialize(output));
    }
    public static void ExportNukeState(IntPtr process,bool once=false) {
        var r=new ClientAssets(process);var frames=new List<object>();
        for(int i=0;i<(once?1:400);i++) {
            if(r.Word(0x01a2e7ec)!=10)break;
            uint bank=r.Word(0x07817ba8);if(bank>=256)throw new Exception("invalid client bank");
            uint cg=r.Word(0x01a19510+bank*4)^0xfbb2c97a;
            var ids=new List<uint>();var targets=new List<uint>();var types=new List<uint>();var weapons=new List<uint>();
            for(uint slot=0;slot<5;slot++) {
                ids.Add(r.Word(cg+0x68+slot*4));targets.Add(r.Word(cg+0x7c+slot*4));
                types.Add(r.Word(cg+0x560+slot*4));weapons.Add(r.Word(cg+0x574+slot*4));
            }
            var inventory=new List<uint>();for(uint slot=0;slot<15;slot++)inventory.Add(r.Word(cg+0x2a4+slot*4));
            uint esSlot=r.Word(0x07817bb0);if(esSlot>=256)throw new Exception("invalid entity bank");
            uint entities=r.Word(0x01a19510+esSlot*4)^0x2168f672;
            var actors=new List<object>();
            for(uint id=0;id<12;id++) {
                uint ent=entities+id*0x238;
                uint ci=cg+0x15dea8+id*0x3b8;if(r.Word(ci)==0||r.Word(ci+4)==0)continue;
                actors.Add(Obj("client",id,"team",r.Read(0x0c95be10+260+id*198,1)[0],"flags",r.Word(ent+0x204),"type",r.Word(ent+0xf4),"origin",r.EntityOrigin(ent+0xf0)));
            }
            frames.Add(Obj("time",r.Word(cg+0xae4c8),"pm_type",r.Word(cg+4),"health",r.Word(cg+0x190),
                "progress",r.Word(cg+0x64),"ids",ids,"targets",targets,"action_types",types,"action_weapons",weapons,
                "weapon",r.Word(cg+0x340),"weapon_state",r.Word(cg+0x27c),"weapon_animation",r.Word(cg+0x26c),
                "selected_weapon",r.Word(cg+0xbd00c),"view_offset",Floats(r.Read(cg+0xa0,12),0,3),
                "inventory",inventory,"replay_active",r.Word(cg+0xc6fe8),"actors",actors,
                "hud_actor_valid",r.Word(cg+0x15dea8+r.Word(cg+0x3558)*0x3b8),
                "action_opacity",r.Read(0x07837fd4,20),
                "effects",r.EffectStates(),
                "pitch",BitConverter.ToSingle(r.Read(cg+0x14c,4),0),"yaw",BitConverter.ToSingle(r.Read(cg+0x150,4),0)));
            System.Threading.Thread.Sleep(50);
        }
        var output=Obj("schema_version",1,"read_only",true,
            "client_sha256","e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b","frames",frames);
        var serializer=new JavaScriptSerializer();serializer.MaxJsonLength=8*1024*1024;
        Console.WriteLine(serializer.Serialize(output));
    }
}
