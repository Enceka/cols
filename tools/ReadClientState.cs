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
            bool mapEntities=args.Length==1 && args[0]=="--map-entities";
            bool bootstrapWeapon=args.Length==1 && args[0]=="--bootstrap-weapon";
            if(args.Length!=0 && !assets && !profiles && !netConstants && !mapEntities && !bootstrapWeapon) throw new Exception("only fixed extraction modes are supported");
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
                if(mapEntities) { ClientAssets.ExportMapEntities(process);return 0; }
                if(bootstrapWeapon) { ClientAssets.ExportBootstrapWeapon(process);return 0; }
                Console.WriteLine("{");
                Field(process,"connection_state",0x01a2e7ec,4);
                Field(process,"client_realtime",0x078b0a58,4);
                Field(process,"client_input_catchers",0x01a2e7e4,4);
                // 0x45f120 divides by this value; it is a time interval, not
                // a boolean mouse-enable flag (0x45f260 also tests nonzero).
                Field(process,"mouse_interval_msec",0x01a2e7ac,4);
                Field(process,"input_frame_msec",0x078b0a50,4);
                Field(process,"mouse_sensitivity_bits",checked(Read(process,0x01a2a540,4)+12),4);
                Field(process,"mouse_yaw_scale_bits",checked(Read(process,0x01a2a560,4)+12),4);
                Field(process,"mouse_pitch_scale_bits",checked(Read(process,0x01a2a568,4)+12),4);
                Field(process,"frame_scale_bits",0x00cdcd3c,4);
                Field(process,"script_time_scale_bits",0x02e8cf88,4);
                uint fixedTime=Read(process,0x02e8a98c,4);
                if(fixedTime>=0x10000) Field(process,"fixed_frame_time",checked(fixedTime+12),4);
                uint maxPackets=Read(process,0x01a2e7cc,4);
                if(maxPackets>=0x10000) Field(process,"client_max_packets",checked(maxPackets+12),4);
                Field(process,"native_server_id",0x0796f2d8,4);
                Field(process,"full_snapshot_valid",0x0799964c,4);
                Field(process,"full_snapshot_flags",0x07999650,4);
                Field(process,"full_snapshot_server_time",0x07999654,4);
                Field(process,"full_snapshot_message",0x07999658,4);
                Field(process,"full_snapshot_command_time",0x07996128,4);
                Field(process,"full_snapshot_health",0x079962b8,4);
                Field(process,"full_snapshot_max_health",0x079962c0,4);
                Field(process,"full_snapshot_other_flags",0x07996138,4);
                // 0x46d673 caches a 0x3564 snapshot at 0x07996128.
                // 0x5a4230: clip entries at PS+0x3dc, reserve at PS+0x364.
                Field(process,"full_snapshot_clip_key_0",0x07996128+0x3dc,4);
                Field(process,"full_snapshot_clip_ammo_0",0x07996128+0x3e0,4);
                Field(process,"full_snapshot_reserve_key_0",0x07996128+0x364,4);
                Field(process,"full_snapshot_reserve_ammo_0",0x07996128+0x368,4);
                // 0x46c1c0 increments snapshot+0x3540; cached entity records
                // start at 0x07999690 and eType is their second word.
                Field(process,"full_snapshot_entity_count",0x07996128+0x3540,4);
                Field(process,"full_entity_0_type",0x07999690+4,4);
                Field(process,"full_snapshot_origin_x_bits",0x07996144,4);
                Field(process,"full_snapshot_origin_y_bits",0x07996148,4);
                Field(process,"full_snapshot_origin_z_bits",0x0799614c,4);
                Field(process,"full_snapshot_view_height_bits",0x07996284,4);
                Field(process,"full_snapshot_view_locked_entity",0x079962dc,4);
                Field(process,"full_snapshot_view_linked_entity",0x07996318,4);
                Field(process,"net_constants_initialized",0x02df9934,4);
                Field(process,"net_constants_crc",0x02df7d90,4);
                uint connection=Read(process,0x01a2a538,4)^0xd362501f;
                if(connection>=0x10000 && connection<0x7ff00000) {
                    Field(process,"server_message_sequence",checked(connection+0x40148),4);
                    Field(process,"client_command_ack",checked(connection+0x144),4);
                    Field(process,"server_command_sequence",checked(connection+0x4014c),4);
                }
                uint active=Read(process,0x01a2e838,4)^0x6891d1c7;
                // 0x45f260 / 0x50b8b0 decode the local cg pointer this way;
                // cg starts with the predicted playerState.
                uint cgSlot=Read(process,0x07817ba8,4);
                Field(process,"cg_pointer_slot",0x07817ba8,4);
                if(cgSlot<256) {
                    uint cg=Read(process,checked(0x01a19510+cgSlot*4),4)^0xfbb2c97a;
                    if(cg>=0x10000 && cg<0xff000000) {
                        Field(process,"predicted_command_time",cg,4);
                        Field(process,"predicted_pm_type",checked(cg+4),4);
                        Field(process,"predicted_pm_flags",checked(cg+0xc),4);
                        Field(process,"predicted_other_flags",checked(cg+0x10),4);
                        Field(process,"predicted_view_pitch_bits",checked(cg+0x14c),4);
                        Field(process,"predicted_view_yaw_bits",checked(cg+0x150),4);
                        Field(process,"predicted_weapon_animation",checked(cg+0x26c),4);
                        Field(process,"cg_frame_msec",checked(cg+0xae4c4),4);
                        Field(process,"cg_time",checked(cg+0xae4c8),4);
                        // 0x451720 copies/interpolates these two CG snapshots.
                        for(uint slot=0;slot<2;slot++) {
                            uint snap=Read(process,checked(cg+0x3578+slot*4),4);
                            if(snap>=0x10000 && snap<0xff000000) {
                                Field(process,"cg_snapshot_"+slot+"_command_time",snap,4);
                                Field(process,"cg_snapshot_"+slot+"_pitch_bits",checked(snap+0x14c),4);
                                Field(process,"cg_snapshot_"+slot+"_yaw_bits",checked(snap+0x150),4);
                            }
                        }
                    }
                }
                if(active>=0x10000 && active<0x7ff00000) {
                    // 0x45f090 consumes two alternating raw-delta buckets.
                    Field(process,"mouse_raw_x_0",checked(active+0x35e4),4);
                    Field(process,"mouse_raw_x_1",checked(active+0x35e8),4);
                    Field(process,"mouse_raw_y_0",checked(active+0x35ec),4);
                    Field(process,"mouse_raw_y_1",checked(active+0x35f0),4);
                    Field(process,"mouse_raw_bucket",checked(active+0x35f4),4);
                    // 0x45f120 multiplies sensitivity by this game-side scale.
                    Field(process,"mouse_game_scale_bits",checked(active+0x3610),4);
                    // 0x50b8b0 only applies motion when this aim state is ready;
                    // the caller 0x45f577 explicitly selects local client0.
                    Field(process,"mouse_aim_state_ready",0x02c27640,1);
                    // CL_CreateCmd/MouseMove use active+0x3650/54 for live
                    // view angles; active+0xc is the current snapshot PS.
                    Field(process,"client_view_pitch_bits",checked(active+0x3650),4);
                    Field(process,"client_view_yaw_bits",checked(active+0x3654),4);
                    // 0x45f9be adds these offsets to live view angles before
                    // producing short angles in the 44-byte command ring.
                    Field(process,"client_angle_offset_pitch_bits",checked(active+0x361c),4);
                    Field(process,"client_angle_offset_yaw_bits",checked(active+0x3620),4);
                    uint cmd=Read(process,checked(active+0x625c),4);
                    Field(process,"last_local_cmd_pitch",checked(active+0x365c+(cmd&255)*44+8),4);
                    Field(process,"last_local_cmd_yaw",checked(active+0x365c+(cmd&255)*44+12),4);
                    Field(process,"current_pm_flags",checked(active+0x18),4);
                    Field(process,"current_clamp_pitch_bits",checked(active+0xc+0x174),4);
                    Field(process,"current_clamp_yaw_bits",checked(active+0xc+0x178),4);
                    Field(process,"client_command_number",checked(active+0x625c),4);
                    uint sent=Read(process,checked(connection+0x80158),4);
                    uint mask=Read(process,checked(active+0x8668),4);
                    uint history=Read(process,checked(active+0x867c),4)^0x96b9dc91;
                    if(history>=0x10000 && history<0x7ff00000 && mask<4096 && sent>0)
                        Field(process,"last_input_packet_time",checked(history+((sent-1)&mask)*12+8),4);
                    Field(process,"current_snapshot_valid",checked(active+0x3530),4);
                    Field(process,"current_snapshot_flags",checked(active+0x3534),4);
                    Field(process,"current_snapshot_server_time",checked(active+0x3538),4);
                    Field(process,"current_snapshot_message",checked(active+0x353c),4);
                    // Current snapshot is active+0xc (0x46d686); PS table
                    // indices19/18 store pitch/yaw at0x14c/0x150, codec -87.
                    Field(process,"current_view_pitch_bits",checked(active+0xc+0x14c),4);
                    Field(process,"current_view_yaw_bits",checked(active+0xc+0x150),4);
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
