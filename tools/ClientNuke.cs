// Fixed read-only radio and nuclear weapon/layout capture for the marked 3.3.3.3 client.
using System;
using System.Collections.Generic;
using System.Web.Script.Serialization;

partial class ClientAssets {
    // Minimal scalar read layouts, statically recovered for this pinned build.
    // Only ReadProcessMemory is used; none of these native getters is called.
    static readonly uint[,] RadarCodecs=new uint[,] {
        {0x004016b0,0x00ce481c,0,1,1},
        {0x004017b0,0x00ce4fb4,8,10,0},
        {0x004018c0,0x00ce4fc0,16,6,0},
        {0x004019c0,0x00ce4fcc,16,10,0},
        {0x00401ad0,0x00ce4fd8,8,8,1},
        {0x00401bf0,0x00ce4fe4,8,10,0},
        {0x00401d00,0x00ce4ff0,16,2,1},
        {0x00401e10,0x00ce4ffc,24,5,0},
        {0x00401f30,0x00ce5008,16,12,0},
        {0x00402040,0x00ce5014,24,12,0},
        {0x00402160,0x00ce5020,8,8,1},
        {0x00402280,0x00ce502c,8,10,0},
        {0x004023a0,0x00ce5038,24,8,1},
        {0x004024c0,0x00ce5044,8,5,0},
        {0x004025e0,0x00ce5050,8,12,0},
        {0x00402700,0x00ce505c,16,8,1},
        {0x00402810,0x00ce5068,8,10,0},
        {0x00402930,0x00ce5074,24,8,1},
        {0x00402a50,0x00ce5080,16,3,0},
        {0x00402b60,0x00ce508c,16,4,1},
        {0x00402c70,0x00ce5098,16,9,0},
        {0x00402d80,0x00ce50a4,24,6,0},
        {0x00402ea0,0x00ce50b0,24,5,0},
        {0x00402fd0,0x00ce50bc,8,9,0},
        {0x004030f0,0x00ce50c8,24,6,0},
        {0x00403210,0x00ce50d4,24,10,0},
        {0x00403330,0x00ce50e0,16,5,0},
        {0x00403430,0x00ce50ec,16,12,0},
        {0x00403540,0x00ce50f8,24,9,0},
        {0x00403660,0x00ce5104,24,10,0},
        {0x00403780,0x00ce5110,24,2,1},
        {0x004038a0,0x00ce511c,16,5,0},
        {0x004039b0,0x00ce5128,8,3,0},
        {0x00403ad0,0x00ce5134,24,10,0},
        {0x00403bf0,0x00ce5140,8,5,0},
    };
    object RadarFlags(uint address,uint getter) {
        for(int row=0;row<RadarCodecs.GetLength(0);row++) {
            if(RadarCodecs[row,0]!=getter)continue;
            int rotate=(int)RadarCodecs[row,2];
            uint bits=Word(RadarCodecs[row,1])^(rotate==0?address:(address>>rotate)|(address<<(32-rotate)));
            if(RadarCodecs[row,4]!=0)bits=~bits;
            for(int word=0;word<4;word++)if((RadarCodecs[row,3]&(1u<<word))!=0)bits^=Word(address+(uint)word*4);
            float value=BitConverter.ToSingle(BitConverter.GetBytes(bits),0);
            return !Single.IsNaN(value)&&!Single.IsInfinity(value)&&value>=0&&value<=63&&value==(int)value ? (object)(uint)value : null;
        }
        return null;
    }
    public static void ExportNukeAssets(IntPtr process,bool all=false) {
        var r=new ClientAssets(process);int count=Count(r.Word(0x07817b88),899);
        var weapons=new List<object>();
        for(uint i=1;i<=count;i++) {
            uint def=r.Word(0x017f5a40+i*4);string name=r.Name(r.Word(def));
            if(!all && name!="killstreak_nuke_mp" && name!="nuke_mp" && name!="killstreak_uav_mp" && name!="killstreak_counter_uav_mp")continue;
            weapons.Add(Obj("name",name,"index",i,"definition_base64",Convert.ToBase64String(r.Read(def,0xe28)),
                "runtime_fields",r.WeaponRuntimeFields(def)));
        }
        var effects=new List<object>();
        for(uint i=1;i<=399;i++) {
            uint fx=r.Word(0x01a1f9bc+i*4);if(fx==0)continue;
            string name=r.Name(r.Word(fx));
            if(all||name=="explosions/player_death_nuke"||name=="explosions/player_death_nuke_flash"||name=="dust/nuke_aftermath_mp")
                effects.Add(Obj("name",name,"selector",i));
        }
        var sounds=new List<object>();
        foreach(string name in new string[]{"nuke_incoming","nuke_explosion","nuke_wave","ui_mp_nukebomb_timer","nuke_music"}) {
            uint entry;uint asset=r.Find(11,name,out entry,false);
            sounds.Add(Obj("name",name,"found",asset!=0,"loaded",asset!=0&&(r.Read(entry,28)[17]&5)!=0));
        }
        var aceTables=new Dictionary<string,object>();
        if(all)foreach(string name in new string[]{"mp/item_etk.csv","mp/item_kis.csv","mp/coAceStreakStatsTable.csv","mp/coAceKillstreakStatsTable.csv","mp/coAceKillstreakTable.csv"})aceTables[name]=r.Table(name,false);
        var optional=new List<object>();
        if(all)foreach(string name in new string[]{"killstreak_locker_mp","super_katana_mp","killstreak_consumable_mp","killstreak_pro_ac130_mp"}) {
            uint entry;uint asset=r.Find(28,name,out entry,false);
            optional.Add(Obj("name",name,"found",asset!=0,"loaded",asset!=0&&(r.Read(entry,28)[17]&5)!=0,"layout",asset==0?null:r.Layout(asset,0x40,1)));
        }
        var models=new List<object>();
        if(all) {
            uint slot=r.Word(0x07817bac);if(slot>=256)throw new Exception("invalid reward model bank");
            uint cgs=r.Word(0x01a19510+slot*4)^0xce9af8cf;
            byte[] registered=r.Read(cgs+0xf0,1024*4);
            for(uint id=1;id<1024;id++) {
                uint model=U(registered,(int)id*4);if(model==0)continue;
                byte[] bounds=r.Read(model+0x140,24);
                models.Add(Obj("name",r.Name(r.Word(model)),"index",id,"bounds_center",Floats(bounds,0,3),"bounds_half",Floats(bounds,12,3)));
            }
        }
        var output=Obj("schema_version",1,"read_only",true,
            "client_sha256","e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b",
            "connection_state",r.Word(0x01a2e7ec),"map",r.Name(r.Word(0x02dfb000)),
            "ncs_crc",r.Word(0x02df7d90),"weapons",weapons,"effects",effects,"sounds",sounds,"models",models,"optional_assets",optional,"ace_tables",aceTables);
        var serializer=new JavaScriptSerializer();serializer.MaxJsonLength=16*1024*1024;Console.WriteLine(serializer.Serialize(output));
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
            uint compassBank=r.Word(0x07817bc0);if(compassBank>=256)throw new Exception("invalid compass bank");
            uint compass=r.Word(0x01a19510+compassBank*4)^0x680db32c;
            uint radarGetter=(cg+0x228)^r.Word(cg+0x22c)^r.Word(0x00ce5188);
            uint radarAddress=cg+0x218;
            object radarFlags=r.RadarFlags(radarAddress,radarGetter);
            uint esSlot=r.Word(0x07817bb0);if(esSlot>=256)throw new Exception("invalid entity bank");
            uint entities=r.Word(0x01a19510+esSlot*4)^0x2168f672;
            var actors=new List<object>();
            for(uint id=0;id<12;id++) {
                uint ent=entities+id*0x238;
                uint ci=cg+0x15dea8+id*0x3b8;if(r.Word(ci)==0||r.Word(ci+4)==0)continue;
                uint actor=compass+id*0x100;
                actors.Add(Obj("client",id,"team",r.Word(ci+0x7c),"flags",r.Word(ent+0x204),"type",r.Word(ent+0xf4),"origin",r.EntityOrigin(ent+0xf0),
                    "compass_last_update",r.Word(actor),"compass_last_entity",r.Word(actor+0x54),
                    "compass_last_scan",r.Word(actor+0x58),"compass_flags",r.Word(actor+0x6c)));
            }
            frames.Add(Obj("time",r.Word(cg+0xae4c8),"pm_type",r.Word(cg+4),"health",r.Word(cg+0x190),
                "player_flags",r.Word(cg+0x10),"ability_mask_0",r.Word(cg+0x4cc),
                "progress",r.Word(cg+0x64),"ids",ids,"targets",targets,"action_types",types,"action_weapons",weapons,
                "weapon",r.Word(cg+0x340),"weapon_state",r.Word(cg+0x27c),"weapon_animation",r.Word(cg+0x26c),
                "selected_weapon",r.Word(cg+0xbd00c),"view_offset",Floats(r.Read(cg+0xa0,12),0,3),
                "inventory",inventory,"replay_active",r.Word(cg+0xc6fe8),"actors",actors,
                "radar_phase",BitConverter.ToSingle(r.Read(cg+0xc6f98,4),0),
                "radar_flags",radarFlags,
                "radar_protected_address",cg+0x218,"radar_protected_base64",Convert.ToBase64String(r.Read(cg+0x218,24)),
                "radar_getter_address",radarGetter,"radar_getter_base64",Convert.ToBase64String(r.Read(radarGetter,256)),
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
