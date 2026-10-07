// Read-only extraction of fixed profile/table allowlists and net-constant resources.
// Asset hash lookup: target VA 0x49e170/0x49e370; no calls inside the game.
using System;
using System.Collections.Generic;
using System.Text;
using System.Web.Script.Serialization;

class ClientAssets {
    readonly IntPtr process;
    readonly Dictionary<uint,byte[]> strings=new Dictionary<uint,byte[]>();
    static uint U(byte[] b,int at) { return BitConverter.ToUInt32(b,at); }
    byte[] Read(uint at,int count) { return ReadClientState.ReadBytes(process,at,count); }
    uint Word(uint at) { return U(Read(at,4),0); }
    ClientAssets(IntPtr p) { process=p; }
    static Dictionary<string,object> Obj(params object[] pairs) {
        var o=new Dictionary<string,object>();for(int i=0;i<pairs.Length;i+=2)o[(string)pairs[i]]=pairs[i+1];return o;
    }
    static int Count(uint n,int maximum) { if(n>maximum)throw new Exception("asset count exceeds limit");return (int)n; }
    byte[] StringBytes(uint at) {
        if(at==0)return new byte[0];byte[] cached;
        if(strings.TryGetValue(at,out cached))return cached;
        var bytes=new List<byte>();
        while(bytes.Count<4096) {
            byte[] block=Read(checked(at+(uint)bytes.Count),64);
            for(int i=0;i<block.Length;i++) {
                if(block[i]==0) {cached=bytes.ToArray();strings.Add(at,cached);return cached;}
                bytes.Add(block[i]);
            }
        }
        throw new Exception("unterminated asset string");
    }
    string Name(uint at) {
        if(at==0)return null;var b=StringBytes(at);
        if(b.Length>255)throw new Exception("asset identifier too long");
        foreach(byte c in b)if(c<32||c>126)throw new Exception("non-ASCII asset identifier; check layout");
        return Encoding.ASCII.GetString(b);
    }
    uint Find(uint kind,string name,out uint entryAddress) {
        uint hash=kind;foreach(char c in name.ToLowerInvariant())hash=unchecked(hash*31+(c=='\\'?47u:c));
        uint index=Word(0x01bc9f20+(hash%80000)*4);
        var seen=new HashSet<uint>();
        while(index!=0) {
            if(index>=100000 || !seen.Add(index)||seen.Count>128)throw new Exception("invalid asset hash chain");
            uint entry=checked(0x01fc4e28+index*28);byte[] raw=Read(entry,28);
            if(U(raw,0)==kind) {
                uint header=U(raw,8);
                if(String.Equals(Name(Word(header)),name,StringComparison.OrdinalIgnoreCase)) {
                    if((raw[17]&5)==0)throw new Exception("asset is not loaded");
                    entryAddress=entry;return header;
                }
            }
            index=U(raw,20);
        }
        entryAddress=0;return 0;
    }
    object Definition(uint at) {
        byte[] d=Read(at,60);
        int enumCount=Count(U(d,8),4096),structCount=Count(U(d,16),4096),iaCount=Count(U(d,24),4096),eaCount=Count(U(d,32),4096);
        int leaves=Count(U(d,52),65536);if(U(d,48)>32764)throw new Exception("profile definition is larger than target buffer");
        var enums=new List<object>();
        for(int i=0;i<enumCount;i++) {
            byte[] e=Read(checked(U(d,12)+(uint)i*12),12);int count=Count(U(e,0),32768);var entries=new List<object>();
            byte[] values=count==0?new byte[0]:Read(U(e,8),checked(count*8));
            for(int j=0;j<count;j++)entries.Add(Obj("name",Name(U(values,j*8)),"value",BitConverter.ToUInt16(values,j*8+4)));
            enums.Add(Obj("reserved",U(e,4),"entries",entries));
        }
        var structs=new List<object>();
        for(int i=0;i<structCount;i++) {
            byte[] s=Read(checked(U(d,20)+(uint)i*20),20);int count=Count(U(s,0),8192);var props=new List<object>();
            byte[] values=count==0?new byte[0]:Read(U(s,4),checked(count*24));
            for(int j=0;j<count;j++) {
                int o=j*24;props.Add(Obj("name",Name(U(values,o)),"type",U(values,o+4),"index",U(values,o+8),"leaf",U(values,o+12),"extra4",U(values,o+16),"extra5",U(values,o+20)));
            }
            structs.Add(Obj("size",U(s,8),"bitOffset",U(s,12),"leafCount",U(s,16),"properties",props));
        }
        var indexed=new List<object>();var enumed=new List<object>();
        for(int i=0;i<iaCount;i++) {
            byte[] a=Read(checked(U(d,28)+(uint)i*16),16);indexed.Add(Obj("count",U(a,0),"type",U(a,4),"index",U(a,8),"size",U(a,12)));
        }
        for(int i=0;i<eaCount;i++) {
            byte[] a=Read(checked(U(d,36)+(uint)i*16),16);enumed.Add(Obj("enum",U(a,0),"type",U(a,4),"index",U(a,8),"size",U(a,12)));
        }
        var offsets=new List<uint>();byte[] table=leaves==0?new byte[0]:Read(U(d,56),checked(leaves*4));
        for(int i=0;i<leaves;i++)offsets.Add(U(table,i*4));
        return Obj("address",at,"version",U(d,0),"formatChecksum",U(d,4),"rootType",U(d,40),"rootIndex",U(d,44),"size",U(d,48),"leafCount",leaves,"enums",enums,"structs",structs,"indexedArrays",indexed,"enumedArrays",enumed,"offsets",offsets);
    }
    object Schema(string name) {
        uint entry;uint at=Find(39,name,out entry);if(at==0)return Obj("found",false);
        byte[] s=Read(at,12);int count=Count(U(s,4),64);var defs=new List<object>();
        for(int i=0;i<count;i++)defs.Add(Definition(checked(U(s,8)+(uint)i*60)));
        return Obj("found",true,"entry",entry,"address",at,"name",Name(U(s,0)),"definitions",defs);
    }
    object Table(string name) {
        uint entry;uint at=Find(37,name,out entry);if(at==0)return Obj("found",false);
        byte[] t=Read(at,16);int cols=Count(U(t,4),256),rows=Count(U(t,8),30000);
        if(cols<1||rows<1||cols*(long)rows>200000)throw new Exception("string table dimensions outside budget");
        // CODOL StringTable cells contain a string pointer and hash (8 bytes).
        byte[] values=Read(U(t,12),checked(cols*rows*8));var cells=new List<string>();
        for(int i=0;i<cols*rows;i++)cells.Add(Convert.ToBase64String(StringBytes(U(values,i*8))));
        return Obj("found",true,"entry",entry,"address",at,"name",Name(U(t,0)),"columns",cols,"rows",rows,"cells_base64",cells);
    }
    object MarketplaceHeader() {
        // Target type-name table at 0xce0ea0 assigns marketplacedata type 45.
        // VA 0x70517e loads it under the same name as the separate StringTable.
        uint entry;uint at=Find(45,"mp/coMarketplaceItems.csv",out entry);
        if(at==0)return Obj("found",false);
        return Obj("found",true,"entry",entry,"address",at,"header_base64",Convert.ToBase64String(Read(at,128)));
    }
    object RuntimeSkus() {
        // Fixed, read-only runtime tables from 0x5840a0/0x7072f0/0x4a6550.
        // Undo field obfuscation locally; never export the process's mask values.
        uint skuBase=Word(0x02dbb288);int skuCount=Count(Word(0x02dbb28c),30000);
        if(skuCount<1)throw new Exception("runtime SKU pool not initialized");
        byte[] skuBytes=Read(skuBase,checked(skuCount*24));
        uint[] keys=new uint[9];for(int j=0;j<9;j++)keys[j]=Word(0x00cdd57c+(uint)j*4);
        var skus=new List<object>();
        for(int i=0;i<skuCount;i++) {
            uint at=checked(skuBase+(uint)i*24);var fields=new List<uint>();
            for(int j=0;j<7;j++) {
                uint v=(uint)BitConverter.ToUInt16(skuBytes,i*24+j*2)^((at+(uint)j*2)&65535)^(keys[j]&65535);
                fields.Add(((v<<4)|(v>>12))&65535);
            }
            for(int j=0;j<2;j++) {
                uint v=U(skuBytes,i*24+16+j*4)^(at+16+(uint)j*4)^keys[7+j];
                fields.Add((v<<8)|(v>>24));
            }
            skus.Add(Obj("index",i,"fields",fields,"byte14",skuBytes[i*24+14],"byte15",skuBytes[i*24+15]));
        }
        uint productBase=Word(0x02dbb270);int productCount=Count(Word(0x02dbb274),30000);
        byte[] products=Read(productBase,checked(productCount*24));var rows=new List<object>();
        for(int i=0;i<productCount;i++) {
            int n=products[i*24+10];var items=new List<int>();
            if(n>0) {
                byte[] ids=Read(U(products,i*24+12),n*2);
                for(int j=0;j<n;j++)items.Add(BitConverter.ToUInt16(ids,j*2));
            }
            rows.Add(Obj("index",i,"header_base64",Convert.ToBase64String(products,i*24,24),"item_ids",items));
        }
        return Obj("skus",skus,"products",rows);
    }
    public static void Export(IntPtr process) {
        var reader=new ClientAssets(process);var schemas=new Dictionary<string,object>();var tables=new Dictionary<string,object>();
        foreach(string name in new string[]{"mp/playerdata.def","mp/playersecuredata.def"}) {
            try{schemas[name]=reader.Schema(name);}catch(Exception e){schemas[name]=Obj("error",e.Message);}
        }
        foreach(string name in new string[]{"mp/coMarketplaceItems.csv","mp/coCharacterDefTable.csv","mp/cogroupdefs.csv","mp/coWeapSlotDefs.csv","mp/coCharItemStatsTable.csv","mp/coCharItems/cha.csv","mp/uiNotifications.csv","mp/functionalUnlockTable.csv","mp/unlockFlags.csv","mp/rankTable.csv",
            "mp/coWeapItems/ast.csv","mp/coWeapItems/pst.csv","mp/coWeapItems/cam.csv","mp/coWeapItems/oth.csv","mp/coWeapItems/ret.csv","mp/coItemDependency.csv",
            "mp/coWeaponPrimaryRankTable.csv","mp/coWeaponSecondaryRankTable.csv"}) {
            try{tables[name]=reader.Table(name);}catch(Exception e){tables[name]=Obj("error",e.Message);}
        }
        object market;try{market=reader.MarketplaceHeader();}catch(Exception e){market=Obj("error",e.Message);}
        object runtime;try{runtime=reader.RuntimeSkus();}catch(Exception e){runtime=Obj("error",e.Message);}
        var output=Obj("schema_version",1,"read_only",true,"client_sha256","e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b","schemas",schemas,"tables",tables,"marketplace",market,"runtime_marketplace",runtime);
        var serializer=new JavaScriptSerializer();serializer.MaxJsonLength=32*1024*1024;
        Console.WriteLine(serializer.Serialize(output));
    }
    public static void ExportProfiles(IntPtr process) {
        // Current player singleton returned by VA 0x6da140. Consumer paths
        // address public/private data at +0x90/+0x91b8, following their CRCs.
        var r=new ClientAssets(process);
        var output=Obj("schema_version",1,"read_only",true,
            "client_sha256","e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b",
            "user_id",BitConverter.ToUInt64(r.Read(0x0c95bd80,8),0).ToString(),
            "public_snapshot",Convert.ToBase64String(r.Read(0x0c95be0c,32768)),
            "private_snapshot",Convert.ToBase64String(r.Read(0x0c964f34,32768)));
        Console.WriteLine(new JavaScriptSerializer().Serialize(output));
    }

