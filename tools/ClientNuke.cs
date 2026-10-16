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
            var firingAliases=new Dictionary<string,object>();
            if(all)foreach(uint offset in new uint[]{0x4c0,0x4c4,0x4cc,0x4d0,0x4d8,0x4dc,0x4e0}) {
                uint pointer=r.Word(def+offset);string alias=null;
                if(pointer!=0)try{alias=r.Name(r.Word(pointer));}catch{}
                firingAliases["0x"+offset.ToString("x")]=alias;
            }
            object projectileExplosion=null;
            if(all&&name=="remotemissile_projectile_mp")
                projectileExplosion=r.EffectDefinition(r.Word(def+0x918));
            weapons.Add(Obj("name",name,"index",i,"definition_base64",Convert.ToBase64String(r.Read(def,0xe28)),
                "runtime_fields",r.WeaponRuntimeFields(def),"firing_aliases",firingAliases,
                "projectile_explosion_effect",projectileExplosion));
        }
        var effects=new List<object>();
        for(uint i=1;i<=399;i++) {
            uint fx=r.Word(0x01a1f9bc+i*4);if(fx==0)continue;
            string name=r.Name(r.Word(fx));
            if(all||name=="explosions/player_death_nuke"||name=="explosions/player_death_nuke_flash"||name=="dust/nuke_aftermath_mp")
                effects.Add(Obj("name",name,"selector",i));
        }
        var sounds=new List<object>();
        foreach(string name in new string[]{"nuke_incoming","nuke_explosion","nuke_wave","ui_mp_nukebomb_timer","nuke_music","weap_cobra_20mm_fire_npc"}) {
            uint entry;uint asset=r.Find(11,name,out entry,false);
            sounds.Add(Obj("name",name,"found",asset!=0,"loaded",asset!=0&&(r.Read(entry,28)[17]&5)!=0));
        }
        var remoteVision=new Dictionary<string,object>();
        if(all)foreach(string name in new string[]{"vision/black_bw.vision","vision/thermal_mp.vision","vision/thermal_snowlevel_mp.vision","vision/missilecam.vision","vision/mpnuke.vision","vision/mpnuke_aftermath.vision"}) {
            try{remoteVision[name]=r.RawFile(name,256*1024,false);}catch(Exception e){remoteVision[name]=Obj("error",e.Message);}
        }
        if(all)try{remoteVision["mp/basemaps.arena"]=r.RawFile("mp/basemaps.arena",2*1024*1024,false);}catch(Exception e){remoteVision["mp/basemaps.arena"]=Obj("error",e.Message);}
        var remoteMaterials=new List<object>();
        if(all)foreach(string name in new string[]{"white","ac130_overlay_grain","hud_fofbox_hostile","hud_fofbox_self"}) {
            uint entry;uint asset=r.Find(5,name,out entry,false);
            remoteMaterials.Add(Obj("name",name,"found",asset!=0,"loaded",asset!=0&&(r.Read(entry,28)[17]&5)!=0));
        }
        var aceTables=new Dictionary<string,object>();
        if(all)foreach(string name in new string[]{"mp/item_etk.csv","mp/item_kis.csv","mp/coAceStreakStatsTable.csv","mp/coAceKillstreakStatsTable.csv","mp/coAceKillstreakTable.csv"})aceTables[name]=r.Table(name,false);
        var optional=new List<object>();
        if(all)foreach(string name in new string[]{"killstreak_locker_mp","super_katana_mp","killstreak_consumable_mp","killstreak_pro_ac130_mp"}) {
            uint entry;uint asset=r.Find(28,name,out entry,false);
            optional.Add(Obj("name",name,"found",asset!=0,"loaded",asset!=0&&(r.Read(entry,28)[17]&5)!=0,"layout",asset==0?null:r.Layout(asset,0x40,1)));
        }
        var actorRawfiles=new Dictionary<string,object>();
        if(all)foreach(string name in new string[]{"animlists/common_dog.animlist","animtrees/dog.atr"}) {
            try{actorRawfiles[name]=r.RawFile(name,256*1024,false);}catch(Exception e){actorRawfiles[name]=Obj("error",e.Message);}
        }
        var actorTrees=new List<object>();uint actorMask=0;
        if(all) {
            uint bgs=r.Word(0x07817b80);actorMask=r.Word(bgs+0x96de8);
            for(uint selector=0;selector<6;selector++) {
                uint tree=r.Word(bgs+0x94d54+selector*20);if(tree==0)continue;
                var ids=new List<uint>();
                for(uint id=1;id<16;id++)if(r.Word(0x02f371c4+id*4)==tree)ids.Add(id);
                var partsList=new List<object>();var parents=new List<uint>();int countNodes=Count(r.Word(tree),2048);
                for(int node=0;node<countNodes;node++)parents.Add(BitConverter.ToUInt16(r.Read(tree+6+(uint)node*8,2),0));
                for(int node=1;node<countNodes;node++) {
                    uint at=tree+4+(uint)node*8;
                    if(BitConverter.ToUInt16(r.Read(at,2),0)!=0)continue;
                    uint parts=r.Word(at+4);string name;
                    try{name=r.Name(r.Word(parts));}catch{continue;}
                    partsList.Add(Obj("node",node,"parent",BitConverter.ToUInt16(r.Read(at+2,2),0),"name",name,
                        "loop_flag",r.Read(parts+0x10,1)[0]&1,"frames",BitConverter.ToUInt16(r.Read(parts+0xe,2),0),
                        "frame_rate",BitConverter.ToSingle(r.Read(parts+0x28,4),0)));
                }
                actorTrees.Add(Obj("selector",selector,"registered_tree_ids",ids,"tree_layout",r.Layout(tree,0x100,1),"parts",partsList,"parents",parents));
            }
        }
        var models=new List<object>();
        if(all) {
            uint slot=r.Word(0x07817bac);if(slot>=256)throw new Exception("invalid reward model bank");
            uint cgs=r.Word(0x01a19510+slot*4)^0xce9af8cf;
            byte[] registered=r.Read(cgs+0xf0,1024*4);
            for(uint id=1;id<1024;id++) {
                uint model=U(registered,(int)id*4);if(model==0)continue;
                byte[] bounds=r.Read(model+0x140,24);
                string name=r.Name(r.Word(model));var bones=new List<string>();var attachments=new Dictionary<string,object>();
                if(name.StartsWith("sentry",StringComparison.Ordinal)||name.StartsWith("t5_weapon_sam",StringComparison.Ordinal)||name.StartsWith("t5_veh_rcbomb_",StringComparison.Ordinal)||name.StartsWith("vehicle_",StringComparison.Ordinal)||name.StartsWith("german_sheperd",StringComparison.Ordinal)||name=="weapon_minigun"||name=="c130_zoomrig"||name=="tag_origin") {
                    int countBones=Count(r.Read(model+4,1)[0],253);byte[] ids=r.Read(r.Word(model+0x2c),countBones*2);uint strings=r.Word(0x02f37164);
                    for(int bone=0;bone<countBones;bone++) {
                        string boneName=r.Name(strings+4+(uint)BitConverter.ToUInt16(ids,bone*2)*20);bones.Add(boneName);
                        // Current0x684244 reads XModel+0x40's 32-byte bind
                        // matrices before rotating a vehicle attachment point.
                        if(((name.StartsWith("t5_veh_rcbomb_",StringComparison.Ordinal)||name.StartsWith("vehicle_",StringComparison.Ordinal)||name=="c130_zoomrig")&&(boneName=="tag_player"||boneName=="tag_origin"))||boneName=="tag_gunner_left"||boneName=="tag_gunner_right"||name=="weapon_minigun"&&boneName.StartsWith("tag_",StringComparison.Ordinal))
                            attachments[boneName]=Floats(r.Read(r.Word(model+0x40)+(uint)bone*32,32),0,8);
                    }
                }
                models.Add(Obj("name",name,"index",id,"bounds_center",Floats(bounds,0,3),"bounds_half",Floats(bounds,12,3),"bone_names",bones,"attachment_bind_matrices",attachments));
            }
        }
        var vehicles=new List<object>();var vehicleTags=new List<object>();var vehicleTrees=new List<object>();
        if(all) {
            // Target initialization0x683762: net constants0xc04..0xc22,
            // asset type41 and registry0x37a49d0; bounded read only.
            for(uint id=1;id<32;id++) {
                uint def=r.Word(0x037a49d0+id*4);if(def==0)continue;
                vehicles.Add(Obj("index",id,"name",r.Name(r.Word(def)),"layout",r.Layout(def,0x2d0,1)));
            }
            foreach(uint address in new uint[]{0x02c24a50,0x02c24a52,0x02c24a56,0x02c24992,0x02c2498e}) {
                uint id=BitConverter.ToUInt16(r.Read(address,2),0),pool=r.Word(0x02f37164);
                vehicleTags.Add(Obj("address",address,"name",r.Name(pool+id*20+4)));
            }
            for(uint id=1;id<16;id++) {
                uint tree=r.Word(0x02f371c4+id*4);if(tree==0)continue;
                int countNodes=Count(r.Word(tree),2048);var rotorParts=new List<object>();var allParts=new List<object>();
                for(int node=0;node<countNodes;node++) {
                    if(BitConverter.ToUInt16(r.Read(tree+4+(uint)node*8,2),0)!=0)continue;
                    uint parts=r.Word(tree+8+(uint)node*8);string animName=null;
                    try {animName=r.Name(r.Word(parts));}catch {continue;}
                    allParts.Add(Obj("node",node,"name",animName,"loop_flag",r.Read(parts+0x10,1)[0]&1));
                    if(animName!="bh_rotors")continue;
                    int countBones=Count(r.Read(parts+0x1a,1)[0],253);byte[] ids=r.Read(r.Word(parts+0x30),countBones*2);uint pool=r.Word(0x02f37164);var names=new List<string>();
                    for(int bone=0;bone<countBones;bone++)names.Add(r.Name(pool+4+(uint)BitConverter.ToUInt16(ids,bone*2)*20));
                    rotorParts.Add(Obj("node",node,"name",animName,"bone_names",names,"loop_flag",r.Read(parts+0x10,1)[0]&1));
                }
                vehicleTrees.Add(Obj("index",id,"layout",r.Layout(tree,allParts.Exists(v=>((Dictionary<string,object>)v)["name"].ToString().StartsWith("german_shepherd_",StringComparison.Ordinal))?4+countNodes*8:0x100,1),"rotor_parts",rotorParts,"parts",allParts));
            }
        }
        var output=Obj("schema_version",1,"read_only",true,
            "client_sha256","e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b",
            "connection_state",r.Word(0x01a2e7ec),"map",r.Name(r.Word(0x02dfb000)),
            "ncs_crc",r.Word(0x02df7d90),"weapons",weapons,"effects",effects,"sounds",sounds,"models",models,"vehicles",vehicles,"vehicle_tags",vehicleTags,"vehicle_animtrees",vehicleTrees,"optional_assets",optional,"ace_tables",aceTables,"remote_vision_rawfiles",remoteVision,"remote_material_assets",remoteMaterials,"actor_animlist_mask",actorMask,"actor_trees",actorTrees,"actor_rawfiles",actorRawfiles);
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
            frames.Add(Obj("time",r.Word(cg+0xae4c8),"pm_type",r.Word(cg+4),"health",r.Word(cg+0x190),"max_health",r.Word(cg+0x198),
                "player_flags",r.Word(cg+0x10),"ability_mask_0",r.Word(cg+0x4cc),
                "progress",r.Word(cg+0x64),"ids",ids,"targets",targets,"action_types",types,"action_weapons",weapons,
                "location_selection",r.Word(cg+0x234),"location_cursor",Floats(r.Read(cg+0xc6f9c,12),0,3),
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
