// Fixed 3.3.3.3 voice-resource inventory. ReadProcessMemory only; no playback.
using System;
using System.Collections.Generic;
using System.Web.Script.Serialization;

partial class ClientAssets {
    public static void ExportVoicePlayback(IntPtr process) {
        var r=new ClientAssets(process);var frames=new List<object>();var clock=System.Diagnostics.Stopwatch.StartNew();
        var aliases=new Dictionary<uint,string>();
        uint initialState=r.Word(0x01a2e7ec);
        Console.Error.WriteLine("sample_ready=voice-playback");Console.Error.Flush();
        while(clock.ElapsedMilliseconds<20000) {
            uint state=r.Word(0x01a2e7ec),time=0,entities=0;
            if(state==10) {
                uint cgSlot=r.Word(0x07817ba8),entitySlot=r.Word(0x07817bb0);
                if(cgSlot>=256||entitySlot>=256)throw new Exception("invalid voice context slot");
                uint cg=r.Word(0x01a19510+cgSlot*4)^0xfbb2c97a;time=r.Word(cg+0xae4c8);
                entities=r.Word(0x01a19510+entitySlot*4)^0x2168f672;
            }
            byte[] channels=r.Read(0x07810d58,53*0xa8);var sounds=new List<object>();
            for(int channel=0;channel<53;channel++) {
                int at=channel*0xa8;uint alias=U(channels,at+0x68);
                if(alias<0x10000)continue;
                string name;
                if(!aliases.TryGetValue(alias,out name)) {
                    try{name=r.Name(r.Word(alias));}catch(Exception){continue;}
                    aliases[alias]=name;
                }
                if(!VoiceName(name))continue;
                var fileInfo=new Dictionary<string,object>();
                try {
                    uint file=r.Word(alias+0x14);byte[] header=r.Read(file,12);
                    fileInfo["type"]=header[0];fileInfo["exists"]=header[1];
                    if(header[0]==2||header[0]==3) {
                        fileInfo["directory"]=r.Name(U(header,4));fileInfo["name"]=r.Name(U(header,8));
                    } else if(header[0]==1&&U(header,4)!=0) {
                        fileInfo["name"]=r.Name(r.Word(U(header,4)));
                    }
                } catch(Exception e) {fileInfo["error"]=e.Message;}
                sounds.Add(Obj("slot",channel,"alias",name,"source",U(channels,at+0xc),
                    "handle",U(channels,at+0x20),"status_word",U(channels,at+0x10),
                    "active_byte",channels[at+0x94],"sample_handle",r.Word(0x07813854+(uint)channel*4),
                    "origin",Floats(channels,at+0x74,3),"sound_file",fileInfo,"alias_header_base64",Convert.ToBase64String(r.Read(alias,0x6c))));
            }
            var events=new List<object>();
            if(entities!=0)for(uint id=1668;id<1700;id++) {
                uint entity=entities+id*0x238;
                if((r.Word(entity+0x204)&1)==0)continue;
                events.Add(Obj("entity",id,"type",r.Word(entity+0xf4),"source",r.Word(entity+0xf0+0x80),
                    "selector",r.Word(entity+0xf0+0xa8),"latch",r.Word(entity+0x20c),"origin",r.EntityOrigin(entity+0xf0)));
            }
            frames.Add(Obj("elapsed",clock.ElapsedMilliseconds,"state",state,"time",time,
                "audio_initialized",r.Read(0x0780a488,1)[0],"channels",sounds,"events",events));
            System.Threading.Thread.Sleep(20);
        }
        var serializer=new JavaScriptSerializer();serializer.MaxJsonLength=32*1024*1024;
        Console.WriteLine(serializer.Serialize(Obj("schema_version",1,"read_only",true,
            "client_sha256","e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b",
            "initial_state",initialState,"frames",frames)));
    }
    static bool VoiceName(string name) {
        if(name==null)return false;
        string n=name.ToLowerInvariant();
        return n.StartsWith("uk_")||n.StartsWith("ab_")||n.Contains("1mc_")||
            n.Contains("inform_")||n.Contains("announc")||n.Contains("quickmessage")||
            n.Contains("breath")||n.Contains("mp_sprint")||n.Contains("mp_death")||
            n.Contains("mp_pain")||n.Contains("headshot")||n=="mp_hit_alert"||
            n.Contains("_m1_")||n.Contains("_m2_")||n.Contains("_f1_")||n.Contains("_f2_")||
            n=="ui_mp_timer_countdown";
    }
    public static void ExportVoiceAssets(IntPtr process) {
        var r=new ClientAssets(process);var aliases=new List<object>();var rawfiles=new Dictionary<string,object>();
        var tables=new Dictionary<string,object>();var visited=new HashSet<uint>();
        byte[] entries=r.Read(0x01fc4e28,100000*28),heads=r.Read(0x01bc9f20,80000*4);
        int soundCount=0;
        for(int bucket=0;bucket<80000;bucket++) {
            uint index=U(heads,bucket*4);int length=0;
            while(index!=0) {
                if(index>=100000||!visited.Add(index)||++length>128)throw new Exception("invalid voice asset chain");
                int at=(int)index*28;uint kind=U(entries,at),header=U(entries,at+8);
                if(kind==11) {
                    soundCount++;string name=r.Name(r.Word(header));
                    if(VoiceName(name)) {
                        if(aliases.Count>=8192)throw new Exception("voice alias budget exceeded");
                        var item=Obj("name",name,"entry_flags",entries[at+17],"header_base64",Convert.ToBase64String(r.Read(header,12)));
                        // VA0x62dd90 advances by 0x6c. Retain undecoded fields
                        // as candidate words instead of importing Alpha offsets.
                        try {uint first=r.Word(header+4);int count=Count(r.Word(header+8),256);
                            item["count"]=count;
                            if(count>0&&first!=0)item["first_alias_layout"]=r.Layout(first,0x6c,1);
                        } catch(Exception e) {item["error"]=e.Message;}
                        aliases.Add(item);
                    }
                } else if(kind==36||kind==37) {
                    string name=r.Name(r.Word(header));string n=name==null?"":name.ToLowerInvariant();
                    bool related=n.Contains("battlechatter")||n.Contains("quickmessage")||n.Contains("factiontable")||
                        n.Contains("voice")||n=="mp/cocharacterdeftable.csv"||
                        n=="maps/mp/gametypes/_music_and_dialog.gsc"||
                        n=="maps/mp/gametypes/_gamelogic.gsc"||n=="maps/mp/gametypes/_teams.gsc";
                    if(related) {
                        if(rawfiles.Count+tables.Count>=64)throw new Exception("voice script/table budget exceeded");
                        if(kind==36&&!rawfiles.ContainsKey(name))try{rawfiles[name]=r.AnimationRules(name);}catch(Exception e){rawfiles[name]=Obj("error",e.Message);}
                        // Active hash identities with flags0/2 still have valid
                        // tables and aliases in this build. Capture their data
                        // without treating the flag word as playback evidence.
                        if(kind==37&&!tables.ContainsKey(name))try{tables[name]=r.Table(name,false);}catch(Exception e){tables[name]=Obj("error",e.Message);}
                    }
                }
                index=U(entries,at+20);
            }
        }
        var automatic=new Dictionary<string,object>();
        // Current prefix setter VA0x4555c0, distinct from Alpha's handles.
        string[] automaticNames={"sprint","sniper_breathin","sniper_breathout","sniper_breathgasp"};
        uint[] automaticSlots={0x01a1f7f4,0x01a1bdfc,0x01a1be00,0x01a1be04};
        for(int i=0;i<automaticSlots.Length;i++) {
            uint handle=r.Word(automaticSlots[i]);
            automatic[automaticNames[i]]=handle==0?null:r.Name(r.Word(handle));
        }
        uint state=r.Word(0x01a2e7ec);
        var serializer=new JavaScriptSerializer();serializer.MaxJsonLength=32*1024*1024;
        Console.WriteLine(serializer.Serialize(Obj("schema_version",1,"read_only",true,
            "client_sha256","e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b",
            "connection_state",state,"ncs_crc",r.Word(0x02df7d90),
            "map",state==10?r.Name(r.Word(0x02dfb000)):null,"automatic_voice_handles",automatic,
            "sound_asset_count",soundCount,"aliases",aliases,"rawfiles",rawfiles,"tables",tables)));
    }
}