    object NetConstant(uint at) {
        // NetConstStrings type 44: name, category, source, count, string**.
        byte[] h=Read(at,20); int count=Count(U(h,12),8192);
        string name=Name(U(h,0));
        if(U(h,4)==21 && U(h,8)==4 && count==0 && name=="void")
            return Obj("name",name,"category",21,"source",4,"count",0,"placeholder",true,"strings_base64",new string[0]);
        if(U(h,4)>20) throw new Exception("net-constant category out of range: "+name+" category="+U(h,4)+" source="+U(h,8));
        var values=new List<string>();
        byte[] ptrs=count==0?new byte[0]:Read(U(h,16),count*4);
        for(int i=0;i<count;i++) values.Add(Convert.ToBase64String(StringBytes(U(ptrs,i*4))));
        return Obj("name",Name(U(h,0)),"category",U(h,4),"source",U(h,8),"count",count,"strings_base64",values);
    }
    public static void ExportNetConstants(IntPtr process) {
        var r=new ClientAssets(process);var assets=new List<object>();var linked=new List<object>();
        // Only type-44 resources reachable from current asset hash buckets.
        // Do not filter by entry flags: the default/level and common records differ.
        byte[] entries=r.Read(0x01fc4e28,100000*28),heads=r.Read(0x01bc9f20,80000*4);
        var visited=new HashSet<uint>();
        for(int bucket=0;bucket<80000;bucket++) {
            uint index=U(heads,bucket*4);int length=0;
            while(index!=0) {
                if(index>=100000||!visited.Add(index)||++length>128)throw new Exception("invalid asset bucket chain");
                int at=(int)index*28;
                if(U(entries,at)==44) {
                    if(assets.Count>=512)throw new Exception("net-constant asset budget");
                    try { assets.Add(r.NetConstant(U(entries,at+8))); }
                    catch(Exception e) { assets.Add(Obj("error",e.Message)); }
                }
                index=U(entries,at+20);
            }
        }
        var seen=new HashSet<uint>();
        for(uint category=0;category<21;category++) {
            uint node=r.Word(0x02df7ce8+category*8);
            while(node!=0) {
                if(!seen.Add(node)||seen.Count>512)throw new Exception("invalid net-constant linked list");
                byte[] item=r.Read(node,8); linked.Add(r.NetConstant(U(item,0)));node=U(item,4);
            }
        }
        var networkDvars=new List<string>();
        int networkCount=Count(r.Word(0x02dfaba4),266);
        byte[] networkPointers=r.Read(0x02dfaba8,networkCount*4);
        for(int i=0;i<networkCount;i++)
            networkDvars.Add(Convert.ToBase64String(r.StringBytes(r.Word(U(networkPointers,i*4)))));
        uint language=r.Word(checked(r.Word(0x0315fbec)+12));
        if(language>14)language=0;
        string localizedPrefix=r.Name(r.Word(0x00cdbc1c+language*12))+"_";
        var output=Obj("schema_version",1,"read_only",true,
            "client_sha256","e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b",
            "connection_state",r.Word(0x01a2e7ec),"initialized",r.Word(0x02df9934),
            "checksum",r.Word(0x02df7d90),"common_checksum",r.Word(0x02df9d38),
            "category_layout_base64",Convert.ToBase64String(r.Read(0x00c57118,21*12)),
            "index_types_base64",Convert.ToBase64String(r.Read(0x02df7da0,7058)),
            "network_dvars_base64",networkDvars,"localized_prefix",localizedPrefix,
            "assets",assets,"linked",linked);
        var serializer=new JavaScriptSerializer();serializer.MaxJsonLength=32*1024*1024;
        Console.WriteLine(serializer.Serialize(output));
    }

