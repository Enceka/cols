// Read-only map prop model bounds, registrations and effect resources.
using System;
using System.Collections.Generic;
using System.Web.Script.Serialization;

partial class ClientAssets {
    public static void ExportPropsAssets(IntPtr process) {
        var r=new ClientAssets(process);
        if(r.Word(0x01a2e7ec)!=10||r.Word(0x02df9934)!=1)throw new Exception("initialize the map before prop capture");
        uint slot=r.Word(0x07817bac);if(slot>=256)throw new Exception("invalid cgs bank");
        uint cgs=r.Word(0x01a19510+slot*4)^0xce9af8cf;
        byte[] registered=r.Read(cgs+0xf0,1024*4);var models=new List<object>();
        for(uint id=1;id<1024;id++) {
            uint model=U(registered,(int)id*4);if(model==0)continue;
            string name=r.Name(r.Word(model));
            if(!name.StartsWith("vehicle_",StringComparison.Ordinal)&&!name.StartsWith("veh_",StringComparison.Ordinal)&&!name.Contains("barrel"))continue;
            byte[] bounds=r.Read(model+0x140,24);float[] center=Floats(bounds,0,3),half=Floats(bounds,12,3);
            float[] min=new float[3],max=new float[3];for(int i=0;i<3;i++){min[i]=center[i]-half[i];max[i]=center[i]+half[i];}
            models.Add(Obj("name",name,"index",id,"bounds_min",min,"bounds_max",max));
        }
        var effects=new List<object>();
        for(uint id=1;id<=399;id++) {
            uint fx=r.Word(0x01a1f9bc+id*4);if(fx==0)continue;string name=r.Name(r.Word(fx));
            if(name.StartsWith("props/barrel",StringComparison.Ordinal)||name.StartsWith("explosions/",StringComparison.Ordinal)&&name.Contains("vehicle")||name.StartsWith("smoke/car_damage_",StringComparison.Ordinal))
                effects.Add(Obj("name",name,"selector",id));
        }
        var weapons=new List<object>();uint count=r.Word(0x07817b88);if(count>=900)throw new Exception("weapon registration budget");
        for(uint id=1;id<=count;id++) {
            uint def=r.Word(0x017f5a40+id*4);string name=r.Name(r.Word(def));
            if(name=="barrel_mp"||name=="destructible_car_mp"||name=="destructible_car")weapons.Add(Obj("name",name,"index",id,"radius",r.Word(def+0x8d8),"inner_damage",r.Word(def+0x8e0),"outer_damage",r.Word(def+0x8e4)));
        }
        var sounds=new List<object>();
        foreach(string name in new string[]{"explo_metal_rand","car_explode","vehicle_car_exp","fire_metal_large"}) {
            uint entry;uint at=r.Find(11,name,out entry,false);
            sounds.Add(Obj("name",name,"found",at!=0,"loaded",at!=0&&(r.Read(entry,28)[17]&5)!=0));
        }
        Console.WriteLine(new JavaScriptSerializer().Serialize(Obj("schema_version",1,"read_only",true,
            "client_sha256","e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b",
            "map",r.Name(r.Word(0x02dfb000)),"connection_state",r.Word(0x01a2e7ec),"ncs_crc",r.Word(0x02df7d90),"models",models,"effects",effects,"weapons",weapons,"sounds",sounds)));
    }
    public static void ExportControlStream(IntPtr process) {
        var r=new ClientAssets(process);var clock=System.Diagnostics.Stopwatch.StartNew();
        var serializer=new JavaScriptSerializer();
        Console.Error.WriteLine("sample_ready=control-stream");Console.Error.Flush();
        for(int i=0;i<2400&&clock.ElapsedMilliseconds<60000;i++) {
            if(r.Word(0x01a2e7ec)!=10)break;
            uint bank=r.Word(0x07817ba8);if(bank>=256)throw new Exception("invalid control bank");
            uint cg=r.Word(0x01a19510+bank*4)^0xfbb2c97a;
            uint entBank=r.Word(0x07817bb0);if(entBank>=256)throw new Exception("invalid control entity bank");
            uint entities=r.Word(0x01a19510+entBank*4)^0x2168f672;
            uint linked=r.Word(cg+0x1f0),bodyId=linked>=640&&linked<656?linked-16:linked;
            object bodyOrigin=null,bodyAngles=null;
            if(bodyId>=624&&bodyId<640) {
                uint es=entities+bodyId*0x238+0xf0;
                bodyOrigin=r.EntityOrigin(es);bodyAngles=Floats(r.Read(es+0x3c,12),0,3);
            }
            uint controlBase=r.Word(0x01a2e838)^0x6891d1c7,controlNumber=r.Word(controlBase+0x625c);
            var actors=new List<object>();
            for(uint id=0;id<12;id++) {
                uint ci=cg+0x15dea8+id*0x3b8,ent=entities+id*0x238;
                if(r.Word(ci)==0)continue;
                actors.Add(Obj("client",id,"team",r.Word(ci+0x7c),"origin",r.EntityOrigin(ent+0xf0)));
            }
            var animals=new List<object>();
            for(uint id=656;id<688;id++) {
                uint ent=entities+id*0x238;
                if((r.Word(ent+0x204)&1)==0||r.Word(ent+0xf4)!=19)continue;
                animals.Add(Obj("entity",id,"origin",r.EntityOrigin(ent+0xf0)));
            }
            Console.WriteLine(serializer.Serialize(Obj("time",r.Word(cg+0xae4c8),"held_weapon",r.Word(cg+0x340),
                "selected_weapon",r.Word(cg+0xbd00c),"weapon_state",r.Word(cg+0x27c),"progress",r.Word(cg+0x64),
                "input_buttons",r.Word(controlBase+0x365c+(controlNumber&255)*44+4),"carrier_origin",bodyOrigin,"carrier_angles",bodyAngles,
                "server_time",r.Word(controlBase+0x3538),"server_pitch",BitConverter.ToSingle(r.Read(controlBase+0xc+0x14c,4),0),
                "server_yaw",BitConverter.ToSingle(r.Read(controlBase+0xc+0x150,4),0),"server_weapon",r.Word(controlBase+0xc+0x340),
                "server_health",r.Word(controlBase+0xc+0x190),"server_progress",r.Word(controlBase+0xc+0x64),
                "server_origin",Floats(r.Read(controlBase+0xc+0x1c,12),0,3),"server_view_height",BitConverter.ToSingle(r.Read(controlBase+0xc+0x15c,4),0),"animals",animals,
                "health",r.Word(cg+0x190),"max_health",r.Word(cg+0x198),"view_linked_entity",r.Word(cg+0x1f0),
                "link_flags",r.Word(cg+0x10),"pm_flags",r.Word(cg+0xc),"thermal_render_active",r.Read(cg+0x16bfd0,1)[0],
                "render_origin",Floats(r.Read(cg+0xae504,12),0,3),"render_angles",Floats(r.Read(cg+0xb2688,12),0,3),
                "pitch",BitConverter.ToSingle(r.Read(cg+0x14c,4),0),"yaw",BitConverter.ToSingle(r.Read(cg+0x150,4),0),"actors",actors)));
            Console.Out.Flush();System.Threading.Thread.Sleep(25);
        }
    }
    public static void ExportPropsState(IntPtr process,bool once=false) {
        var r=new ClientAssets(process);var frames=new List<object>();
        var clock=System.Diagnostics.Stopwatch.StartNew();var aliasNames=new Dictionary<uint,string>();
        string previousRemotePhase=null;int phaseMarkers=0;
        for(int frame=0;frame<(once?1:400)&&(once||clock.ElapsedMilliseconds<20000);frame++) {
            if(r.Word(0x01a2e7ec)!=10)break;
            uint cgBank=r.Word(0x07817ba8),entBank=r.Word(0x07817bb0);if(cgBank>=256||entBank>=256)throw new Exception("invalid prop state bank");
            uint cg=r.Word(0x01a19510+cgBank*4)^0xfbb2c97a,ents=r.Word(0x01a19510+entBank*4)^0x2168f672;
            var props=new List<object>();
            // Map props, devices, aircraft, airdrop carriers, crates and RC cars.
            for(uint id=256;id<688;id++) {
                uint ent=ents+id*0x238;uint flags=r.Word(ent+0x204),type=r.Word(ent+0xf4);if(type!=0&&type!=11&&type!=12&&type!=13&&type!=19)continue;
                uint model=type==19?r.Word(ent+0xf0+0x58)>>16:r.Word(ent+0xf0+(type==13?0xe0u:0x98u));if(model==0)continue;
                uint es=ent+0xf0;
                props.Add(Obj("entity",id,"type",type,"flags",flags,"model",model,"dobj_models",type==19?r.RewardDObjModels(id):null,"origin",r.EntityOrigin(es),"angles",Floats(r.Read(es+0x3c,12),0,3),"weapon",r.Word(es+0xd0),"vehicle_definition",type==13?(object)(r.Word(es+0x58)>>4&31):null,"eflags",r.Word(es+8),"gun_angles",type==13?Floats(r.Read(es+0x6c,8),0,2):type==12?Floats(r.Read(es+0x58,12),0,3):null,"aircraft_animation",type==12||type==13||type==19?r.AircraftAnimation(ent,BitConverter.ToUInt16(r.Read(0x02ef4858+id*2,2),0)):null,"aircraft_pose",type==12||type==13||type==19?r.AircraftPose(ent,BitConverter.ToUInt16(r.Read(0x02ef4858+id*2,2),0)):null,"event_sequence",r.Word(es+0xac),"event_ids",new uint[]{r.Word(es+0xb0),r.Word(es+0xb8),r.Word(es+0xc0),r.Word(es+0xc8)},"dobj_slot",BitConverter.ToUInt16(r.Read(0x02ef4858+id*2,2),0)));
            }
            var missiles=new List<object>();
            for(uint id=1600;id<1896;id++) {
                // Bomb/SAM bank and distinct remote payload bank; spatial
                // sounds and hand grenades between them are unrelated.
                if(id==1664)id=1800;
                uint ent=ents+id*0x238,es=ent+0xf0,flags=r.Word(ent+0x204);
                if((flags&1)==0||r.Word(es+4)!=4)continue;
                missiles.Add(Obj("entity",id,"type",r.Word(es+4),"flags",flags,"origin",r.EntityOrigin(es),"angles",Floats(r.Read(es+0x3c,12),0,3),"weapon",r.Word(es+0xd0),"owner",r.Word(es+0x80),"event_sequence",r.Word(es+0xac),"event_ids",new uint[]{r.Word(es+0xb0),r.Word(es+0xb8),r.Word(es+0xc0),r.Word(es+0xc8)},"dobj_slot",BitConverter.ToUInt16(r.Read(0x02ef4858+id*2,2),0)));
            }
            var sounds=new List<object>();byte[] channels=r.Read(0x07810d58,53*0xa8);
            for(int channel=0;channel<53;channel++) {
                int at=channel*0xa8;uint source=U(channels,at+0xc),alias=U(channels,at+0x68);
                if(alias<0x10000)continue;
                string name;if(!aliasNames.TryGetValue(alias,out name)) {
                    try{name=r.Name(r.Word(alias));}catch{continue;}
                    aliasNames[alias]=name;
                }
                if(!((source&0x7ff)>=384&&(source&0x7ff)<448)&&!((source&0x7ff)>=560&&(source&0x7ff)<656)&&!((source&0x7ff)>=1800&&(source&0x7ff)<1896)&&(source&0x7ff)!=2046&&name!="sentry_minigun_fire"&&name!="sentry_minigun_cooldown")continue;
                sounds.Add(Obj("slot",channel,"source",source,"alias",name,"handle",U(channels,at+0x20),"active_byte",channels[at+0x94]));
            }
            var exitHud=new List<object>();
            for(uint slot=0;slot<31;slot++) {
                uint hud=cg+0x1ed0+slot*168,type=r.Word(hud);if(type==0)break;
                uint material=r.Word(hud+76);if(type!=4||(material!=1000&&material!=1001))continue;
                exitHud.Add(Obj("slot",slot,"type",type,"material",material,"width",r.Word(hud+68),"height",r.Word(hud+72),"align_screen",r.Word(hud+44),"color",r.Word(hud+48),"sort",Floats(r.Read(hud+136,4),0,1)[0]));
            }
            byte[] visionName=r.Read(cg+0x16b428,64);int visionEnd=Array.IndexOf(visionName,(byte)0);
            if(visionEnd<0)throw new Exception("unterminated missile vision name");
            string missileVision=System.Text.Encoding.ASCII.GetString(visionName,0,visionEnd);
            uint phaseFlags=r.Word(cg+0x10);
            string remotePhase=exitHud.Count!=0?"static":(phaseFlags&2)!=0?"linked":"body";
            if(remotePhase!=previousRemotePhase&&phaseMarkers<20) {
                var marker=new JavaScriptSerializer();
                Console.Error.WriteLine("CODOL_REMOTE_PHASE "+marker.Serialize(Obj("phase",remotePhase,"time",r.Word(cg+0xae4c8),"camera_entity",r.Word(cg+0x54),"link_flags",phaseFlags,"vision",missileVision)));
                Console.Error.Flush();previousRemotePhase=remotePhase;phaseMarkers++;
            }
            uint radarGetter=(cg+0x228)^r.Word(cg+0x22c)^r.Word(0x00ce5188);
            uint controlBase=r.Word(0x01a2e838)^0x6891d1c7,controlNumber=r.Word(controlBase+0x625c);
            string controlCommand=Convert.ToBase64String(r.Read(controlBase+0x365c+(controlNumber&255)*44,44));
            byte[] gunnerPs=r.Read(cg,0xa78);
            var gunnerAmmo=new Dictionary<string,object>();
            foreach(uint gun in new uint[]{187,188,289,290,291,304})gunnerAmmo[gun.ToString()]=r.ClipCount(gunnerPs,gun);
            frames.Add(Obj("gunner_inventory",Array.ConvertAll(new int[]{0,1,2,3,4,5,6,7,8,9,10,11,12,13,14},i=>r.Word(cg+0x2a4+(uint)i*4)),"naked_vision_name",r.Name(cg+0x16b3a8),"naked_vision_state",r.Word(cg+0x16b338),"thermal_vision_name",r.Name(cg+0x16b468),"thermal_vision_state",r.Word(cg+0x16b374),"thermal_render_active",r.Read(cg+0x16bfd0,1)[0],"view_linked_entity",r.Word(cg+0x1f0),"view_linked_flags",r.Word(cg+0x14),"gunner_clip_counts",gunnerAmmo,"weapon_state",r.Word(cg+0x27c),"weapon_time",r.Word(cg+0x270),"control_command_number",controlNumber,"control_command_base64",controlCommand,"archived_exit_hud",exitHud,"missile_vision_name",missileVision,"missile_vision_state",r.Word(cg+0x16b360),"time",r.Word(cg+0xae4c8),"pm_flags",r.Word(cg+0xc),"link_flags",r.Word(cg+0x10),"camera_entity",r.Word(cg+0x54),"camera_tag",r.Word(cg+0x58),"controls_entity",r.Word(cg+0x5c),"held_weapon",r.Word(cg+0x340),"render_origin",Floats(r.Read(cg+0xae504,12),0,3),"render_angles",Floats(r.Read(cg+0xb2688,12),0,3),"health",r.Word(cg+0x190),"max_health",r.Word(cg+0x198),"local_dobj_models",r.RewardDObjModels(0),"origin",Floats(r.Read(cg+0x1c,12),0,3),"yaw",Floats(r.Read(cg+0x150,4),0,1)[0],"pitch",Floats(r.Read(cg+0x14c,4),0,1)[0],"props",props,"missiles",missiles,"radar_flags",r.RadarFlags(cg+0x218,radarGetter),"effects",r.EffectStates(),"event_links",r.EventLinks(),"device_sounds",sounds));
            System.Threading.Thread.Sleep(50);
        }
        var s=new JavaScriptSerializer();s.MaxJsonLength=16*1024*1024;Console.WriteLine(s.Serialize(Obj("schema_version",1,"read_only",true,"frames",frames)));
    }
    object RewardDObjModels(uint entity) {
        try {
            uint slot=BitConverter.ToUInt16(Read(0x02ef4858+entity*2,2),0);
            var names=new List<string>();
            if(slot>0&&slot<2048) {
                uint obj=0x02e8e850+slot*0xcc;int count=Count(Read(obj+9,1)[0],32);uint list=Word(obj+0xc0);
                for(uint i=0;i<count;i++)names.Add(Name(Word(Word(list+i*4))));
            }
            return names;
        } catch(Exception e){return Obj("error",e.Message);}
    }

}
