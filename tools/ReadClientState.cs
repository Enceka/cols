// Read-only Win32 probe for the marked research client. It does not attach a
// debugger, inject a thread, patch memory, or enumerate account/session keys.
// Build with the private Wine prefix's csc.exe; see docs/ONBOARDING_PROGRESS.md.
using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;

class ReadClientState {
    [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)]
    struct Entry {
        public uint size, usage, pid;
        public IntPtr heap;
        public uint module, threads, parent;
        public int priority;
        public uint flags;
        [MarshalAs(UnmanagedType.ByValTStr,SizeConst=260)] public string name;
    }
    [DllImport("kernel32.dll",SetLastError=true)] static extern IntPtr CreateToolhelp32Snapshot(uint flags,uint pid);
    [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern bool Process32FirstW(IntPtr snapshot,ref Entry entry);
    [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern bool Process32NextW(IntPtr snapshot,ref Entry entry);
    [DllImport("kernel32.dll",SetLastError=true)] static extern IntPtr OpenProcess(uint access,bool inherit,uint pid);
    [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern bool QueryFullProcessImageNameW(IntPtr process,uint flags,StringBuilder path,ref uint size);
    [DllImport("kernel32.dll",SetLastError=true)] static extern bool ReadProcessMemory(IntPtr process,IntPtr address,byte[] bytes,UIntPtr count,out UIntPtr read);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
    static long totalRead;
    internal static byte[] ReadBytes(IntPtr process,uint address,int size) {
        if(address<0x10000 || size<0 || size>4*1024*1024 || (ulong)address+(uint)size>0x100000000UL ||
           (totalRead+=size)>64*1024*1024) throw new Exception("read budget or address limit");
        byte[] bytes=new byte[size]; UIntPtr read;
        if(!ReadProcessMemory(process,new IntPtr(address),bytes,new UIntPtr((uint)size),out read)||read.ToUInt64()!=(uint)size)
            throw new Exception("read failed at 0x"+address.ToString("x")+": "+Marshal.GetLastWin32Error());
        return bytes;
    }
    static uint Read(IntPtr process,uint address,int size) { byte[] bytes=ReadBytes(process,address,size);return size==1?bytes[0]:size==2?BitConverter.ToUInt16(bytes,0):BitConverter.ToUInt32(bytes,0); }
    static void Field(IntPtr process,string name,uint address,int size) {
        Console.WriteLine("\""+name+"\":"+Read(process,address,size)+",");
    }
    static int Main(string[] args) {
        try {
            bool assets=args.Length==1 && args[0]=="--assets";
            bool profiles=args.Length==1 && args[0]=="--profiles";
            bool netConstants=args.Length==1 && args[0]=="--net-constants";
            if(args.Length!=0 && !assets && !profiles && !netConstants) throw new Exception("only fixed extraction modes are supported");
            IntPtr snapshot=CreateToolhelp32Snapshot(2,0); uint found=0;
            if(snapshot==new IntPtr(-1)) throw new Exception("process snapshot failed");
            try {
                Entry e=new Entry();e.size=(uint)Marshal.SizeOf(typeof(Entry));
                bool next=Process32FirstW(snapshot,ref e);
                while(next) {
                    if(String.Equals(e.name,"codoMP_client_shipRetail.exe",StringComparison.OrdinalIgnoreCase)) {
                        if(found!=0) throw new Exception("more than one client in prefix");
                        found=e.pid;
                    }
                    next=Process32NextW(snapshot,ref e);
                }
            } finally {CloseHandle(snapshot);}
            if(found==0) throw new Exception("research client not running");
            IntPtr process=OpenProcess(0x1010,false,found); // QUERY_LIMITED_INFORMATION | VM_READ
            if(process==IntPtr.Zero) throw new Exception("open process failed");
            try {
                StringBuilder path=new StringBuilder(4096);uint size=4096;
                if(!QueryFullProcessImageNameW(process,0,path,ref size)||
                   !File.Exists(Path.Combine(Path.GetDirectoryName(path.ToString()),".codol-research-copy.json")))
                    throw new Exception("target is not a marked research copy");
                using(var source=File.OpenRead(path.ToString())) using(var sha=SHA256.Create()) {
                    string hash=BitConverter.ToString(sha.ComputeHash(source)).Replace("-","").ToLowerInvariant();
                    if(hash!="e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b")
                        throw new Exception("target executable hash differs from the recovered version");
                }
                if(assets) { ClientAssets.Export(process);return 0; }
                if(profiles) { ClientAssets.ExportProfiles(process);return 0; }
                if(netConstants) { ClientAssets.ExportNetConstants(process);return 0; }
                Console.WriteLine("{");
                Field(process,"connection_state",0x01a2e7ec,4);
                Field(process,"net_constants_initialized",0x02df9934,4);
                Field(process,"net_constants_crc",0x02df7d90,4);
                uint connection=Read(process,0x01a2a538,4)^0xd362501f;
                if(connection>=0x10000 && connection<0x7ff00000) {
                    Field(process,"server_message_sequence",checked(connection+0x40148),4);
                    Field(process,"client_command_ack",checked(connection+0x144),4);
                    Field(process,"server_command_sequence",checked(connection+0x4014c),4);
                }
                Field(process,"config_version",0x078a06bc,4);
                Field(process,"playlist_version",0x031fe614,4);
                Field(process,"server_time",0x0c9e34e0,4);
                Field(process,"catalogue_state",0x07817dd8,4);
                Field(process,"catalogue_status_bits",0x07817df8,4);
                Field(process,"catalogue_count",0x07817de0,4);
                Field(process,"market_initialization_state",0x0783f86c,4);
                Field(process,"coupon_query_completed",0x07817c07,1);
                Field(process,"external_currency",0x0ba0b2e8,1);
                Field(process,"external_balance",0x0ba0b2ec,4);
                Field(process,"marketplace_currency",0x0ba0b2c8,1);
                Field(process,"marketplace_balance",0x0ba0b2cc,4);
                uint balanceCount=Read(process,0x07bdc728,4);
                Console.WriteLine("\"player_balance_count\":"+balanceCount+",");
                if(balanceCount<=10) for(uint i=0;i<balanceCount;i++) {
                    uint entry=0x07bdc730+i*0x40;
                    Field(process,"player_currency_"+i,entry+0x30,1);
                    Field(process,"player_amount_"+i,entry+0x34,4);
                }
                Field(process,"frontend_state",0x078b06c8,4);
                uint roomController=Read(process,0x078b0708,4);
                if(roomController>=0x10000) {
                    uint roomVtable=Read(process,roomController,4);
                    Console.WriteLine("\"room_controller\":"+roomController+",");
                    Console.WriteLine("\"room_vtable\":"+roomVtable+",");
                    Field(process,"room_create_method",checked(roomVtable+4),4);
                }
                Field(process,"cqb_training_flag",0x0c9670b5,2);
                Field(process,"prestige_rank_unlock",0x0c966f9d,2);
                Field(process,"onboard_character_id",0x0c95e386,4);
                uint inventoryCount=Read(process,0x07bdc9b0,4);
                Console.WriteLine("\"inventory_result_count\":"+inventoryCount+",");
                if(inventoryCount>0 && inventoryCount<=128) {
                    uint entries=Read(process,0x07bdc9b4,4);
                    Field(process,"inventory_first_item_id",checked(entries+0x30),4);
                    Field(process,"inventory_first_quantity",checked(entries+0x34),4);
                }
                uint ownedCount=Read(process,0x07817ddc,4);
                Console.WriteLine("\"active_inventory_count\":"+ownedCount+",");
                if(ownedCount<=128) {
                    var ids=new StringBuilder();
                    for(uint i=0;i<ownedCount;i++) {
                        if(i>0)ids.Append(',');
                        ids.Append(Read(process,checked(0x07bec478+i*0xa8),4));
                    }
                    Console.WriteLine("\"active_inventory_ids\":["+ids+"],");
                }
                Console.WriteLine("\"read_only\":true}");
            } finally {CloseHandle(process);}
            return 0;
        } catch(Exception e) {Console.Error.WriteLine(e.Message);return 1;}
    }
}
