// Read-only loaded story asset extraction. Raw bytes remain in local captures.
using System;
using System.Collections.Generic;
using System.Web.Script.Serialization;
using System.IO;

partial class ClientAssets {
    public static void ExportStoryBundle(IntPtr process) {
        var r=new ClientAssets(process);uint state=r.Word(0x01a2e7ec);
        if(state<9||state>10||r.Word(0x02df9934)!=1)throw new Exception("native cargo resources are not initialized");
        var serializer=new JavaScriptSerializer();serializer.MaxJsonLength=64*1024*1024;
        var captures=new Dictionary<string,object>();
        var methods=new Dictionary<string,Action<IntPtr>> {
            {"map-entities",ExportMapEntities},{"net-constants",ExportNetConstants},
            {"story-assets",ExportStoryAssets},{"collision",ExportCollision},
            {"arsenal",ExportArsenal},{"characters",ExportCharacters}
        };
        TextWriter output=Console.Out;
        foreach(var item in methods) {
            var buffer=new StringWriter();
            try {
                Console.SetOut(buffer);item.Value(process);
                captures[item.Key]=serializer.DeserializeObject(buffer.ToString());
            }catch(Exception e){captures[item.Key]=Obj("error",e.Message);}
            finally{Console.SetOut(output);buffer.Dispose();}
        }
        Console.WriteLine(serializer.Serialize(Obj("schema_version",1,"read_only",true,
            "client_sha256","e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b",
            "connection_state",state,"final_connection_state",r.Word(0x01a2e7ec),"captures",captures)));
    }
    public static void ExportStoryAssets(IntPtr process) {
        var r=new ClientAssets(process);
        var names=new List<object>();var rawfiles=new Dictionary<string,object>();
        var tables=new Dictionary<string,object>();
        foreach(string name in new string[]{"mp/coMapList.csv","mp/coGameTypesTable.csv","mp/unlockFlags.csv"})
            try{tables[name]=r.Table(name);}catch(Exception e){tables[name]=Obj("error",e.Message);}
        byte[] entries=r.Read(0x01fc4e28,100000*28),heads=r.Read(0x01bc9f20,80000*4);
        var seen=new HashSet<uint>();int total=0;
        for(int bucket=0;bucket<80000;bucket++) {
            uint index=U(heads,bucket*4);int length=0;
            while(index!=0) {
                if(index>=100000||!seen.Add(index)||++length>128)throw new Exception("invalid story asset chain");
                int at=(int)index*28;uint kind=U(entries,at),header=U(entries,at+8);
                if((kind==36||kind==37||kind==19||kind==44)&&(entries[at+17]&5)!=0) {
                    string name=r.Name(r.Word(header));
                    if(names.Count>=8192)throw new Exception("story asset name budget");
                    names.Add(Obj("kind",kind,"name",name));
                    string lower=name.ToLowerInvariant();
                    bool script=lower.EndsWith(".gsc")||lower.EndsWith(".csc")||lower.EndsWith(".script");
                    bool related=lower.Contains("coop")||lower.Contains("story")||lower.Contains("objective")||lower.Contains("mission");
                    if(kind==36&&(script||related)&&!rawfiles.ContainsKey(name)) {
                        try {
                            byte[] raw=r.Read(header,16);int compressed=Count(U(raw,4),2*1024*1024);
                            int size=compressed!=0?compressed:Count(U(raw,8),2*1024*1024);
                            if(total+size>16*1024*1024)throw new Exception("story rawfile byte budget");
                            total+=size;rawfiles[name]=r.RawFile(name,2*1024*1024);
                        }catch(Exception e){rawfiles[name]=Obj("error",e.Message);}
                    }
                    if(kind==37&&related&&!tables.ContainsKey(name))
                        try{tables[name]=r.Table(name);}catch(Exception e){tables[name]=Obj("error",e.Message);}
                }
                index=U(entries,at+20);
            }
        }
        var serializer=new JavaScriptSerializer();serializer.MaxJsonLength=32*1024*1024;
        Console.WriteLine(serializer.Serialize(Obj("schema_version",1,"read_only",true,
            "client_sha256","e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b",
            "connection_state",r.Word(0x01a2e7ec),"ncs_initialized",r.Word(0x02df9934),
            "ncs_crc",r.Word(0x02df7d90),"rawfiles",rawfiles,"tables",tables,"asset_names",names)));
    }
}
