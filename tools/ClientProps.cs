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
    public static void ExportPropsState(IntPtr process,bool once=false) {
        var r=new ClientAssets(process);var frames=new List<object>();
        var clock=System.Diagnostics.Stopwatch.StartNew();var aliasNames=new Dictionary<uint,string>();
        for(int frame=0;frame<(once?1:400)&&(once||clock.ElapsedMilliseconds<20000);frame++) {
            if(r.Word(0x01a2e7ec)!=10)break;
            uint cgBank=r.Word(0x07817ba8),entBank=r.Word(0x07817bb0);if(cgBank>=256||entBank>=256)throw new Exception("invalid prop state bank");
            uint cg=r.Word(0x01a19510+cgBank*4)^0xfbb2c97a,ents=r.Word(0x01a19510+entBank*4)^0x2168f672;
            var props=new List<object>();
            for(uint id=256;id<464;id++) {
                uint ent=ents+id*0x238;uint flags=r.Word(ent+0x204),type=r.Word(ent+0xf4);if(type!=0&&type!=11)continue;
                uint model=r.Word(ent+0xf0+0x98);if(model==0)continue;
                uint es=ent+0xf0;
                props.Add(Obj("entity",id,"type",type,"flags",flags,"model",model,"origin",r.EntityOrigin(es),"angles",Floats(r.Read(es+0x3c,12),0,3),"weapon",r.Word(es+0xd0),"event_sequence",r.Word(es+0xac),"event_ids",new uint[]{r.Word(es+0xb0),r.Word(es+0xb8),r.Word(es+0xc0),r.Word(es+0xc8)},"dobj_slot",BitConverter.ToUInt16(r.Read(0x02ef4858+id*2,2),0)));
            }
            var missiles=new List<object>();
            for(uint id=1648;id<1664;id++) {
                uint ent=ents+id*0x238,es=ent+0xf0,flags=r.Word(ent+0x204);
                if((flags&1)==0||r.Word(es+4)!=4)continue;
                missiles.Add(Obj("entity",id,"type",r.Word(es+4),"flags",flags,"origin",r.EntityOrigin(es),"angles",Floats(r.Read(es+0x3c,12),0,3),"weapon",r.Word(es+0xd0),"owner",r.Word(es+0x80),"dobj_slot",BitConverter.ToUInt16(r.Read(0x02ef4858+id*2,2),0)));
            }
            var sounds=new List<object>();byte[] channels=r.Read(0x07810d58,53*0xa8);
            for(int channel=0;channel<53;channel++) {
                int at=channel*0xa8;uint source=U(channels,at+0xc),alias=U(channels,at+0x68);
                if(alias<0x10000)continue;
                string name;if(!aliasNames.TryGetValue(alias,out name)) {
                    try{name=r.Name(r.Word(alias));}catch{continue;}
                    aliasNames[alias]=name;
                }
                if(!((source&0x7ff)>=384&&(source&0x7ff)<448)&&name!="sentry_minigun_fire"&&name!="sentry_minigun_cooldown")continue;
                sounds.Add(Obj("slot",channel,"source",source,"alias",name,"handle",U(channels,at+0x20),"active_byte",channels[at+0x94]));
            }
            uint radarGetter=(cg+0x228)^r.Word(cg+0x22c)^r.Word(0x00ce5188);
            frames.Add(Obj("time",r.Word(cg+0xae4c8),"health",r.Word(cg+0x190),"origin",Floats(r.Read(cg+0x1c,12),0,3),"yaw",Floats(r.Read(cg+0x150,4),0,1)[0],"pitch",Floats(r.Read(cg+0x14c,4),0,1)[0],"props",props,"missiles",missiles,"radar_flags",r.RadarFlags(cg+0x218,radarGetter),"effects",r.EffectStates(),"device_sounds",sounds));
            System.Threading.Thread.Sleep(50);
        }
        var s=new JavaScriptSerializer();s.MaxJsonLength=16*1024*1024;Console.WriteLine(s.Serialize(Obj("schema_version",1,"read_only",true,"frames",frames)));
    }

}
