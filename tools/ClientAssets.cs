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
                if(String.Equals(Name(Word(header+(kind==5?0x10u:0u))),name,StringComparison.OrdinalIgnoreCase)) {
                    if((raw[17]&5)==0)throw new Exception("asset is not loaded");
                    entryAddress=entry;return header;
                }
            }
            index=U(raw,20);
        }
        entryAddress=0;return 0;
    }
    // Layout-agnostic read of a loaded asset: each pointer word reports a
    // printable string at its target and, to the given depth, nested words.
    string Printable(uint at) {
        byte[] b;try{b=Read(at,64);}catch(Exception){return null;}
        int n=Array.IndexOf(b,(byte)0);if(n<3)return null;
        for(int i=0;i<n;i++)if(b[i]<32||b[i]>126)return null;
        return Encoding.ASCII.GetString(b,0,n);
    }
    object Layout(uint at,int size,int depth) {
        byte[] raw;try{raw=Read(at,size);}catch(Exception e){return Obj("error",e.Message);}
        var words=new List<object>();
        for(int i=0;i+4<=size;i+=4) {
            uint w=U(raw,i);if(w<0x10000||w>=0x7fff0000)continue;
            string text=Printable(w);
            words.Add(text!=null ? Obj("offset",i,"pointer",w,"string",text)
                : depth>0 ? Obj("offset",i,"pointer",w,"target",Layout(w,0x40,depth-1)) : Obj("offset",i,"pointer",w));
        }
        return Obj("hex",BitConverter.ToString(raw).Replace("-",""),"pointers",words);
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
    object AnimationRules(string name) {
        uint entry;uint at=Find(36,name,out entry);if(at==0)return Obj("found",false);
        // Current rawfile type36: name, compressedLen, len, buffer. Export
        // bounded bytes read-only; decode compressed text outside the game.
        byte[] header=Read(at,16);int compressed=Count(U(header,4),2*1024*1024);
        int length=Count(U(header,8),2*1024*1024);int bytes=compressed!=0?compressed:length;
        return Obj("found",true,"name",name,"compressed_length",compressed,"length",length,
            "buffer_base64",Convert.ToBase64String(Read(U(header,12),bytes)));
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
        foreach(string name in new string[]{"mp/coGameTypesTable.csv","mp/coMapList.csv","mp/coMarketplaceItems.csv","mp/coCharacterDefTable.csv","mp/cogroupdefs.csv","mp/coWeapSlotDefs.csv","mp/coCharItemStatsTable.csv","mp/coCharItems/cha.csv","mp/coKnifeStatsTable.csv","mp/uiNotifications.csv","mp/functionalUnlockTable.csv","mp/unlockFlags.csv","mp/rankTable.csv",
            "mp/coPerkStatsTable.csv","mp/coAbilityModifiersTable.csv",
            "mp/coWeapItems/ast.csv","mp/coWeapItems/pst.csv","mp/coWeapItems/cam.csv","mp/coWeapItems/oth.csv","mp/coWeapItems/ret.csv","mp/coItemDependency.csv",
            "mp/coWeaponPrimaryRankTable.csv","mp/coWeaponSecondaryRankTable.csv",
            "mp/callsignTitle.csv","mp/callsignIcon.csv","mp/callsignBackground.csv","mp/callsignNameplate.csv"}) {
            try{tables[name]=reader.Table(name);}catch(Exception e){tables[name]=Obj("error",e.Message);}
        }
        object market;try{market=reader.MarketplaceHeader();}catch(Exception e){market=Obj("error",e.Message);}
        object runtime;try{runtime=reader.RuntimeSkus();}catch(Exception e){runtime=Obj("error",e.Message);}
        var output=Obj("schema_version",1,"read_only",true,"client_sha256","e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b","schemas",schemas,"tables",tables,"marketplace",market,"runtime_marketplace",runtime);
        var serializer=new JavaScriptSerializer();serializer.MaxJsonLength=32*1024*1024;
        Console.WriteLine(serializer.Serialize(output));
    }
    public static void ExportHudAssets(IntPtr process) {
        var r=new ClientAssets(process);var menus=new List<object>();
        byte[] entries=r.Read(0x01fc4e28,100000*28),heads=r.Read(0x01bc9f20,80000*4);var seen=new HashSet<uint>();
        for(int bucket=0;bucket<80000;bucket++) {
            uint index=U(heads,bucket*4);int chain=0;
            while(index!=0) {
                if(index>=100000||!seen.Add(index)||++chain>128)throw new Exception("invalid HUD asset chain");
                int at=(int)index*28;uint kind=U(entries,at),header=U(entries,at+8);
                if(kind==25||kind==26) {
                    if(menus.Count>=4096)throw new Exception("HUD menu budget");
                    string name=r.Name(r.Word(header));
                    bool selected=kind==26&&(name=="perk_sidebar_hud"||name=="perk_gained"||name=="hud_fullscreen"||name=="loadsprite"||name=="splash"||name=="extra_info"||name=="announcer_line"||name=="weaponbar_hd"||name=="weaponbar_hd_extend"||name=="kd_rate_bar"||name=="combat_performance"||name=="scorebar_hd"||name=="exp_bar");
                    menus.Add(Obj("type",kind,"name",name,"header_base64",Convert.ToBase64String(r.Read(header,64)),
                        "full_header_base64",selected?Convert.ToBase64String(r.Read(header,0x194)):null,
                        "visible_expression",selected?r.HudExpression(r.Word(header+0xe8)):null,
                        "items",selected?r.HudItems(header):null,
                        "on_open",kind==26?r.HudEvents(r.Word(header+0xd4)):null));
                }
                index=U(entries,at+20);
            }
        }
        var rawfiles=new Dictionary<string,object>();
        foreach(string name in new string[]{"ui_mp/hud.menu","ui_mp/hud_fullscreen.menu","ui_mp/perks.menu","ui_mp/perk_display.menu"})
            rawfiles[name]=r.AnimationRules(name);
        object perkCache=null;
        if(r.Word(0x01a2e7ec)==10) {
            uint bank=r.Word(0x07817ba8);if(bank>=256)throw new Exception("invalid HUD bank");
            uint cg=r.Word(0x01a19510+bank*4)^0xfbb2c97a;
            perkCache=Obj("native_masks",new uint[]{r.Word(cg+0x4c0),r.Word(cg+0x4c4),r.Word(cg+0x4c8)},
                "seen_masks",new uint[]{r.Word(cg+0x16be0c),r.Word(cg+0x16be10),r.Word(cg+0x16be14)},
                "ids_base64",Convert.ToBase64String(r.Read(cg+0x16be18,128)),"times_base64",Convert.ToBase64String(r.Read(cg+0x16be98,128)),
                "sidebar_gate",r.Word(cg+0x4bc),"other_mode_gate",r.Word(cg+0x3508));
        }
        var serializer=new JavaScriptSerializer();serializer.MaxJsonLength=32*1024*1024;
        Console.WriteLine(serializer.Serialize(Obj("schema_version",1,"read_only",true,
            "client_sha256","e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b",
            "connection_state",r.Word(0x01a2e7ec),"menus",menus,"rawfiles",rawfiles,"perk_cache",perkCache,
            "perk_display_dvar",r.Name(r.Word(r.Word(0x01a2e834))),"perk_display_dvar_value",r.Word(r.Word(0x01a2e834)+12),
            "perk_popup_dvar",r.Name(r.Word(r.Word(0x01a202f0))),"perk_popup_dvar_value",r.Word(r.Word(0x01a202f0)+12))));
    }
    object HudEvents(uint pointer) {
        if(pointer==0)return null;
        try {
            byte[] header=Read(pointer,8);int count=Count(U(header,0),128);byte[] pointers=Read(U(header,4),count*4);var handlers=new List<object>();
            for(int i=0;i<count;i++) {
                byte[] handler=Read(U(pointers,i*4),8);uint kind=U(handler,4),value=U(handler,0);
                handlers.Add(Obj("kind",kind,"header_base64",Convert.ToBase64String(handler),
                    "script_base64",kind==0?Convert.ToBase64String(StringBytes(value)):null));
            }
            return Obj("handlers",handlers);
        } catch(Exception error) {return Obj("error",error.Message);}
    }
    object HudExpression(uint pointer) { return HudExpression(pointer,0); }
    object HudExpression(uint pointer,int depth) {
        if(pointer==0)return null;
        try {
        if(depth>8)throw new Exception("HUD expression depth");
        byte[] header=Read(pointer,8);int count=Count(U(header,0),1024);
        byte[] data=count==0?new byte[0]:Read(U(header,4),count*12);var entries=new List<object>();
        for(int i=0;i<count;i++) {
            uint type=U(data,i*12),kind=U(data,i*12+4),value=U(data,i*12+8);
            entries.Add(Obj("type",type,"kind",kind,"value",value,"string",type==1&&kind==2?Name(value):null,
                "expression",type==1&&kind==3?HudExpression(value,depth+1):null));
        }
        return Obj("entries",entries,"entries_base64",Convert.ToBase64String(data));
        } catch(Exception error) { return Obj("error",error.Message); }
    }
    object HudItems(uint menu) {
        int count=Count(Word(menu+0xb4),512);byte[] pointers=Read(Word(menu+0x120),count*4);var items=new List<object>();
        for(int i=0;i<count;i++) {
            uint pointer=U(pointers,i*4);
            byte[] header=Read(pointer,0x188);
            items.Add(Obj("index",i,"header_base64",Convert.ToBase64String(header),"name",Name(U(header,0)),"text",Name(U(header,0xe8)),
                "alpha_expression",HudExpression(Word(pointer+0x148)),
                "text_expression",HudExpression(Word(pointer+0x150)),"material_expression",HudExpression(Word(pointer+0x158))));
        }
        return items;
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

    public static void ExportTacticalAssets(IntPtr process) {
        var r=new ClientAssets(process);var rawfiles=new Dictionary<string,object>();var tables=new Dictionary<string,object>();
        foreach(string name in new string[]{"shock/flashbang_mp.shock","shock/concussion_grenade_mp.shock","shock/default.shock",
            "shock/damage_mp.shock","shock/frag_grenade_mp.shock","shock/tabun_gas_mp.shock","shock/tabun_gas_nokick_mp.shock","shock/dog_bite.shock","shock/emp_grenade_mp.shock"}) {
            try{rawfiles[name]=r.AnimationRules(name);}catch(Exception e){rawfiles[name]=Obj("error",e.Message);}
        }
        foreach(string name in new string[]{"mp/fxflashstatus.csv","mp/coPerkStatsTable.csv","mp/coAbilityModifiersTable.csv"}) {
            try{tables[name]=r.Table(name);}catch(Exception e){tables[name]=Obj("error",e.Message);}
        }
        // Asset identities contain no profile or connection material. Enumerate
        // only loaded rawfiles and string tables; retain related data losslessly.
        var names=new List<object>();byte[] entries=r.Read(0x01fc4e28,100000*28),heads=r.Read(0x01bc9f20,80000*4);
        var seen=new HashSet<uint>();
        for(int bucket=0;bucket<80000;bucket++) {
            uint index=U(heads,bucket*4);int length=0;
            while(index!=0) {
                if(index>=100000||!seen.Add(index)||++length>128)throw new Exception("invalid tactical asset chain");
                int at=(int)index*28;uint kind=U(entries,at),header=U(entries,at+8);
                if((kind==36||kind==37)&&(entries[at+17]&5)!=0) {
                    string name=r.Name(r.Word(header)),lower=name.ToLowerInvariant();
                    if(names.Count>=8192)throw new Exception("tactical asset name budget");
                    names.Add(Obj("kind",kind,"name",name));
                    bool related=lower.Contains("flash")||lower.Contains("shock")||lower.Contains("concussion")||lower.Contains("stun")||lower.Contains("shell")||lower.Contains("emp");
                    if(related&&kind==36&&!rawfiles.ContainsKey(name)) {
                        try{rawfiles[name]=r.AnimationRules(name);}catch(Exception e){rawfiles[name]=Obj("error",e.Message);}
                    }
                    if(related&&kind==37&&!tables.ContainsKey(name)) {
                        try{tables[name]=r.Table(name);}catch(Exception e){tables[name]=Obj("error",e.Message);}
                    }
                }
                index=U(entries,at+20);
            }
        }
        var shocks=new List<object>();for(uint i=0;i<16;i++)
            shocks.Add(Obj("index",i,"parameters_base64",Convert.ToBase64String(r.Read(0x017e2dc0+i*0x268,0x268))));
        var linked=new List<object>();seen.Clear();uint node=r.Word(0x02df7d20);
        while(node!=0) {
            if(!seen.Add(node)||seen.Count>64)throw new Exception("invalid shock NCS linked list");
            byte[] item=r.Read(node,8);linked.Add(r.NetConstant(U(item,0)));node=U(item,4);
        }
        var serializer=new JavaScriptSerializer();serializer.MaxJsonLength=32*1024*1024;
        Console.WriteLine(serializer.Serialize(Obj("schema_version",1,"read_only",true,
            "client_sha256","e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b",
            "connection_state",r.Word(0x01a2e7ec),"rawfiles",rawfiles,"tables",tables,
            "asset_names",names,"shocks",shocks,"shock_ncs",linked)));
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

    public static void ExportBootstrapWeapon(IntPtr process) {
        var r=new ClientAssets(process);int count=Count(r.Word(0x07817b88),899);
        // Only identify pointers that are actual type-4 asset headers. Do not
        // interpret arbitrary weapon words as pointers or guess model roles.
        var models=new HashSet<uint>();byte[] entries=r.Read(0x01fc4e28,100000*28);
        for(int j=1;j<100000;j++)if(U(entries,j*28)==4)models.Add(U(entries,j*28+8));
        object weapon=null,composed=null;uint baseIndex=0;
        for(uint i=1;i<=count;i++) {
            uint at=r.Word(0x017f5a40+i*4);string name=r.Name(r.Word(at));
            if(name!="m4_mp")continue;
            if(weapon!=null)throw new Exception("duplicate M4 weapon registration");
            var modelFields=new Dictionary<string,object>();
            for(uint offset=0x10;offset<=0x50;offset+=4) {
                uint pointer=r.Word(at+offset);
                if(pointer!=0 && models.Contains(pointer))modelFields["0x"+offset.ToString("x")]=r.Name(r.Word(pointer));
            }
            weapon=Obj("name",name,"index",i,
                "ammo_name",r.Name(r.Word(at+0x6b4)),"ammo_key",r.Word(at+0x6b8),
                "clip_name",r.Name(r.Word(at+0x6bc)),"clip_key",r.Word(at+0x6c0),"model_fields",modelFields,
                "runtime_fields",r.WeaponRuntimeFields(at));
            baseIndex=i;
        }
        if(weapon==null)throw new Exception("M4 not registered; load the training map first");
        string composedName="** "+baseIndex+" ";
        for(uint i=1;i<=count;i++) {
            uint at=r.Word(0x017f5a40+i*4);string name=r.Name(r.Word(at));
            if(!name.StartsWith(composedName,StringComparison.Ordinal))continue;
            if(composed!=null)throw new Exception("duplicate composed M4");
            composed=Obj("name",name,"index",i,"ammo_key",r.Word(at+0x6b8),"clip_key",r.Word(at+0x6c0),
                "runtime_fields",r.WeaponRuntimeFields(at));
        }
        var output=Obj("schema_version",1,"read_only",true,
            "client_sha256","e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b",
            "connection_state",r.Word(0x01a2e7ec),"ncs_crc",r.Word(0x02df7d90),"weapon",weapon,"composed_weapon",composed);
        Console.WriteLine(new JavaScriptSerializer().Serialize(output));
    }
    public static void ExportWeaponRegistry(IntPtr process) {
        var r=new ClientAssets(process);int count=Count(r.Word(0x07817b88),899);
        var weapons=new List<object>();
        for(uint i=1;i<=count;i++) {
            uint def=r.Word(0x017f5a40+i*4);
            if(def==0)throw new Exception("registered weapon pointer missing");
            weapons.Add(Obj("index",i,"name",r.Name(r.Word(def)),
                "type",r.Word(def+0x488),"alternate_index",r.Word(def+0xd24),
                "clip_key",r.Word(def+0x6c0),"model_name",r.Name(r.Word(def+0xc))));
        }
        var offhands=new List<object>();
        for(uint i=1;i<=count;i++) {
            uint def=r.Word(0x017f5a40+i*4);
            if(r.Word(def+0x47c)!=1)continue;
            offhands.Add(Obj("index",i,"name",r.Name(r.Word(def)),
                "definition_base64",Convert.ToBase64String(r.Read(def,0xe28)),
                "explosion_effect",r.EffectDefinition(r.Word(def+0x918))));
        }
        var supply=new List<object>();var modelHeaders=new HashSet<uint>();var soundHeaders=new HashSet<uint>();byte[] supplyEntries=r.Read(0x01fc4e28,100000*28);
        for(int entry=1;entry<100000;entry++) {
            if(U(supplyEntries,entry*28)==4)modelHeaders.Add(U(supplyEntries,entry*28+8));
            if(U(supplyEntries,entry*28)==11)soundHeaders.Add(U(supplyEntries,entry*28+8));
        }
        for(uint i=1;i<=count;i++) {
            uint def=r.Word(0x017f5a40+i*4);string name=r.Name(r.Word(def));
            if(name!="scavenger_bag_mp")continue;
            var models=new List<object>();
            // 0x501320 uses WeaponDef[slot+6], i.e. the world model at+0x18.
            for(uint slot=0;slot<8;slot++) {
                uint model=r.Word(def+0x18+slot*4);if(model!=0&&modelHeaders.Contains(model))
                    models.Add(Obj("slot",slot,"name",r.Name(r.Word(model)),"header_base64",Convert.ToBase64String(r.Read(model,0x16c))));
            }
            var sounds=new List<object>();
            foreach(uint offset in new uint[]{0x4a4,0x4a8,0x4ac,0x4b0}) {
                uint sound=r.Word(def+offset);if(sound!=0&&soundHeaders.Contains(sound))
                    sounds.Add(Obj("offset",offset,"name",r.Name(r.Word(sound)),"header_base64",Convert.ToBase64String(r.Read(sound,12))));
            }
            supply.Add(Obj("index",i,"name",name,"definition_base64",Convert.ToBase64String(r.Read(def,0xe28)),"world_models",models,"pickup_sounds",sounds,
                "touch_constants",new double[]{BitConverter.ToDouble(r.Read(0xc85f68,8),0),BitConverter.ToDouble(r.Read(0xc85f70,8),0),BitConverter.ToDouble(r.Read(0xc85f60,8),0),BitConverter.ToDouble(r.Read(0xc85a68,8),0)}));
        }
        var sources=new List<object>();
        foreach(string name in new string[]{"csg12ss_mp","ak104_mp","m4_mp","m9_mp"}) {
            try {
                uint entry;uint def=r.Find(28,name,out entry);if(def==0)continue;
                sources.Add(Obj("name",name,"model_name",r.Name(r.Word(def+0xc)),
                    "alternate_name",r.Name(r.Word(def+0xd20)),"alternate_index",r.Word(def+0xd24),
                    "secondary_name",r.Name(r.Word(def+0xd34))));
            } catch(Exception e) {sources.Add(Obj("name",name,"error",e.Message));}
        }
        var playerDefinitions=new List<object>();
        if(r.Word(0x01a2e7ec)==10) {
            uint bank=r.Word(0x07817ba8);if(bank>=256)throw new Exception("invalid player bank");
            uint cg=r.Word(0x01a19510+bank*4)^0xfbb2c97a;byte[] ps=r.Read(cg,0xa78);
            var ids=new SortedSet<uint>();for(int at=0x2a4;at<0x2a4+15*4;at+=4)ids.Add(U(ps,at));
            ids.Add(U(ps,0x340));ids.Add(U(ps,0x344));
            foreach(uint id in ids)if(id>0&&id<=count) {
                uint def=r.Word(0x017f5a40+id*4);
                playerDefinitions.Add(Obj("index",id,"name",r.Name(r.Word(def)),"definition_base64",Convert.ToBase64String(r.Read(def,0xe28))));
            }
        }
        Console.WriteLine(new JavaScriptSerializer().Serialize(Obj("schema_version",1,"read_only",true,
            "client_sha256","e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b",
            "connection_state",r.Word(0x01a2e7ec),"weapons",weapons,"sources",sources,"offhand_definitions",offhands,"supply_definitions",supply,"player_definitions",playerDefinitions)));
    }
    object EffectDefinition(uint pointer) {
        return EffectDefinition(pointer,0,new HashSet<uint>());
    }
    object EffectDefinition(uint pointer,int depth,HashSet<uint> seen) {
        if(pointer==0)return null;
        if(depth>8||seen.Count>=128)throw new Exception("FX graph exceeds capture bounds");
        if(!seen.Add(pointer))return Obj("name",Name(Word(pointer)),"already_captured",true);
        byte[] header=Read(pointer,32);
        int count=Count(U(header,16)+U(header,20)+U(header,24),128);
        byte[] raw=Read(U(header,28),count*0x104);var elements=new List<object>();
        for(int i=0;i<count;i++) {
            int at=i*0x104;var children=new List<object>();uint visuals=raw[at+0xb1],visual=U(raw,at+0xbc);
            if(raw[at+0xb0]==14) {
                if(visuals>32)throw new Exception("FX runner count exceeds capture bounds");
                for(uint v=0;v<visuals;v++)children.Add(EffectDefinition(visuals==1?visual:Word(visual+v*4),depth+1,seen));
            }
            foreach(int offset in new int[]{0xd8,0xdc,0xe0})if(U(raw,at+offset)!=0)
                children.Add(EffectDefinition(U(raw,at+offset),depth+1,seen));
            uint vis=U(raw,at+0xb8),vel=U(raw,at+0xb4);
            elements.Add(Obj("index",i,"type",raw[at+0xb0],"children",children,
                "visual_samples_base64",vis==0?null:Convert.ToBase64String(Read(vis,(raw[at+0xb3]+1)*0x40)),
                "velocity_samples_base64",vel==0?null:Convert.ToBase64String(Read(vel,(raw[at+0xb2]+1)*0x60))));
        }
        return Obj("name",Name(U(header,0)),"flags",U(header,4),"total_size",U(header,8),
            "looping_life_msec",U(header,12),"looping_count",U(header,16),
            "one_shot_count",U(header,20),"emission_count",U(header,24),
            "element_stride",0x104,"elements_base64",Convert.ToBase64String(raw),"elements",elements);
    }
    object WeaponRuntimeFields(uint at) {
        // 0x4253e0 writes def+0x6f0 to PS.weaponDelay and def+0xd10 to
        // PS.weaponTime; no capacity field is guessed here. Names below are
        // diagnostic slots; 0x5587e0 explicitly consumes def+0x154/+0x15c.
        var names=new Dictionary<string,object>();
        foreach(uint offset in new uint[]{0x7c,0x80,0x84,0x88,0x154,0x15c}) {
            uint pointer=Word(checked(at+offset));
            names["0x"+offset.ToString("x")]=pointer==0?null:Name(pointer);
        }
        return Obj("fire_delay_msec",Word(at+0x6f0),"fire_interval_msec",Word(at+0xd10),
            // 0x4232d0 selects normal/empty duration; 0x4230a0 selects the
            // earlier ammo-transfer marker (zero means the full duration).
            "reload_msec",Word(at+0x71c),"reload_empty_msec",Word(at+0x728),
            "reload_add_msec",Word(at+0x730),"reload_empty_add_msec",Word(at+0x738),
            // 0x4224e0: fraction +=/-= frameMsec * def+0xbac/+0xbb0;
            // reload gates use+0xd78/+0xd86 and+0xb38. Keep float bits exact.
            "ads_enabled",Read(at+0xd76,1)[0],"ads_in_rate_bits",Word(at+0xbac),"ads_out_rate_bits",Word(at+0xbb0),
            "ads_reload_allowed",Read(at+0xd78,1)[0],"ads_reload_interruptible",Read(at+0xd86,1)[0],
            "ads_reload_end_msec",Word(at+0xb38),
            "animation_name_fields",names);
    }
    uint ItemRecord(ushort id) {
        // Target0x5845b0: compact item ID -> record index ->40-byte record.
        if(id==0)return 0;
        uint group=BitConverter.ToUInt16(Read(0x02db8ad4+(uint)(id/1000)*2,2),0);
        uint slot=(group+(uint)(id%1000))&65535;
        if(slot>=Word(0x02db8ad0))return 0;
        uint index=BitConverter.ToUInt16(Read(Word(0x02db8acc)+slot*2,2),0);
        if(index==0||index>=Word(0x02db8ac8))return 0;
        return Word(0x02db8ac4)+index*0x28;
    }
    uint AttachmentIndex(string name,byte[] registry) {
        // Target0x42c740: multiply/XOR hash, upper run then wrapped lower run.
        uint hash=0;foreach(char c in name)hash=unchecked(hash*0x1000193^(uint)c);
        uint start=hash%0x3fff,found=0;
        for(uint i=start==0?1:start;i<0x3fff;i++) {
            uint ptr=U(registry,(int)i*4);if(ptr==0)return 0;
            if(String.Equals(Name(ptr),name,StringComparison.OrdinalIgnoreCase)){found=i;break;}
        }
        for(uint i=1;i<start;i++) {
            uint ptr=U(registry,(int)i*4);if(ptr==0)return found;
            if(String.Equals(Name(ptr),name,StringComparison.OrdinalIgnoreCase))return i;
        }
        return found;
    }
    byte[] itemPool;
    object M4DefaultParts(byte[] registry) { return WeaponDefaultParts("m4_mp",registry); }
    object WeaponDefaultParts(string weaponName,byte[] registry) {
        // 0x584470 finds a record by its internal name; this bounded scan
        // reads the same pool without calling the game or its hash routine.
        int count=Count(Word(0x02db8ac8),65535);uint pool=Word(0x02db8ac4);
        byte[] records=itemPool??(itemPool=Read(pool,count*0x28));uint found=0;
        for(int i=1;i<count;i++)if(Name(U(records,i*0x28+12))==weaponName) {
            if(found!=0)throw new Exception("ambiguous M4 item record");found=pool+(uint)i*0x28;
        }
        if(found==0)throw new Exception("M4 item record missing");
        uint detailIndex=BitConverter.ToUInt16(Read(found+2,2),0);
        uint detailCount=BitConverter.ToUInt16(Read(0x02dbba30,2),0);
        if(detailIndex==0||detailIndex>=detailCount)throw new Exception("M4 details index invalid");
        uint details=Word(0x02dbba2c)+detailIndex*0x4c;
        var parts=new List<object>();
        // 0x709650 compares item ID against details+0x20+item.kind*2.
        // +0x40 is separate; do not treat it as a seventeenth slot.
        for(uint kind=0;kind<16;kind++) {
            ushort item=BitConverter.ToUInt16(Read(details+0x20+kind*2,2),0);
            if(item==0||item==20008)continue; //0x709650 treats20008 as universal none
            uint at=ItemRecord(item);if(at==0)throw new Exception("default part item missing");
            byte actualKind=Read(at+6,1)[0];if(actualKind!=kind)throw new Exception("default part kind mismatch");
            string name=Name(Word(at+12));uint wire=AttachmentIndex(name,registry);
            if(wire==0)throw new Exception("default part not registered: "+name);
            parts.Add(Obj("kind",kind,"item_id",item,"name",name,"wire_index",wire));
        }
        return Obj("weapon",weaponName,"parts",parts);
    }
    public static void ExportArsenal(IntPtr process) {
        var r=new ClientAssets(process);
        if(r.Word(0x01a2e7ec)!=10)throw new Exception("load the map before arsenal capture");
        var marketplace=(Dictionary<string,object>)r.Table("mp/coMarketplaceItems.csv");
        var cells=(List<string>)marketplace["cells_base64"];int columns=(int)marketplace["columns"];
        var items=new Dictionary<string,List<uint>>(StringComparer.Ordinal);
        for(int row=0;row<cells.Count;row+=columns) {
            string group=Encoding.ASCII.GetString(Convert.FromBase64String(cells[row+2]));
            if(group!="wea")continue;
            string name=Encoding.ASCII.GetString(Convert.FromBase64String(cells[row+1]));
            uint id=UInt32.Parse(Encoding.ASCII.GetString(Convert.FromBase64String(cells[row])));
            List<uint> ids;if(!items.TryGetValue(name,out ids)){ids=new List<uint>();items.Add(name,ids);}ids.Add(id);
        }
        int count=Count(r.Word(0x07817b88),899);byte[] names=r.Read(0x017e5848,16384*4);
        var weapons=new List<object>();
        // Alternate modes (def+0xd24, e.g. CSG12SS) have no marketplace item;
        // export the base alternates of every exported weapon as well.
        var alternates=new HashSet<uint>();
        for(uint i=1;i<=count;i++) {
            uint def=r.Word(0x017f5a40+i*4);if(def==0)continue;
            if(!items.ContainsKey(r.Name(r.Word(def))))continue;
            uint alt=r.Word(def+0xd24);if(alt!=0&&alt<=count)alternates.Add(alt);
        }
        for(uint i=1;i<=count;i++) {
            uint def=r.Word(0x017f5a40+i*4);if(def==0)continue;
            string name=r.Name(r.Word(def));
            bool alternateOnly=!items.ContainsKey(name);
            if(alternateOnly&&!alternates.Contains(i))continue;
            var compatible=new List<object>();int n=Count(r.Read(0x017f5848+i,1)[0],64);
            byte[] pairs=r.Read(0x017f8248+i*0x100,n*4);
            for(int j=0;j<n;j++) {
                uint wire=BitConverter.ToUInt16(pairs,j*4),attr=BitConverter.ToUInt16(pairs,j*4+2);
                if(wire==0||wire>=0x3fff)throw new Exception("attachment index invalid");
                string part=r.Name(U(names,(int)wire*4));uint data=r.Word(0x017f7a48+attr*4);
                int fields=Count(r.Word(data+4),256);
                compatible.Add(Obj("name",part,"wire_index",wire,"items",r.AttachmentItems(part,wire),
                    "attributes_base64",Convert.ToBase64String(fields==0?new byte[0]:r.Read(r.Word(data+8),fields*12))));
            }
            if(alternateOnly) {
                weapons.Add(Obj("name",name,"base_index",i,"alternate_only",true,
                    "definition_base64",Convert.ToBase64String(r.Read(def,0xe28)),
                    "melee_charge_animation",r.Name(r.Word(def+0x98)),"compatible",compatible));
                continue;
            }
            weapons.Add(Obj("name",name,"base_index",i,"item_ids",items[name],
                "definition_base64",Convert.ToBase64String(r.Read(def,0xe28)),
                "melee_charge_animation",r.Name(r.Word(def+0x98)),
                "defaults",r.WeaponDefaultParts(name,names),"compatible",compatible));
        }
        var serializer=new JavaScriptSerializer();serializer.MaxJsonLength=16*1024*1024;
        Console.WriteLine(serializer.Serialize(Obj("schema_version",1,"read_only",true,
            "client_sha256","e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b",
            "connection_state",10,"ncs_crc",r.Word(0x02df7d90),"registered_count",count,
            "marketplace",marketplace,"weapons",weapons)));
    }
    public static void ExportCollision(IntPtr process) {
        var r=new ClientAssets(process);
        uint state=r.Word(0x01a2e7ec);if(state<6||state>10)throw new Exception("load the map before collision capture");
        uint cm=0x02dfb000;byte[] header=r.Read(cm,256);string map=r.Name(U(header,0));
        if(map!="maps/mp/mp_afghan.d3dbsp"&&map!="maps/mp/mp_killhouse_sh.d3dbsp"&&map!="maps/mp/mp_crash_tropical.d3dbsp")throw new Exception("unexpected collision map: "+map);
        var brushes=new List<object>();int count=Count(BitConverter.ToUInt16(header,0x38),65535);
        int materialCount=Count(U(header,0x10),65536);byte[] materials=r.Read(U(header,0x14),materialCount*12);
        byte[] raw=r.Read(U(header,0x3c),count*36),bounds=r.Read(U(header,0x40),count*24),contents=r.Read(U(header,0x44),count*4);
        for(int b=0;b<count;b++) {
            uint mask=U(contents,b*4);if((mask&(0x02814011|0x0280e831))==0)continue;
            var planes=new List<object>();var surfaceFlags=new List<uint>();int sides=Count(BitConverter.ToUInt16(raw,b*36),256);
            byte[] side=sides==0?new byte[0]:r.Read(U(raw,b*36+4),sides*8);
            // Current0x589520: side+4 holds the material index; brush+0xc
            // stores -X/-Y/-Z indices and +0x12 stores +X/+Y/+Z indices.
            for(int j=0;j<sides;j++) {
                byte[] plane=r.Read(U(side,j*8),16);planes.Add(Floats(plane,0,4));
                int material=BitConverter.ToUInt16(side,j*8+4);if(material>=materialCount)throw new Exception("brush side material invalid");
                surfaceFlags.Add(U(materials,material*12+4));
            }
            for(int axis=0;axis<3;axis++)foreach(int face in new int[]{0x12+axis*2,0xc+axis*2}) {
                int material=BitConverter.ToUInt16(raw,b*36+face);if(material>=materialCount)throw new Exception("brush axial material invalid");
                surfaceFlags.Add(U(materials,material*12+4));
            }
            brushes.Add(Obj("center",Floats(bounds,b*24,3),"half",Floats(bounds,b*24+12,3),"contents",mask,"planes",planes,"plane_surface_flags",surfaceFlags));
        }
        int vc=Count(U(header,0x64),300000),tc=Count(U(header,0x6c),600000),pc=Count(U(header,0x80),100000),ac=Count(U(header,0x88),100000);
        byte[] verts=r.Read(U(header,0x68),vc*12),tris=r.Read(U(header,0x70),tc*6),parts=r.Read(U(header,0x84),pc*12),trees=r.Read(U(header,0x8c),ac*32);
        var triangles=new List<object>();var done=new HashSet<uint>();
        for(int a=0;a<ac;a++) {
            if(BitConverter.ToUInt16(trees,a*32+14)!=0)continue;
            uint part=U(trees,a*32+28);if(part>=pc)throw new Exception("collision partition index invalid");
            uint mat=BitConverter.ToUInt16(trees,a*32+12);byte[] material=r.Read(U(header,0x14)+mat*12,12);uint mask=U(material,8);
            if((mask&(0x02814011|0x0280e831))==0||!done.Add(part))continue;
            uint first=U(parts,(int)part*12+4),block=parts[part*12+2];int n=parts[part*12];
            if(first+n>tc)throw new Exception("collision triangle span invalid");
            for(uint t=first;t<first+n;t++) {
                var points=new List<object>();
                for(int j=0;j<3;j++){uint v=block*1024+BitConverter.ToUInt16(tris,(int)t*6+j*2);if(v>=vc)throw new Exception("collision vertex invalid");points.Add(Floats(verts,(int)v*12,3));}
                triangles.Add(Obj("vertices",points,"contents",mask,"surface_flags",U(material,4)));
            }
        }
        var serializer=new JavaScriptSerializer();serializer.MaxJsonLength=64*1024*1024;
        Console.WriteLine(serializer.Serialize(Obj("schema_version",1,"read_only",true,
            "client_sha256","e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b",
            "map",map,"ncs_crc",r.Word(0x02df7d90),"captured_contents_mask",0x02814011|0x0280e831,"brushes",brushes,"triangles",triangles)));
    }
    object ViewModelNames() {
        try {
            uint slot=Word(0x07817bb8);if(slot>=256)throw new Exception("invalid viewmodel slot");
            uint views=Word(0x01a19510+slot*4)^0x39641402;
            var hands=new List<object>();
            for(uint hand=0;hand<2;hand++) {
                uint dobj=Word(views+hand*24);if(dobj==0)continue;
                int count=Count(Read(dobj+9,1)[0],32);uint list=Word(dobj+0xc0);
                var names=new List<string>();
                for(uint i=0;i<count;i++)names.Add(Name(Word(Word(list+i*4))));
                if(dobj!=Word(views+hand*24)||list!=Word(dobj+0xc0))throw new Exception("viewmodel changed during read");
                hands.Add(Obj("hand",hand,"models",names));
            }
            return hands;
        } catch(Exception error) {return Obj("error",error.Message);}
    }
    object CharacterInfo(uint cg,uint client) {
        if(client>=19)return null;
        uint ci=cg+0x15dea8+client*0x3b8;
        return Obj("client",client,"parts",Word(ci+0x38c),"faction",Word(ci+0x390),
            "gender",Word(ci+0x394),"skin",Word(ci+0x398),
            "model_load_state",Word(ci+0x344),"model_changed",Word(ci+0x230));
    }
    uint ClipCount(byte[] ps,uint weapon) {
        if(weapon==0||weapon>=900)return 0;
        uint def=Word(0x017f5a40+weapon*4);if(def<0x10000)return 0;
        uint key=Word(def+0x6c0);
        for(int slot=0;slot<15;slot++)if(U(ps,0x3dc+slot*12)==key)return U(ps,0x3e0+slot*12);
        return 0;
    }
    object MissileStates() {
        uint slot=Word(0x07817bb0);if(slot>=256)throw new Exception("invalid missile entity slot");
        uint entities=Word(0x01a19510+slot*4)^0x2168f672;var missiles=new List<object>();
        for(uint id=1700;id<1708;id++) {
            uint entity=entities+id*0x238;
            if((Word(entity+0x204)&1)==0||Word(entity+0xf4)!=4)continue;
            uint dobjSlot=BitConverter.ToUInt16(Read(0x02ef4858+id*2,2),0);var models=new List<string>();
            if(dobjSlot!=0&&dobjSlot<2048) {
                uint dobj=0x02e8e850+dobjSlot*0xcc,list=Word(dobj+0xc0);int count=Count(Read(dobj+9,1)[0],32);
                for(uint part=0;part<count;part++)models.Add(Name(Word(Word(list+part*4))));
            }
            missiles.Add(Obj("entity",id,"weapon",Word(entity+0xf0+0xd0)&65535,
                "origin",EntityOrigin(entity+0xf0),"models",models));
        }
        return missiles;
    }
    object BagStates() {
        uint slot=Word(0x07817bb0);if(slot>=256)throw new Exception("invalid item entity slot");
        uint entities=Word(0x01a19510+slot*4)^0x2168f672;var bags=new List<object>();
        for(uint id=1740;id<1800;id++) {
            uint entity=entities+id*0x238;
            if((Word(entity+0x204)&1)==0||Word(entity+0xf4)!=3)continue;
            uint handle=BitConverter.ToUInt16(Read(0x02ef4858+id*2,2),0);var models=new List<string>();
            if(handle!=0&&handle<2048) {
                uint obj=0x02e8e850+handle*0xcc,list=Word(obj+0xc0);int count=Count(Read(obj+9,1)[0],32);
                for(uint part=0;part<count;part++)models.Add(Name(Word(Word(list+part*4))));
            }
            bags.Add(Obj("entity",id,"item",Word(entity+0x188),"flags",Word(entity+0xf8),"owner",Word(entity+0x170),
                "origin",EntityOrigin(entity+0xf0),"scale",BitConverter.ToSingle(Read(entity+0x144,4),0),
                "dobj_slot",handle,"models",models,"hidden_until",Word(entity+0x210),"raw_base64",Convert.ToBase64String(Read(entity,0x238))));
        }
        return bags;
    }
    object SmokeStates() {
        uint slot=Word(0x07817bb0);if(slot>=256)throw new Exception("invalid smoke entity slot");
        uint entities=Word(0x01a19510+slot*4)^0x2168f672;var smokes=new List<object>();
        for(uint id=1708;id<1740;id++) {
            uint entity=entities+id*0x238;
            if((Word(entity+0x204)&1)==0||Word(entity+0xf4)!=0||(Word(entity+0xf8)&0x40)==0)continue;
            smokes.Add(Obj("entity",id,"flags",Word(entity+0xf8),"weapon",Word(entity+0x1c0)&65535,
                "start_time",Word(entity+0xf0+0x58),"end_time",Word(entity+0xf0+0x7c),"origin",EntityOrigin(entity+0xf0)));
        }
        return smokes;
    }
    object EffectStates() {
        try {
            // 0x4d5480 traverses the live handle ring, not a dense FX array.
            byte[] system=Read(0x02972874,0x888);
            uint arena=U(system,0),begin=U(system,0x7c),end=U(system,0x80);
            uint count=end-begin;if(count>1024||count>U(system,0x84)-begin)throw new Exception("FX ring changed during read");
            var effects=new List<object>();
            for(uint i=0;i<count;i++) {
                uint handle=BitConverter.ToUInt16(system,0x88+(int)(((begin+i)&1023)*2));
                if(handle==65535||handle%47!=0||handle/47>=1024)throw new Exception("invalid FX handle");
                uint address=checked(arena+handle*4);byte[] effect=Read(address,0x38);
                if((U(effect,4)&0x1fff)==0||(U(effect,4)&0xc0000000)!=0)continue;
                string name=Name(Word(U(effect,0)));
                if(name.IndexOf("grenade",StringComparison.Ordinal)<0&&!name.StartsWith("explosions/",StringComparison.Ordinal))continue;
                byte[] checkedState=Read(address,0x94);
                if(U(checkedState,0)!=U(effect,0)||U(checkedState,0x30)!=U(effect,0x30)
                    ||(U(checkedState,4)&0x1fff)==0||(U(checkedState,4)&0xc0000000)!=0)continue;
                effects.Add(Obj("handle",handle,"name",name,"owner",BitConverter.ToUInt16(effect,0x1a),
                    "flags",BitConverter.ToUInt16(effect,0x24),"start_time",U(effect,0x30),"origin",Floats(checkedState,0x48,3)));
            }
            return effects;
        } catch(Exception error) {return Obj("error",error.Message);}
    }
    object EffectVisibility() {
        try {
            uint pointer=Word(0x029728e8);byte[] data=Read(pointer,0x1010);
            uint count=U(data,0x1000);if(count>256||Word(0x029728e8)!=pointer)
                throw new Exception("FX visibility buffer changed during read");
            return Obj("count",count,"buffer_base64",Convert.ToBase64String(data));
        } catch(Exception error) {return Obj("error",error.Message);}
    }
    public static void ExportReplay(IntPtr process) {
        var r=new ClientAssets(process);var frames=new List<object>();uint previousActive=uint.MaxValue;
        for(int i=0;i<400;i++) {
            if(r.Word(0x01a2e7ec)!=10)break;
            uint slot=r.Word(0x07817ba8);if(slot>=256)throw new Exception("invalid cg slot");
            uint cg=r.Word(0x01a19510+slot*4)^0xfbb2c97a;
            byte[] ps=r.Read(cg,0xa78);
            uint active=r.Word(cg+0xc6fe8);
            if(active!=previousActive) { Console.Error.WriteLine("killcam_state="+active);Console.Error.Flush();previousActive=active; }
            uint input=r.Word(0x01a2e838)^0x6891d1c7;uint command=r.Word(input+0x625c);
            // AimAssist's local client is0; the CG pointer slot is an
            // obfuscation-bank index, not a local-client number.
            uint assist=0x02c27580;
            byte[] assistState=r.Read(assist,0xe90);
            uint candidateCount=U(assistState,0xe80);
            var meleeCandidates=new List<object>();
            if(candidateCount<=64) for(int candidate=0;candidate<candidateCount;candidate++) {
                int at=0x180+candidate*0x34;
                meleeCandidates.Add(Obj("entity",U(assistState,at),
                    "clip_min",Floats(assistState,at+4,2),"clip_max",Floats(assistState,at+12,2),
                    "target",Floats(assistState,at+20,3),"velocity",Floats(assistState,at+32,3),
                    "distance_squared",BitConverter.ToSingle(assistState,at+44),
                    "crosshair_distance_squared",BitConverter.ToSingle(assistState,at+48)));
            }
            // Native L <client> 7 populates this bounded presentation card.
            // Read its copied nickname, never profile/account identifiers.
            byte[] card=r.Read(0x037e8108+7*0xa8,0xa8);
            int nameEnd=Array.IndexOf(card,(byte)0,0x20,64);if(nameEnd<0)nameEnd=0x60;
            frames.Add(Obj("time",r.Word(cg+0xae4c8),"active",r.Word(cg+0xc6fe8),
                "card_visible",r.Read(0x0b5f3024,1)[0],
                "render_origin",Floats(r.Read(cg+0xae504,12),0,3),
                "render_angles",Floats(r.Read(cg+0xb2688,12),0,3),
                "cmd_angles",new uint[]{r.Word(input+0x365c+(command&255)*44+8),r.Word(input+0x365c+(command&255)*44+12)},
                "card_time",U(card,0),"card_name",Encoding.GetEncoding(936).GetString(card,0x20,nameEnd-0x20),
                "card_title",U(card,4),"card_background",U(card,8),"card_icon",U(card,12),"card_rank",U(card,0x14),
                "buttons",r.Word(input+0x365c+(command&255)*44+4),
                "timer_type",r.Word(cg+0xa78),"timer_end",r.Word(cg+0xa78+0x78),
                "timer_font",r.Word(cg+0xa78+0x24),"timer_scale_bits",r.Word(cg+0xa78+0x14),
                "skip_type",r.Word(cg+0xa78+0xa8),"skip_text",r.Word(cg+0xa78+0xa8+0x84),
                "skip_y",BitConverter.ToSingle(r.Read(cg+0xa78+0xa8+8,4),0),
                "skip_scale",BitConverter.ToSingle(r.Read(cg+0xa78+0xa8+0x14,4),0),
                "delay",U(ps,0xa68),"client",U(ps,0x144),"target",U(ps,0xa6c),
                "character",r.CharacterInfo(cg,U(ps,0x144)),"first_person",r.ViewModelNames(),"missiles",r.MissileStates(),"smokes",r.SmokeStates(),"bags",r.BagStates(),"effects",r.EffectStates(),"effect_visibility",r.EffectVisibility(),
                "event_sequence",U(ps,0xf4),"events_base64",Convert.ToBase64String(ps,0xf8,32),
                "external_event_sequence",U(ps,0x11c),"external_events_base64",Convert.ToBase64String(ps,0x124,32),
                "feedback_type",r.Word(cg+0x1ed0),"feedback_material",r.Word(cg+0x1ed0+0x4c),
                "feedback_begin",r.Word(cg+0x1ed0+0x38),"feedback_duration",r.Word(cg+0x1ed0+0x3c),
                "feedback_x",BitConverter.ToSingle(r.Read(cg+0x1ed0+4,4),0),"feedback_y",BitConverter.ToSingle(r.Read(cg+0x1ed0+8,4),0),
                "feedback_width",r.Word(cg+0x1ed0+0x44),"feedback_height",r.Word(cg+0x1ed0+0x48),
                "inventory_base64",Convert.ToBase64String(ps,0x2a4,60),"stocks_base64",Convert.ToBase64String(ps,0x364,120),"clips_base64",Convert.ToBase64String(ps,0x3dc,180),
                "pm_type",U(ps,4),"other_flags",U(ps,0x10),"command_time",U(ps,0),
                "shellshock_index",U(ps,0x5ac),"shellshock_begin",U(ps,0x5b0),"shellshock_duration",U(ps,0x5b4),"shellshock_adjacent",U(ps,0x5a8),
                "shellshock_cache_base64",Convert.ToBase64String(r.Read(cg+0xbdb68,0x30)),
                "origin",Floats(ps,0x1c,3),"angles",Floats(ps,0x14c,3),
                "weapon",U(ps,0x340),"weapon_state",U(ps,0x27c),"clip",U(ps,0x3e0),
                "offhand_weapon",U(ps,0x32c),"lethal_weapon",U(ps,0x330),"secondary_offhand_weapon",U(ps,0x334),"tactical_weapon",U(ps,0x338),
                "grenade_time",U(ps,0x34),"weapon_flags",U(ps,0x348),"offhand_count",r.ClipCount(ps,U(ps,0x32c)),"lethal_count",r.ClipCount(ps,U(ps,0x330)),"tactical_count",r.ClipCount(ps,U(ps,0x338)),
                "weapon_time",U(ps,0x270),"fire_count",U(ps,0x284),"queued_burst",U(ps,0x280)&1,
                "weapon_delay",U(ps,0x274),"weapon_animation",U(ps,0x26c),
                "cmd_melee_yaw",BitConverter.ToSingle(r.Read(input+0x365c+(command&255)*44+28,4),0),
                "cmd_melee_distance",r.Read(input+0x365c+(command&255)*44+32,1)[0],
                "cmd_melee_entity",BitConverter.ToUInt16(r.Read(input+0x365c+(command&255)*44+34,2),0),
                "pm_flags",U(ps,12),"melee_yaw",BitConverter.ToSingle(ps,0x4a8),
                "melee_distance",U(ps,0x4ac),"melee_time",U(ps,0x4b0),
                "melee_assist_range",BitConverter.ToSingle(assistState,0x14),
                "melee_assist_half_clip",Floats(assistState,0x154,2),
                "melee_assist_origin",Floats(assistState,0xc4,3),
                "melee_assist_matrix",Floats(assistState,0x40,16),
                "melee_assist_viewport",Floats(assistState,0x108,2),
                "melee_assist_fov_scale",BitConverter.ToSingle(assistState,0xf8),
                "melee_assist_base_fov",BitConverter.ToSingle(r.Read(r.Word(0x01a2035c)+12,4),0),
                "melee_candidate_count",candidateCount,"melee_candidates",meleeCandidates,
                "melee_row",U(ps,0x33c),
                "health",U(ps,0x190),"spawn_count",U(ps,0x19c),"corpse_slot",U(ps,0xe8),
                "stable_command",r.Word(cg)==U(ps,0)));
            System.Threading.Thread.Sleep(25);
        }
        var serializer=new JavaScriptSerializer();serializer.MaxJsonLength=16*1024*1024;
        Console.WriteLine(serializer.Serialize(Obj("schema_version",1,"read_only",true,
            "client_sha256","e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b",
            "frames",frames)));
    }
    public static object TopPlayerCards(IntPtr process) {
        var r=new ClientAssets(process);var cards=new List<object>();
        // GetTop3Players0x6b0130 reads only presentation slots1..3.
        for(uint slot=1;slot<=3;slot++) {
            byte[] card=r.Read(0x037e8108+slot*0xa8,0xa8);
            int end=Array.IndexOf(card,(byte)0,0x20,64);if(end<0)end=0x60;
            cards.Add(Obj("slot",slot,"time",U(card,0),
                "name",Encoding.GetEncoding(936).GetString(card,0x20,end-0x20),
                "rank",U(card,0x14),"prestige",U(card,0x18),
                "title",U(card,4),"background",U(card,8),"icon",U(card,12)));
        }
        return cards;
    }
    public static object ShowcaseActors(IntPtr process) {
        var r=new ClientAssets(process);var actors=new List<object>();
        uint slot=r.Word(0x07817bb0);if(slot>=256)throw new Exception("invalid entity slot");
        uint entities=r.Word(0x01a19510+slot*4)^0x2168f672;
        for(uint id=12;id<=14;id++) {
            uint at=entities+id*0x238;bool valid=(r.Word(at+0x204)&1)!=0;
            uint cgSlot=r.Word(0x07817ba8);if(cgSlot>=256)throw new Exception("invalid cg slot");
            uint cg=r.Word(0x01a19510+cgSlot*4)^0xfbb2c97a,ci=cg+0x15dea8+id*0x3b8;
            uint dobjSlot=BitConverter.ToUInt16(r.Read(0x02ef4858+id*2,2),0);var models=new List<string>();
            if(dobjSlot!=0) {
                uint obj=0x02e8e850+dobjSlot*0xcc;int count=Count(r.Read(obj+9,1)[0],32);
                uint list=r.Word(obj+0xc0);for(uint part=0;part<count;part++)models.Add(r.Name(r.Word(r.Word(list+part*4))));
            }
            actors.Add(Obj("entity",id,"valid",valid,"type",r.Word(at+0xf4),
                "dobj_slot",dobjSlot,"models",models,"cached_weapon",r.Word(ci+0x324),
                "entity_bg_weapon",r.Word(at+0xf0+0x60),"weapon_flags",r.Word(at+0xf0+8),
                "client",r.Word(at+0xf0+0x9c),"weapon",r.Word(at+0xf0+0xd0)&0xffff,
                "origin",valid?r.EntityOrigin(at+0xf0):null));
        }
        return actors;
    }
    public static object HeldWeapons(IntPtr process) {
        var r=new ClientAssets(process);var result=new List<object>();
        uint slot=r.Word(0x07817bb0),cgSlot=r.Word(0x07817ba8);
        if(slot>=256||cgSlot>=256)throw new Exception("invalid native pointer slots");
        uint entities=r.Word(0x01a19510+slot*4)^0x2168f672,cg=r.Word(0x01a19510+cgSlot*4)^0xfbb2c97a;
        for(uint parent=0;parent<27;parent++) {
            uint body=entities+parent*0x238,id=27+parent,at=entities+id*0x238;
            bool parentValid=(r.Word(body+0x204)&1)!=0,valid=(r.Word(at+0x204)&1)!=0;
            if(!parentValid&&!valid)continue;
            uint link=r.Word(at+0x1d4),weapon=r.Word(at+0x158),dobjSlot=BitConverter.ToUInt16(r.Read(0x02ef4858+id*2,2),0);
            var models=new List<string>();
            if(valid&&dobjSlot!=0) {
                uint obj=0x02e8e850+dobjSlot*0xcc;int count=Count(r.Read(obj+9,1)[0],32);
                uint list=r.Word(obj+0xc0);for(uint part=0;part<count;part++)models.Add(r.Name(r.Word(r.Word(list+part*4))));
            }
            result.Add(Obj("parent",parent,"parent_valid",parentValid,"entity",id,"valid",valid,
                "parent_origin",parentValid?r.EntityOrigin(body+0xf0):null,
                "type",r.Word(at+0xf4),"weapon",weapon,"link_parent",link&0xffff,"link_tag",(link>>16)&255,
                "dobj_slot",dobjSlot,"models",models,"client_reference",parent<19?r.Word(cg+0x15dea8+parent*0x3b8+0x330):0));
        }
        return result;
    }
    public static void ExportFeedback(IntPtr process) {
        var r=new ClientAssets(process);
        if(r.Word(0x01a2e7ec)!=10)throw new Exception("feedback capture requires active map");
        var resources=new Dictionary<string,object>();
        foreach(string name in new string[]{"damage_feedback","kill_feedback","MP_hit_alert","waypoint_dead"}) {
            try {
            uint entry;uint asset=r.Find(name=="MP_hit_alert"?11u:5u,name,out entry);
            resources[name]=asset==0?Obj("found",false):Obj("found",true,"header",asset,"layout",r.Layout(asset,name=="MP_hit_alert"?12:104,1));
            } catch(Exception error) { resources[name]=Obj("found",false,"error",error.Message); }
        }
        var frames=new List<object>();
        for(int i=0;i<200;i++) {
            if(r.Word(0x01a2e7ec)!=10)break;
            uint slot=r.Word(0x07817ba8);if(slot>=256)throw new Exception("invalid cg slot");
            uint cg=r.Word(0x01a19510+slot*4)^0xfbb2c97a;
            byte[] hud=r.Read(cg+0x1ed0,0xa8);
            var deaths=new List<object>();
            for(uint slotIndex=0;slotIndex<5;slotIndex++) {
                byte[] death=r.Read(cg+0xa78+slotIndex*0xa8,0xa8);
                if(U(death,0)==0)break;
                deaths.Add(Obj("type",U(death,0),"material",U(death,0x4c),"origin",Floats(death,4,3),
                    "color",U(death,0x30),"flags",U(death,0xa4),"entity",U(death,0x10),"width",U(death,0x44),"height",U(death,0x48)));
            }
            frames.Add(Obj("death_waypoints",deaths,"time",r.Word(cg+0xae4c8),"type",U(hud,0),"material",U(hud,0x4c),
                "color",U(hud,0x30),"from_color",U(hud,0x34),"fade_start",U(hud,0x38),"fade_msec",U(hud,0x3c)));
            System.Threading.Thread.Sleep(25);
        }
        var serializer=new JavaScriptSerializer();serializer.MaxJsonLength=4*1024*1024;
        Console.WriteLine(serializer.Serialize(Obj("schema_version",1,"read_only",true,
            "client_sha256","e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b",
            "ncs_crc",r.Word(0x02df7d90),"resources",resources,"frames",frames)));
    }
    public static void ExportCharacters(IntPtr process) {
        var r=new ClientAssets(process);
        var hudMaterials=new Dictionary<string,object>();
        foreach(string name in new string[]{"compass_map_mp_crash_tropical","compass_map_mp_crash_dlc","compass_map_mp_crash","compass_map_mp_afghan","compass_map_mp_killhouse_sh","waypoint_dead","headicon_dead","waypoint_flag_friendly"}) {
            try {
                uint entry;uint material=r.Find(5,name,out entry);
                if(material==0) {hudMaterials[name]=Obj("found",false);continue;}
                // 0xaa3650 tests rendering data, not XAssetEntry+0x10.
                uint fallback=r.Word(0x00f31444);
                bool isDefault=r.Word(material+0x58)==r.Word(fallback+0x58)
                    && r.Word(material+0x5c)==r.Word(fallback+0x5c)
                    && r.Word(material+0x60)==r.Word(fallback+0x60);
                int count=Count(r.Read(material+0x50,1)[0],32);
                uint textureTable=r.Word(material+0x5c);
                var textures=new List<object>();
                for(int t=0;t<count;t++) {
                    uint texture=textureTable+(uint)t*12;
                    uint image=r.Word(texture+8);
                    textures.Add(Obj("semantic",r.Read(texture+7,1)[0],"image",image,
                        "name",r.Name(r.Word(image+0x30)),
                        "width",BitConverter.ToUInt16(r.Read(image+0x20,2),0),
                        "height",BitConverter.ToUInt16(r.Read(image+0x22,2),0),
                        "texture",r.Word(image)));
                }
                hudMaterials[name]=Obj("found",true,"render_default",isDefault,
                    "entry_byte16",r.Read(entry+0x10,1)[0],"flags",r.Read(entry+0x11,1)[0],
                    "header",material,"textures",textures,
                    "render_layout",name.Contains("dead")?r.Layout(material,104,2):null);
            } catch(Exception error) {hudMaterials[name]=Obj("error",error.Message);}
        }
        if(r.Word(0x01a2e7ec)!=10)throw new Exception("load the map before character capture");
        uint slot=r.Word(0x07817bac);if(slot>=256)throw new Exception("cgs model slot invalid");
        uint cgs=r.Word(0x01a19510+slot*4)^0xce9af8cf;
        byte[] indices=r.Read(cgs+0xf0,1024*4);var models=new List<object>();
        for(int id=1;id<1024;id++) {
            uint model=U(indices,id*4);if(model==0)continue;
            byte[] header=r.Read(model,0x168);int count=Count(header[4],253);
            byte[] ids=count==0?new byte[0]:r.Read(U(header,0x2c),count*2);var bones=new List<string>();
            uint strings=r.Word(0x02f37164);
            for(int bone=0;bone<count;bone++) {
                uint key=BitConverter.ToUInt16(ids,bone*2);
                bones.Add(r.Name(strings+key*20+4));
            }
            float[] center=Floats(header,0x140,3),half=Floats(header,0x14c,3);
            float[] min=new float[3],max=new float[3];for(int i=0;i<3;i++){min[i]=center[i]-half[i];max[i]=center[i]+half[i];}
            models.Add(Obj("model",id,"name",r.Name(U(header,0)),"bounds_min",min,"bounds_max",max,"bone_names",bones));
        }
        var tables=new Dictionary<string,object>();
        foreach(string name in new string[]{"mp/coCharacterDefTable.csv","mp/coCharItemStatsTable.csv","mp/coCharItems/cha.csv","mp/coMapList.csv","mp/coGameTypesTable.csv","mp/botClassTable.csv","mp/botDifficultyTable.csv","mp/botNames.csv"}) {
            try { tables[name]=r.Table(name); } catch(Exception error) { tables[name]=Obj("error",error.Message); }
        }
        // Enumerate only character model/table identities and bounded headers;
        // asset names supply evidence without executing game code.
        var characterAssets=new List<object>();byte[] assetEntries=r.Read(0x01fc4e28,100000*28);
        byte[] assetHeads=r.Read(0x01bc9f20,80000*4);var assetSeen=new HashSet<uint>();
        for(int bucket=0;bucket<80000;bucket++) {
            uint index=U(assetHeads,bucket*4);int chain=0;
            while(index!=0) {
                if(index>=100000||!assetSeen.Add(index)||++chain>128)throw new Exception("character asset chain invalid");
                int at=(int)index*28;uint kind=U(assetEntries,at),header=U(assetEntries,at+8);
                if(kind==4||kind==37||kind==36||kind==45||kind==14) {
                    string name=r.Name(r.Word(header));string lower=name.ToLowerInvariant();
                    if(lower.StartsWith("boo_")||lower.StartsWith("fja_")||lower.StartsWith("fpa_")||lower.StartsWith("han_")||lower.StartsWith("hea_")||lower.StartsWith("heg_")||lower.StartsWith("ves_")||lower.Contains("botdifficulty")||lower.Contains("cochar")||kind==14) {
                        if(characterAssets.Count>=4096)throw new Exception("character asset budget");
                        characterAssets.Add(Obj("kind",kind,"name",name,"flags",assetEntries[at+17]));
                        if(kind==37&&(assetEntries[at+17]&5)!=0&&!tables.ContainsKey(name)) {
                            try{tables[name]=r.Table(name);}catch(Exception e){tables[name]=Obj("error",e.Message);}
                        }
                    }
                }
                index=U(assetEntries,at+20);
            }
        }
        var animationRules=new Dictionary<string,object>();
        foreach(string name in new string[]{"mp/playeranim.script","mp/playeranimtypes.txt","waypoints/mp_afghan_wp.csv","waypoints/mp_killhouse_sh_wp.csv","waypoints/mp_crash_tropical_wp.csv"}) {
            try {animationRules[name]=r.AnimationRules(name);}catch(Exception error){animationRules[name]=Obj("error",error.Message);}
        }
        var clients=new List<object>();
        var animations=new List<object>();var animationContexts=new List<object>();var seenContexts=new HashSet<uint>();
        foreach(uint address in new uint[]{0x07817b80,0x07817b84}) {
            uint context=r.Word(address);if(context<0x10000||!seenContexts.Add(context))continue;
            int count=Count(r.Word(context+0x1a000),1024);animationContexts.Add(Obj("source",address,"count",count));
            byte[] records=r.Read(context,count*0x68);
            for(int animation=1;animation<count;animation++) {
                int begin=animation*0x68,len=0;while(len<64&&records[begin+len]!=0)len++;
                string name=Encoding.ASCII.GetString(records,begin,len);if(name.Length==0)continue;
                animations.Add(Obj("source",address,"index",animation,"name",name,
                    "initial_lerp_msec",U(records,begin+0x40),"duration_msec",U(records,begin+0x48),
                    "move_speed_bits",U(records,begin+0x44),"flags",U(records,begin+0x50)));
            }
        }
        // The animation preparation BG context may be released once loading
        // completes. The renderer's current CG owns the live client infos.
        uint currentCgSlot=r.Word(0x07817ba8);
        if(currentCgSlot>=256)throw new Exception("invalid current character context");
        uint currentCg=r.Word(0x01a19510+currentCgSlot*4)^0xfbb2c97a;
        for(uint id=0;id<12;id++) {
            uint ci=currentCg+0x15dea8+id*0x3b8;var parts=new List<object>();
            for(uint part=0;part<10;part++) {
                uint modelName=r.Word(ci+0x20+part*4),tagName=r.Word(ci+0x48+part*4);
                parts.Add(Obj("part",part,"model",modelName>=0x10000?r.Name(modelName):null,"tag",tagName>=0x10000?r.Name(tagName):null));
            }
            uint root=r.Word(ci+0x18),tree=r.Word(ci+0x320);
            uint dobjSlot=BitConverter.ToUInt16(r.Read(0x02ef4858+id*2,2),0);var objModels=new List<string>();
            if(dobjSlot!=0) {
                uint dobj=0x02e8e850+dobjSlot*0xcc;int count=Count(r.Read(dobj+9,1)[0],32);
                uint array=r.Word(dobj+0xc0);for(uint part=0;part<count;part++)objModels.Add(r.Name(r.Word(r.Word(array+part*4))));
            }
            uint entitySlot=r.Word(0x07817bb0);if(entitySlot>=256)throw new Exception("entity slot invalid");
            uint entities=r.Word(0x01a19510+entitySlot*4)^0x2168f672;
            uint entity=entities+id*0x238;
            clients.Add(Obj("client",id,"valid",r.Word(ci),"current_valid",r.Word(ci+4),
                "character_parts",r.Word(ci+0x38c),"character_faction",r.Word(ci+0x390),
                "character_gender",r.Word(ci+0x394),"character_skin",r.Word(ci+0x398),
                "team",r.Word(ci+0x80),"cached_weapon",r.Word(ci+0x324),"entity_weapon",r.Word(entity+0xf0+0xd0),
                "legs_animation",r.Word(entity+0xf0+0xd4),"torso_animation",r.Word(entity+0xf0+0xd8),
                "legs_lerp_animation",r.Word(ci+0x170),"legs_lerp_rate",BitConverter.ToSingle(r.Read(ci+0x194,4),0),
                "torso_lerp_animation",r.Word(ci+0x1b0),"torso_lerp_rate",BitConverter.ToSingle(r.Read(ci+0x1d4,4),0),
                "entity_client",r.Word(entity+0xf0+0x9c),"entity_type",r.Word(entity+0xf4),
                "entity_events_sequence",r.Word(entity+0xf0+0xac),"entity_event0",r.Word(entity+0xf0+0xb0),
                "body",root>=0x10000?r.Name(root):null,"parts",parts,"animation_tree_ready",tree>=0x10000,
                "dobj_slot",dobjSlot,"dobj_models",objModels,"ground_pose",r.GroundPose(entity,dobjSlot),
                "entity_origin",r.EntityOrigin(entities+id*0x238+0xf0)));
        }
        var corpses=new List<object>();uint corpseEntitySlot=r.Word(0x07817bb0);
        if(corpseEntitySlot>=256)throw new Exception("corpse entity slot invalid");
        uint corpseEntities=r.Word(0x01a19510+corpseEntitySlot*4)^0x2168f672;
        for(uint id=19;id<=26;id++) {
            uint entity=corpseEntities+id*0x238;
            uint slotId=BitConverter.ToUInt16(r.Read(0x02ef4858+id*2,2),0);
            var corpseModels=new List<string>();
            bool corpseValid=(r.Word(entity+0x204)&1)!=0 && r.Word(entity+0xf4)==2;
            if(corpseValid&&slotId!=0&&slotId<2048) {
                uint dobj=0x02e8e850+slotId*0xcc,list=r.Word(dobj+0xc0);
                int count=Count(r.Read(dobj+9,1)[0],32);
                for(uint part=0;part<count;part++)corpseModels.Add(r.Name(r.Word(r.Word(list+part*4))));
            }
            corpses.Add(Obj("entity",id,"type",r.Word(entity+0xf4),"flags",r.Word(entity+0xf8),
                "donor",r.Word(entity+0xf0+0x9c),"legs",r.Word(entity+0xf0+0xd4),"torso",r.Word(entity+0xf0+0xd8),
                "valid",corpseValid,"dobj_models",corpseModels,"dobj_slot",slotId,"origin",r.EntityOrigin(entity+0xf0),
                "ground_pose",corpseValid?r.GroundPose(entity,slotId):null));
        }
        var serializer=new JavaScriptSerializer();serializer.MaxJsonLength=16*1024*1024;
        Console.WriteLine(serializer.Serialize(Obj("schema_version",1,"read_only",true,
            "client_sha256","e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b",
            "ncs_crc",r.Word(0x02df7d90),"models",models,"character_assets",characterAssets,"tables",tables,"clients",clients,"map_hud_materials",hudMaterials,
            "animation_contexts",animationContexts,"animations",animations,"animation_rules",animationRules,"corpses",corpses)));
    }
    public static void ExportTrainingTarget(IntPtr process) {
        var r=new ClientAssets(process);
        if(r.Word(0x01a2e7ec)!=10)throw new Exception("load the map before training model capture");
        string name="com_barrel_russian_fuel_dirt";uint entry;uint model=r.Find(4,name,out entry);
        if(model==0)throw new Exception("training model is not loaded");
        uint slot=r.Word(0x07817bac);if(slot>=256)throw new Exception("invalid cgs model slot");
        uint cgs=r.Word(0x01a19510+slot*4)^0xce9af8cf;uint id=0;
        byte[] models=r.Read(cgs+0xf0,1024*4);
        for(uint i=1;i<1024;i++)if(U(models,(int)i*4)==model) {if(id!=0)throw new Exception("ambiguous training model index");id=i;}
        if(id==0)throw new Exception("training model has no network index");
        byte[] bounds=r.Read(model+0x140,24);float[] center=Floats(bounds,0,3),half=Floats(bounds,12,3);
        float[] mins=new float[3],maxs=new float[3];for(int i=0;i<3;i++){if(half[i]<=0)throw new Exception("empty model bounds");mins[i]=center[i]-half[i];maxs[i]=center[i]+half[i];}
        uint entitySlot=r.Word(0x07817bb0);if(entitySlot>=256)throw new Exception("entity slot invalid");
        uint entities=r.Word(0x01a19510+entitySlot*4)^0x2168f672,entity=entities+19*0x238;
        Console.WriteLine(new JavaScriptSerializer().Serialize(Obj("schema_version",1,"read_only",true,
            "client_sha256","e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b",
            "ncs_crc",r.Word(0x02df7d90),"model_name",name,"model",id,"bounds_min",mins,"bounds_max",maxs,
            "entity19_model",r.Word(entity+0xf0+0x98),"entity19_origin",r.EntityOrigin(entity+0xf0),
            "entity19_scale",Floats(r.Read(entity+0xf0+0x54,4),0,1)[0],
            "entity19_dobj_slot",BitConverter.ToUInt16(r.Read(0x02ef4858+19*2,2),0))));
    }
    float[] EntityOrigin(uint state) {
        // 0x412ce0: protected trBase float bits at ES+0x18/1c/20 are XYZ.
        float[] origin=new float[3];
        for(uint axis=0;axis<3;axis++) {
            uint at=state+0x18+axis*4;
            uint bits=~Word(0x00cda23c+axis*4)^Word(at)^at;
            origin[axis]=BitConverter.ToSingle(BitConverter.GetBytes(bits),0);
            if(Single.IsNaN(origin[axis])||Single.IsInfinity(origin[axis]))throw new Exception("nonfinite entity origin");
        }
        return origin;
    }
    public static void ExportAppearanceLoad(IntPtr process) {
        var r=new ClientAssets(process);var frames=new List<object>();var clock=System.Diagnostics.Stopwatch.StartNew();
        while(clock.ElapsedMilliseconds<25000) {
            uint state=r.Word(0x01a2e7ec);var actors=new List<object>();
            uint time=0;
            if(state==10) {
                uint slot=r.Word(0x07817bb0),cgSlot=r.Word(0x07817ba8);
                if(slot>=256||cgSlot>=256)throw new Exception("invalid appearance context");
                uint entities=r.Word(0x01a19510+slot*4)^0x2168f672;
                uint cg=r.Word(0x01a19510+cgSlot*4)^0xfbb2c97a;time=r.Word(cg+0xae4c8);
                for(uint id=0;id<=26;id++) {
                    uint entity=entities+id*0x238;
                    if((r.Word(entity+0x204)&1)==0)continue;
                    uint type=r.Word(entity+0xf4);if(type!=1&&type!=2)continue;
                    uint dobjSlot=BitConverter.ToUInt16(r.Read(0x02ef4858+id*2,2),0);var models=new List<string>();
                    if(dobjSlot>0&&dobjSlot<2048) {
                        uint dobj=0x02e8e850+dobjSlot*0xcc,list=r.Word(dobj+0xc0);
                        int count=Count(r.Read(dobj+9,1)[0],32);
                        for(uint part=0;part<count;part++)models.Add(r.Name(r.Word(r.Word(list+part*4))));
                    }
                    uint client=r.Word(entity+0xf0+0x9c);
                    actors.Add(Obj("entity",id,"type",type,"client",client,"models",models,
                        "character",r.CharacterInfo(cg,client)));
                }
            }
            frames.Add(Obj("elapsed",clock.ElapsedMilliseconds,"time",time,"connection_state",state,"actors",actors));
            System.Threading.Thread.Sleep(50);
        }
        var serializer=new JavaScriptSerializer();serializer.MaxJsonLength=8*1024*1024;
        Console.WriteLine(serializer.Serialize(Obj("schema_version",1,"read_only",true,
            "client_sha256","e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b","frames",frames)));
    }
    public static void ExportOutfits(IntPtr process) {
        var r=new ClientAssets(process);var items=new List<object>();var variants=new List<object>();
        var marketplace=(Dictionary<string,object>)r.Table("mp/coMarketplaceItems.csv");
        var cells=(List<string>)marketplace["cells_base64"];int columns=(int)marketplace["columns"];
        if(columns!=12)throw new Exception("outfit marketplace schema differs");
        for(int row=0;row<cells.Count/columns;row++) {
            string type=Encoding.ASCII.GetString(Convert.FromBase64String(cells[row*columns+2]));
            if(type!="cha"&&type!="cat")continue;
            ushort id=UInt16.Parse(Encoding.ASCII.GetString(Convert.FromBase64String(cells[row*columns])));
            string name=Encoding.ASCII.GetString(Convert.FromBase64String(cells[row*columns+1]));
            uint at=r.ItemRecord(id);
            if(at==0||!String.Equals(r.Name(r.Word(at+12)),name,StringComparison.OrdinalIgnoreCase)) {
                items.Add(Obj("item_id",id,"name",name,"error","outfit registry identity mismatch"));continue;
            }
            var item=Obj("item_id",id,"name",name,"type",type,"kind",r.Read(at+6,1)[0],
                "detail_index",BitConverter.ToUInt16(r.Read(at+2,2),0));
            if(type=="cha") {
                uint detail=BitConverter.ToUInt16(r.Read(at+2,2),0);
                if(detail==0||detail>=r.Word(0x02dbba48))throw new Exception("character package index outside pool");
                byte[] record=r.Read(r.Word(0x02dbba44)+detail*40,40);
                item["package_base64"]=Convert.ToBase64String(record);
                if(U(record,32)>1000)throw new Exception("character package members exceed1000: "+id+" count="+U(record,32));
                int count=(int)U(record,32);var members=new List<ushort>();
                byte[] memberBytes=count==0?new byte[0]:r.Read(U(record,36),count*2);
                for(int i=0;i<count;i++)members.Add(BitConverter.ToUInt16(memberBytes,i*2));
                item["members"]=members;
            }
            items.Add(item);
        }
        // 0x40d320 builds the sorted, retained sixteen-byte variant records.
        int total=BitConverter.ToUInt16(r.Read(0x017db2a4,2),0);
        if(total>3584)throw new Exception("character variant budget exceeded");
        byte[] entries=r.Read(0x017cd290,total*16),groups=r.Read(0x017e02e4,11*4);
        for(int group=0;group<11;group++) {
            int start=BitConverter.ToUInt16(groups,group*4),count=BitConverter.ToUInt16(groups,group*4+2);
            if(start+count>total)throw new Exception("character variant span invalid");
            string name=r.Name(r.Word(0x00ce410c+(uint)group*4));
            for(int i=start;i<start+count;i++) {
                int at=i*16;
                variants.Add(Obj("group",name,"index",BitConverter.ToUInt16(entries,at),
                    "selectors_base64",Convert.ToBase64String(entries,at+2,6),
                    "model",r.Name(U(entries,at+8)),"viewmodel",r.Name(U(entries,at+12))));
            }
        }
        var serializer=new JavaScriptSerializer();serializer.MaxJsonLength=8*1024*1024;
        Console.WriteLine(serializer.Serialize(Obj("schema_version",1,"read_only",true,
            "client_sha256","e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b",
            "ncs_crc",r.Word(0x02df7d90),"items",items,"variants",variants,
            "variant_items_base64",Convert.ToBase64String(r.Read(0x017db2a8,2560*8)),
            "variant_groups_base64",Convert.ToBase64String(r.Read(0x017cd1bc,11*4)),
            "packing_fields_base64",Convert.ToBase64String(r.Read(0x017cd0c0,21*12)),
            "packing_bits_base64",Convert.ToBase64String(r.Read(0x017e02b4,11*4)),
            "gender_selectors_base64",Convert.ToBase64String(r.Read(0x017e02a8,3*4)),
            "faction_selectors_base64",Convert.ToBase64String(r.Read(0x017e1758,2*4)),
            "skin_selectors_base64",Convert.ToBase64String(r.Read(0x017db294,4*4)),
            "selector_hashes_base64",Convert.ToBase64String(r.Read(0x017cd1e8,Count(r.Word(0x017db290),128)*4)))));
    }
    public static void ExportGrounding(IntPtr process) {
        var r=new ClientAssets(process);var frames=new List<object>();
        if(r.Word(0x01a2e7ec)!=10)throw new Exception("load the map before grounding capture");
        uint crc=r.Word(0x02df7d90);var clock=System.Diagnostics.Stopwatch.StartNew();
        for(int sample=0;sample<200 && clock.ElapsedMilliseconds<10000;sample++) {
            if(r.Word(0x01a2e7ec)!=10||r.Word(0x02df7d90)!=crc)break;
            uint cgSlot=r.Word(0x07817ba8),entitySlot=r.Word(0x07817bb0);
            if(cgSlot>=256||entitySlot>=256)throw new Exception("invalid grounding context");
            uint cg=r.Word(0x01a19510+cgSlot*4)^0xfbb2c97a;
            uint entities=r.Word(0x01a19510+entitySlot*4)^0x2168f672;
            var clients=new List<object>();
            for(uint id=1;id<12;id++) {
                uint at=entities+id*0x238;
                if((r.Word(at+0x204)&1)==0||r.Word(at+0xf4)!=1)continue;
                uint dobj=BitConverter.ToUInt16(r.Read(0x02ef4858+id*2,2),0);
                object pose=r.GroundPose(at,dobj);
                uint ci=cg+0x15dea8+id*0x3b8;
                clients.Add(Obj("client",id,"entity_origin",r.EntityOrigin(at+0xf0),
                    "legs_animation",r.Word(at+0xf0+0xd4),"ground_pose",pose,
                    "torso_animation",r.Word(at+0xf0+0xd8),"torso_lerp_animation",r.Word(ci+0x1b0),
                    "torso_lerp_rate",BitConverter.ToSingle(r.Read(ci+0x1d4,4),0)));
            }
            frames.Add(Obj("time",r.Word(cg+0xae4c8),"clients",clients));
            System.Threading.Thread.Sleep(50);
        }
        var serializer=new JavaScriptSerializer();serializer.MaxJsonLength=8*1024*1024;
        Console.WriteLine(serializer.Serialize(Obj("schema_version",1,"read_only",true,
            "client_sha256","e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b",
            "ncs_crc",crc,"frames",frames)));
    }
    object GroundPose(uint entity,uint dobjSlot) {
        // Read existing render data only; off-screen actors may have no current
        // skeleton. Never request bone evaluation or treat a stale pose as proof.
        try {
            if(dobjSlot==0||dobjSlot>=2048||(Word(entity+0x204)&1)==0)return null;
            uint cgSlot=Word(0x07817ba8);if(cgSlot>=256)return null;
            uint cg=Word(0x01a19510+cgSlot*4)^0xfbb2c97a,time=Word(cg+0xae4c8);
            uint dobj=0x02e8e850+dobjSlot*0xcc,pose=Word(dobj+0x98);
            if(pose<0x10000)return null;
            // 0x4f9750: only +0x74 part bits in the current skeleton frame
            // identify calculated bones. Off-screen matrices can contain old data.
            uint skeletonFrame=Word(0x078b16c8);
            if(Word(dobj+0x94)!=skeletonFrame)return null;
            byte[] calculated=Read(dobj+0x74,32);
            if((U(calculated,0)&0x80000000)==0)return null;
            uint model=Word(Word(dobj+0xc0));byte[] header=Read(model,0x44);
            int count=header[4];if(count<1||count>253)return null;
            byte[] ids=Read(U(header,0x2c),count*2),matrices=Read(pose,count*32);
            uint pool=Word(0x02f37164);
            float[] reference=Floats(Read(cg+0xae538,12),0,3),snapshot=EntityOrigin(entity+0xf0);
            float[] root=new float[3];double error=0;
            for(int a=0;a<3;a++){root[a]=BitConverter.ToSingle(matrices,16+a*4)+reference[a];double delta=(double)root[a]-snapshot[a];error+=delta*delta;}
            var feet=new List<object>();
            bool plausible=error<32*32;
            for(int b=0;b<count;b++) {
                string name=Name(pool+(uint)BitConverter.ToUInt16(ids,b*2)*20+4);
                if(!name.StartsWith("j_ankle_")&&!name.StartsWith("j_ball_"))continue;
                if((U(calculated,(b>>5)*4)&(0x80000000u>>(b&31)))==0)continue;
                var point=new float[3];for(int a=0;a<3;a++)point[a]=BitConverter.ToSingle(matrices,b*32+16+a*4)+reference[a];
                double norm=0,range=0;
                for(int a=0;a<4;a++){double q=BitConverter.ToSingle(matrices,b*32+a*4);norm+=q*q;}
                for(int a=0;a<3;a++){double delta=(double)point[a]-root[a];range+=delta*delta;}
                plausible &= norm>0.000001 && norm<4 && range<100*100;
                feet.Add(Obj("name",name,"origin",point));
            }
            bool stable=time==Word(cg+0xae4c8)&&pose==Word(dobj+0x98)
                &&skeletonFrame==Word(dobj+0x94)&&skeletonFrame==Word(0x078b16c8);
            // Root/snapshot distance includes normal interpolation delay.
            return Obj("time",time,"stable_read",stable,"plausible",plausible,"root_origin",root,
                "root_snapshot_distance_squared",Double.IsNaN(error)||Double.IsInfinity(error)?(object)null:error,"feet",feet);
        } catch(Exception e) {return Obj("error",e.Message);}
    }
    public static void ExportBulletImpacts(IntPtr process) {
        var r=new ClientAssets(process);
        if(r.Word(0x01a2e7ec)!=10)throw new Exception("load the map before bullet impact capture");
        uint slot=r.Word(0x07817bb0);if(slot>=256)throw new Exception("entity slot invalid");
        uint entities=r.Word(0x01a19510+slot*4)^0x2168f672;
        byte[] states=r.Read(entities,2046*0x238);var impacts=new List<object>();
        for(int id=20;id<2046;id++) {
            int next=id*0x238+0xf0;
            uint type=U(states,next+4);if(type!=96&&type!=109)continue;
            impacts.Add(Obj("entity",id,"type",type,"normal",U(states,next+0xa8),"surface",U(states,next+0x90),
                "shooter",U(states,next+0x80),"flags",U(states,next+0xdc),"weapon",U(states,next+0xd0),
                "hit_entity",U(states,next+0x88),"processed",U(states,id*0x238+0x20c),"origin",r.EntityOrigin(entities+(uint)next)));
        }
        // 0x561220 uses the registered WeaponDef's+d0c row, not a fixed M4.
        int weaponCount=Count(r.Word(0x07817b88),899);var weapons=new List<object>();var rows=new HashSet<uint>();
        foreach(uint id in new uint[]{95,352,353,354,355})if(id<=weaponCount) {
            uint def=r.Word(0x017f5a40+id*4);uint row=r.Word(def+0xd0c);
            if(row<1||row>9)continue;
            uint selected=new uint[]{0,0,2,8,4,5,7,10,12,13}[row];rows.Add(selected);
            weapons.Add(Obj("weapon",id,"name",r.Name(r.Word(def)),"impact_row",row));
        }
        uint table=r.Word(0x01a1f920);var fx=new List<object>();
        if(table>=0x10000) {
            uint pool=r.Word(table+4);
            foreach(uint row in rows)foreach(uint surface in new uint[]{0,3,5,6,11,12,13,16,17,18,21,23,25}) {
                uint pointer=r.Word(pool+(row*35+surface)*4);
                var types=new List<uint>();
                if(pointer!=0) {
                    byte[] definition=r.Read(pointer,0x20);int count=Count(U(definition,0x10)+U(definition,0x14)+U(definition,0x18),128);
                    byte[] elements=r.Read(U(definition,0x1c),count*0x104);
                    for(int element=0;element<count;element++)types.Add(elements[element*0x104+0xb0]);
                }
                fx.Add(Obj("row",row,"surface",surface,"loaded",pointer!=0,"name",pointer==0?null:r.Name(r.Word(pointer)),"element_types",types));
            }
        }
        Console.WriteLine(new JavaScriptSerializer().Serialize(Obj("schema_version",1,"read_only",true,
            "client_sha256","e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b",
            "impacts",impacts,"weapons",weapons,"impact_fx",fx,
            "mark_enabled_value",r.Word(r.Word(0x02921a48)+0xc),
            "mark_enable_dvar",r.Name(r.Word(r.Word(0x02921a48))),
            "mark_disabled_byte",r.Read(0x02928a56,1)[0],"mark_suppressed_word",r.Word(0x02927a18))));
    }
    static float[] Floats(byte[] bytes,int offset,int count) {
        var values=new float[count];for(int i=0;i<count;i++){values[i]=BitConverter.ToSingle(bytes,offset+i*4);if(Single.IsNaN(values[i])||Single.IsInfinity(values[i]))throw new Exception("nonfinite geometry");}return values;
    }
    Dictionary<string,List<object>> attachmentItems;
    object AttachmentItems(string name,uint wire) {
        // Resolve marketplace IDs through the target's compact item registry,
        // not the display slot order in coWeapSlotDefs.csv.
        if(attachmentItems==null) {
            attachmentItems=new Dictionary<string,List<object>>(StringComparer.Ordinal);
            int count=Count(Word(0x02db8ac8),65535),slots=Count(Word(0x02db8ad0),65536);
            byte[] records=Read(Word(0x02db8ac4),count*0x28);
            byte[] indices=Read(Word(0x02db8acc),slots*2),groups=Read(0x02db8ad4,66*2);
            for(uint id=1;id<=65535;id++) {
                uint slot=(BitConverter.ToUInt16(groups,(int)(id/1000)*2)+id%1000)&65535;
                if(slot>=slots)continue;
                int index=BitConverter.ToUInt16(indices,(int)slot*2);
                if(index==0||index>=count)continue;
                string itemName=Name(U(records,index*0x28+12));
                if(itemName==null)continue;
                List<object> list;
                if(!attachmentItems.TryGetValue(itemName,out list)) {
                    list=new List<object>();attachmentItems.Add(itemName,list);
                }
                list.Add(Obj("item_id",id,"kind",records[index*0x28+6],"name",itemName));
            }
        }
        var items=new List<object>();List<object> matches;
        if(name!=null && attachmentItems.TryGetValue(name,out matches))foreach(Dictionary<string,object> item in matches)
            items.Add(Obj("item_id",item["item_id"],"kind",item["kind"],"name",name,"wire_index",wire));
        return items;
    }
    public static void ExportWeaponModels(IntPtr process) {
        var r=new ClientAssets(process);var names=new SortedSet<string>();var itemNames=new SortedSet<string>();
        if(r.Word(0x01a2e7ec)!=10)throw new Exception("load the training map before capturing weapon models");
        byte[] entries=r.Read(0x01fc4e28,100000*28),heads=r.Read(0x01bc9f20,80000*4);
        var seen=new HashSet<uint>();var modelHeaders=new HashSet<uint>();var animHeaders=new Dictionary<uint,string>();
        for(int bucket=0;bucket<80000;bucket++) {
            uint index=U(heads,bucket*4);int chain=0;
            while(index!=0) {
                if(index>=100000||!seen.Add(index)||++chain>128)throw new Exception("invalid model asset chain");
                int offset=(int)index*28;uint kind=U(entries,offset),header=U(entries,offset+8);
                if(kind==4)modelHeaders.Add(header);
                if(kind==2)animHeaders[header]=r.Name(r.Word(header));
                if(kind==37) {
                    string name=r.Name(r.Word(header));
                    if(name.StartsWith("mp/coWeapModels/",StringComparison.OrdinalIgnoreCase)) {
                        if(names.Count>=1024)throw new Exception("model table name budget");
                        names.Add(name);
                    }
                    if(name.StartsWith("mp/coWeapItems",StringComparison.OrdinalIgnoreCase)) {
                        if(itemNames.Count>=256)throw new Exception("item table name budget");
                        itemNames.Add(name);
                    }
                }
                index=U(entries,offset+20);
            }
        }
        var tables=new Dictionary<string,object>();
        foreach(string name in names)if(name.IndexOf("m4",StringComparison.OrdinalIgnoreCase)>=0) {
            if(tables.Count>=32)throw new Exception("M4 table budget");
            try{tables[name]=r.Table(name);}catch(Exception e){tables[name]=Obj("error",e.Message);}
        }
        foreach(string name in new string[]{"mp/coWeapItems/ast.csv","mp/coCharItemStatsTable.csv","mp/coWeapSlotDefs.csv",
            "mp/coWeapItemswea.csv","mp/coWeapItems/gri.csv","mp/coWeapItems/sto.csv","mp/coWeapItems/mag.csv","mp/coWeapItems/iro.csv","mp/coWeapItems/rai.csv","mp/coWeapItems/muz.csv"}) {
            try{tables[name]=r.Table(name);}catch(Exception e){tables[name]=Obj("error",e.Message);}
        }
        var defaults=new List<object>();byte[] attachmentNames=r.Read(0x017e5848,16384*4);
        for(uint id=1;id<16384;id++) {
            uint at=U(attachmentNames,(int)id*4);if(at<0x10000 || at>=0xfff00000)continue;
            string name=r.Name(at);
            if(name.EndsWith("_default",StringComparison.Ordinal)||name.EndsWith("_none",StringComparison.Ordinal))
                defaults.Add(Obj("index",id,"name",name));
        }
        var weapons=new List<object>();var compatible=new List<object>();uint baseIndex=0;int count=Count(r.Word(0x07817b88),899);
        for(uint i=1;i<=count;i++) {
            uint def=r.Word(0x017f5a40+i*4);string name=r.Name(r.Word(def));
            if(name=="m4_mp") {
                baseIndex=i;
                // 0x42e3b0: per-base compatibility count and (attachment,
                // attribute) ushort pairs. Resolve names through its registry.
                int n=Count(r.Read(0x017f5848+i,1)[0],64);
                byte[] pairs=r.Read(0x017f8248+i*0x100,n*4);
                for(int part=0;part<n;part++) {
                    uint id=BitConverter.ToUInt16(pairs,part*4);
                    if(id>=16384)throw new Exception("attachment ID budget");
                    uint attachment=r.Word(0x017e5848+id*4);
                    string partName=attachment==0?null:r.Name(attachment);
                    compatible.Add(Obj("index",id,"name",partName,
                        "attribute_index",BitConverter.ToUInt16(pairs,part*4+2),
                        "items",r.AttachmentItems(partName,id)));
                }
            }
            if(baseIndex==0||!name.StartsWith("** "+baseIndex+" ",StringComparison.Ordinal))continue;
            // 0x42e3b0 embeds WeaponDef at composition+0xa0; 0x42dc10 writes
            // counts+0x91/+0x92 and model arrays+0xf68/+0xfac.
            uint record=checked(def-0xa0);
            if(r.Name(record)!=name)throw new Exception("composed M4 record identity mismatch");
            int parts=Count(r.Read(record+0x92,1)[0],17);
            var models=new List<object>();
            for(uint part=0;part<parts;part++) {
                uint world=r.Word(record+0xf68+part*4),view=r.Word(record+0xfac+part*4);
                models.Add(Obj("part",part,
                    "world",world!=0&&modelHeaders.Contains(world)?r.Name(r.Word(world)):null,
                    "view",view!=0&&modelHeaders.Contains(view)?r.Name(r.Word(view)):null));
            }
            weapons.Add(Obj("index",i,"name",name,"attachment_count",r.Read(record+0x91,1)[0],"models",models));
        }
        var firstPerson=new List<object>();
        uint slot=r.Word(0x07817bb8);
        if(slot>=256)throw new Exception("first-person pointer slot outside budget");
        uint views=r.Word(0x01a19510+slot*4)^0x39641402;
        for(uint hand=0;hand<2;hand++) {
            uint dobj=r.Word(views+hand*24);if(dobj==0)continue;
            // 0x644860 sets DObj+9=count and +0xc0=model pointer array;
            // 0x59e610 iterates that same indirect array. XModel+4 is bone
            // count, +0x2c points to script-string IDs used by0x644860.
            int modelCount=Count(r.Read(dobj+9,1)[0],32);
            uint array=r.Word(dobj+0xc0);var modelList=new List<object>();var combinedBones=new List<string>();
            for(uint index=0;index<modelCount;index++) {
                uint model=r.Word(array+index*4);
                if(!modelHeaders.Contains(model))throw new Exception("DObj references an unregistered model");
                int bones=Count(r.Read(model+4,1)[0],253);
                byte[] ids=r.Read(r.Word(model+0x2c),bones*2);
                var boneNames=new List<string>();uint scriptStrings=r.Word(0x02f37164);
                for(int b=0;b<bones;b++)boneNames.Add(r.Name(scriptStrings+4+(uint)BitConverter.ToUInt16(ids,b*2)*20));
                combinedBones.AddRange(boneNames);
                modelList.Add(Obj("name",r.Name(r.Word(model)),"bone_names",boneNames));
            }
            uint tree=r.Word(dobj);var animSlots=new List<object>();var animStates=new List<object>();
            if(tree!=0) {
                uint anims=r.Word(tree);
                // Bounded pointer matching identifies leaf assets without
                // interpreting unknown XAnim_s fields as arbitrary pointers.
                byte[] bank=r.Read(anims,512);
                for(int offset=0;offset<bank.Length;offset+=4) {
                    uint part=U(bank,offset);string partName;
                    if(!animHeaders.TryGetValue(part,out partName))continue;
                    // 0x649320 binds parts+0x30 script-string IDs, with the
                    // bone count at+0x1a (not the old version's+0x1b).
                    int boneCount=Count(r.Read(part+0x1a,1)[0],253);var bones=new List<string>();
                    if(boneCount>0) {
                        byte[] ids=r.Read(r.Word(part+0x30),boneCount*2);uint stringsAt=r.Word(0x02f37164);
                        for(int b=0;b<boneCount;b++)bones.Add(r.Name(stringsAt+4+(uint)BitConverter.ToUInt16(ids,b*2)*20));
                    }
                    animSlots.Add(Obj("bank_offset",offset,"name",partName,"bone_names",bones));
                }
                // 0x64b490 follows children+0xc and next+0xa in the56-byte
                // XAnimInfo pool; 0x64c880 owns its goal/weight/time fields.
                var pending=new Stack<uint>();var visited=new HashSet<uint>();
                pending.Push(BitConverter.ToUInt16(r.Read(tree+4,2),0));
                while(pending.Count>0) {
                    uint index=pending.Pop();if(index==0)continue;
                    if(!visited.Add(index)||visited.Count>256)throw new Exception("animation tree cycle/budget");
                    byte[] node=r.Read(checked(0x0374db08+index*0x38),0x38);
                    pending.Push(BitConverter.ToUInt16(node,0xa));pending.Push(BitConverter.ToUInt16(node,0xc));
                    uint binding=BitConverter.ToUInt16(node,0x12);var boundNames=new List<string>();
                    if(binding!=0) {
                        // 0x649320 interns eight mask words, one count byte,
                        // then one model-bone index per animation bone;0xfc
                        // means no matching bone. The blob is a script string.
                        uint at=r.Word(0x02f37164)+4+binding*20;
                        int mappedCount=Count(r.Read(at+32,1)[0],253);
                        byte[] mapping=r.Read(at+33,mappedCount);
                        foreach(byte mapped in mapping)boundNames.Add(mapped<combinedBones.Count?combinedBones[mapped]:null);
                    }
                    animStates.Add(Obj("animation_index",BitConverter.ToUInt16(node,0x10),
                        "time_bits",U(node,0x18),"goal_weight_bits",U(node,0x28),"weight_bits",U(node,0x2c),"rate_bits",U(node,0x30),
                        "bound_model_bones",boundNames));
                }
            }
            firstPerson.Add(Obj("hand",hand,"models",modelList,"animation_leaves",animSlots,"animation_states",animStates));
        }
        var candidateHands=new List<object>();
        foreach(string name in new string[]{"viewhands_player_tf141","viewhands_tf141"}) {
            uint entry;uint model=r.Find(4,name,out entry);if(model==0)continue;
            int countBones=Count(r.Read(model+4,1)[0],253);var bones=new List<string>();
            byte[] ids=r.Read(r.Word(model+0x2c),countBones*2);uint baseStrings=r.Word(0x02f37164);
            for(int b=0;b<countBones;b++)bones.Add(r.Name(baseStrings+4+(uint)BitConverter.ToUInt16(ids,b*2)*20));
            candidateHands.Add(Obj("name",name,"bone_names",bones));
        }
        var output=Obj("schema_version",1,"read_only",true,
            "client_sha256","e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b",
            "connection_state",r.Word(0x01a2e7ec),"ncs_crc",r.Word(0x02df7d90),"model_table_names",names,"item_table_names",itemNames,"tables",tables,"weapons",weapons,"compatible_attachments",compatible,"default_attachment_names",defaults,"m4_default_parts",r.M4DefaultParts(attachmentNames),"first_person",firstPerson,"candidate_hands",candidateHands);
        var serializer=new JavaScriptSerializer();serializer.MaxJsonLength=8*1024*1024;
        Console.WriteLine(serializer.Serialize(output));
    }
}