    public static void ExportMapEntities(IntPtr process) {
        var r=new ClientAssets(process);var maps=new List<object>();
        byte[] entries=r.Read(0x01fc4e28,100000*28),heads=r.Read(0x01bc9f20,80000*4);
        var seen=new HashSet<uint>();
        for(int bucket=0;bucket<80000;bucket++) {
            uint index=U(heads,bucket*4);int length=0;
            while(index!=0) {
                if(index>=100000||!seen.Add(index)||++length>128)throw new Exception("invalid asset chain");
                int offset=(int)index*28;
                if(U(entries,offset)==19) {
                    if(maps.Count>=16)throw new Exception("map resource budget");
                    byte[] h=r.Read(U(entries,offset+8),12);string name=r.Name(U(h,0));
                    int count=Count(U(h,8),4*1024*1024);
                    byte[] text=count==0?new byte[0]:r.Read(U(h,4),count);
                    maps.Add(Obj("name",name,"byte_count",count,"entity_string_base64",Convert.ToBase64String(text)));
                }
                index=U(entries,offset+20);
            }
        }
        var output=Obj("schema_version",1,"read_only",true,
            "client_sha256","e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b",
            "connection_state",r.Word(0x01a2e7ec),"maps",maps);
        var serializer=new JavaScriptSerializer();serializer.MaxJsonLength=16*1024*1024;
        Console.WriteLine(serializer.Serialize(output));
    }
}
