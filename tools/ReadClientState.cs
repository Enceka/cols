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
    static void IdentityField(IntPtr process,string name,uint address) {
        uint pointer=Read(process,address,4);
        if(pointer<0x10000)return;
        byte[] bytes=ReadBytes(process,pointer,64);
        int length=Array.IndexOf(bytes,(byte)0);
        if(length<0)return;
        for(int i=0;i<length;i++)if(!(bytes[i]>=65&&bytes[i]<=90)&&!(bytes[i]>=97&&bytes[i]<=122)&&!(bytes[i]>=48&&bytes[i]<=57)&&bytes[i]!=95)return;
        Console.WriteLine("\""+name+"\":\""+Encoding.ASCII.GetString(bytes,0,length)+"\",");
    }
    static int Main(string[] args) {
        try {
            bool feedback=args.Length==1 && args[0]=="--feedback";
            bool replay=args.Length==1 && args[0]=="--replay";
            bool assets=args.Length==1 && args[0]=="--assets";
            bool profiles=args.Length==1 && args[0]=="--profiles";
            bool netConstants=args.Length==1 && args[0]=="--net-constants";
            bool mapEntities=args.Length==1 && args[0]=="--map-entities";
            bool bootstrapWeapon=args.Length==1 && args[0]=="--bootstrap-weapon";
            bool weaponModels=(args.Length==1 || args.Length==2) && args[0]=="--weapon-models";
            bool arsenal=args.Length==1 && args[0]=="--arsenal";
            bool collision=args.Length==1 && args[0]=="--collision";
            bool glass=args.Length==1 && args[0]=="--glass";
            bool frontendAssets=args.Length==1 && args[0]=="--frontend-assets";
            bool weaponRegistry=args.Length==1 && args[0]=="--weapon-registry";
            bool trainingTarget=args.Length==1 && args[0]=="--training-target";
            bool bulletImpacts=args.Length==1 && args[0]=="--bullet-impacts";
            bool characters=args.Length==1 && args[0]=="--characters";
            bool mapCheck=args.Length==1 && args[0]=="--map-check";
            bool grounding=args.Length==1 && args[0]=="--grounding";
            bool outfits=args.Length==1 && args[0]=="--outfits";
            bool appearanceLoad=args.Length==1 && args[0]=="--appearance-load";
            bool hitRegions=args.Length==1 && args[0]=="--hit-regions";
            bool tacticalAssets=args.Length==1 && args[0]=="--tactical-assets";
            bool hudAssets=args.Length==1 && args[0]=="--hud-assets";
            bool voiceAssets=args.Length==1 && args[0]=="--voice-assets";
            bool voicePlayback=args.Length==1 && args[0]=="--voice-playback";
            if(args.Length!=0 && !voicePlayback && !voiceAssets && !frontendAssets && !glass && !mapCheck && !hudAssets && !tacticalAssets && !appearanceLoad && !outfits && !grounding && !replay && !feedback && !hitRegions && !assets && !profiles && !netConstants && !mapEntities && !bootstrapWeapon && !weaponModels && !arsenal && !collision && !weaponRegistry && !trainingTarget && !bulletImpacts && !characters) throw new Exception("only fixed extraction modes are supported");
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
                if(feedback) { ClientAssets.ExportFeedback(process);return 0; }
                if(replay) { ClientAssets.ExportReplay(process);return 0; }
                if(profiles) { ClientAssets.ExportProfiles(process);return 0; }
                if(netConstants) { ClientAssets.ExportNetConstants(process);return 0; }
                if(mapEntities) { ClientAssets.ExportMapEntities(process);return 0; }
                if(bootstrapWeapon) { ClientAssets.ExportBootstrapWeapon(process);return 0; }
                if(weaponModels) { ClientAssets.ExportWeaponModels(process,args.Length==2?args[1]:"m4_mp");return 0; }
                if(arsenal) { ClientAssets.ExportArsenal(process);return 0; }
                if(collision) { ClientAssets.ExportCollision(process);return 0; }
                if(frontendAssets) { ClientAssets.ExportFrontendAssets(process); return 0; }
                if(glass) { ClientAssets.ExportGlass(process);return 0; }
                if(weaponRegistry) { ClientAssets.ExportWeaponRegistry(process);return 0; }
                if(trainingTarget) { ClientAssets.ExportTrainingTarget(process);return 0; }
                if(bulletImpacts) { ClientAssets.ExportBulletImpacts(process);return 0; }
                if(characters) { ClientAssets.ExportCharacters(process);return 0; }
                if(mapCheck) { ClientAssets.ExportMapCheck(process);return 0; }
                if(grounding) { ClientAssets.ExportGrounding(process);return 0; }
                if(outfits) { ClientAssets.ExportOutfits(process);return 0; }
                if(appearanceLoad) { ClientAssets.ExportAppearanceLoad(process);return 0; }
                if(hitRegions) { HitRegions.Export(process);return 0; }
                if(tacticalAssets) { ClientAssets.ExportTacticalAssets(process);return 0; }
                if(hudAssets) { ClientAssets.ExportHudAssets(process);return 0; }
                if(voiceAssets) { ClientAssets.ExportVoiceAssets(process);return 0; }
                if(voicePlayback) { ClientAssets.ExportVoicePlayback(process);return 0; }
                Console.WriteLine("{");
                Field(process,"connection_state",0x01a2e7ec,4);
                Field(process,"player_card_visible",0x0b5f3024,1);
                Field(process,"ui_scoreboard_row_count",0x02f25368,4);
                uint noBotTop3=Read(process,0x017e5454,4);
                if(noBotTop3>=0x10000)Field(process,"native_no_bot_top3",checked(noBotTop3+12),4);
                Console.WriteLine("\"top_player_cards\":"+new System.Web.Script.Serialization.JavaScriptSerializer().Serialize(ClientAssets.TopPlayerCards(process))+",");
                if(Read(process,0x01a2e7ec,4)==10) Console.WriteLine("\"showcase_actors\":"+new System.Web.Script.Serialization.JavaScriptSerializer().Serialize(ClientAssets.ShowcaseActors(process))+",");
                if(Read(process,0x01a2e7ec,4)==10) Console.WriteLine("\"held_weapons\":"+new System.Web.Script.Serialization.JavaScriptSerializer().Serialize(ClientAssets.HeldWeapons(process))+",");
                Field(process,"sound_fade_current_bits",0x0780e4b0,4);
                Field(process,"sound_fade_target_bits",0x0780e4b4,4);
                Field(process,"sound_fade_rate_bits",0x0780e4b8,4);
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
                // This cache is the last full baseline, not necessarily the
                // latest delta. Use current_* / predicted_* for live movement.
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
                        Field(process,"predicted_health",checked(cg+0x190),4);
                        Field(process,"predicted_spawn_count",checked(cg+0x19c),4);
                        Field(process,"predicted_corpse_slot",checked(cg+0xe8),4);
                        Field(process,"predicted_client_number",checked(cg+0x144),4);
                        Field(process,"local_client_rank",checked(cg+0x15dea8+0x9c),4);
                        Field(process,"local_character_parts",checked(cg+0x15dea8+0x38c),4);
                        Field(process,"local_character_faction",checked(cg+0x15dea8+0x390),4);
                        Field(process,"local_character_gender",checked(cg+0x15dea8+0x394),4);
                        Field(process,"local_character_skin",checked(cg+0x15dea8+0x398),4);
                        Field(process,"local_client_prestige",checked(cg+0x15dea8+0xa0),4);
                        Field(process,"local_client_card_icon",checked(cg+0x15dea8+0x358),4);
                        Field(process,"local_client_card_title",checked(cg+0x15dea8+0x35c),4);
                        Field(process,"local_client_card_background",checked(cg+0x15dea8+0x360),4);
                        Field(process,"predicted_replay_delay",checked(cg+0xa68),4);
                        Field(process,"predicted_replay_target",checked(cg+0xa6c),4);
                        Field(process,"native_killcam_active",checked(cg+0xc6fe8),4);
                        Field(process,"killcam_timer_type",checked(cg+0xa78),4);
                        Field(process,"killcam_timer_end",checked(cg+0xa78+0x78),4);
                        Field(process,"predicted_hit_hud_type",checked(cg+0x1ed0),4);
                        Field(process,"predicted_hit_hud_material",checked(cg+0x1ed0+0x4c),4);
                        Field(process,"predicted_death_hud_type",checked(cg+0xa78),4);
                        Field(process,"predicted_death_hud_material",checked(cg+0xa78+0x4c),4);
                        Field(process,"predicted_death_hud_flags",checked(cg+0xa78+0xa4),4);
                        for(uint axis=0;axis<3;axis++)
                            Field(process,"predicted_death_hud_origin_"+axis+"_bits",checked(cg+0xa78+4+axis*4),4);
                        Field(process,"predicted_pm_flags",checked(cg+0xc),4);
                        Field(process,"predicted_pm_time",checked(cg+8),4);
                        Field(process,"predicted_sprint_button_up_required",checked(cg+0x23c),4);
                        Field(process,"predicted_sprint_delay",checked(cg+0x240),4);
                        Field(process,"predicted_sprint_last_start",checked(cg+0x244),4);
                        Field(process,"predicted_sprint_last_end",checked(cg+0x248),4);
                        Field(process,"predicted_sprint_start_max_length",checked(cg+0x24c),4);
                        Field(process,"predicted_other_flags",checked(cg+0x10),4);
                        Field(process,"predicted_camera_reference_entity",checked(cg+0x200),4);
                        Field(process,"predicted_weapon",checked(cg+0x340),4);
                        Field(process,"predicted_offhand_weapon",checked(cg+0x32c),4);
                        Field(process,"predicted_lethal_weapon",checked(cg+0x330),4);
                        Field(process,"predicted_secondary_offhand_weapon",checked(cg+0x334),4);
                        Field(process,"predicted_tactical_weapon",checked(cg+0x338),4);
                        Field(process,"predicted_grenade_time",checked(cg+0x34),4);
                        for(uint slot=0;slot<4;slot++) {
                            Field(process,"predicted_clip_key_"+slot,checked(cg+0x3dc+slot*12),4);
                            Field(process,"predicted_clip_count_"+slot,checked(cg+0x3e0+slot*12),4);
                        }
                        Field(process,"predicted_paired_weapon",checked(cg+0x344),4);
                        Field(process,"predicted_weapon_fire_count",checked(cg+0x284),4);
                        Field(process,"predicted_perk_display",checked(cg+0x4bc),1);
                        Field(process,"native_xp_ready",0x0318b16b,1);
                        for(uint slot=0;slot<3;slot++) {
                            Field(process,"predicted_perk_"+slot,checked(cg+0x4c0+slot*4),4);
                            Field(process,"native_perk_id_"+slot,checked(cg+0x16be18+slot*4),4);
                        }
                        for(uint slot=0;slot<5;slot++) {
                            Field(process,"predicted_ability_mask_"+slot,checked(cg+0x4cc+slot*4),4);
                            Field(process,"local_ability_mask_"+slot,checked(cg+0x15dea8+0xa8+slot*4),4);
                        }
                        for(uint slot=0;slot<30;slot++)
                            Field(process,"predicted_ability_value_"+slot+"_bits",checked(cg+0x4e0+slot*4),4);
                        Field(process,"predicted_inventory_weapon_0",checked(cg+0x2a4),4);
                        Field(process,"predicted_inventory_needs_rechamber_0",checked(cg+0x2e3),1);
                        Field(process,"predicted_origin_x_bits",checked(cg+0x1c),4);
                        Field(process,"predicted_origin_y_bits",checked(cg+0x20),4);
                        Field(process,"predicted_origin_z_bits",checked(cg+0x24),4);
                        Field(process,"predicted_velocity_x_bits",checked(cg+0x28),4);
                        Field(process,"predicted_velocity_y_bits",checked(cg+0x2c),4);
                        Field(process,"predicted_velocity_z_bits",checked(cg+0x30),4);
                        for(uint axis=0;axis<3;axis++)for(uint component=0;component<3;component++)
                            Field(process,"render_axis_"+axis+"_"+component+"_bits",checked(cg+0xae510+(axis*3+component)*4),4);
                        for(uint axis=0;axis<3;axis++) {
                            Field(process,"render_angle_"+axis+"_bits",checked(cg+0xb2688+axis*4),4);
                            Field(process,"render_origin_"+axis+"_bits",checked(cg+0xae504+axis*4),4);
                        }
                        uint infoSlot=Read(process,0x07817bb4,4);
                        if(infoSlot<256) {
                            uint info=Read(process,0x01a19510+infoSlot*4,4)^0x7a634997;
                            foreach(uint id in new uint[]{352,353,354,355}) {
                                Field(process,"weapon_info_"+id+"_ready",checked(info+id*0x108),4);
                                Field(process,"weapon_info_"+id+"_display",checked(info+id*0x108+4),4);
                            }
                        }
                        Field(process,"predicted_view_pitch_bits",checked(cg+0x14c),4);
                        Field(process,"predicted_view_yaw_bits",checked(cg+0x150),4);
                        Field(process,"predicted_weapon_animation",checked(cg+0x26c),4);
                        // 0x557390 selects the first-person animation from
                        // PS+0x26c; weapon state is+0x27c and ADS fraction+0x34c.
                        Field(process,"predicted_weapon_state",checked(cg+0x27c),4);
                        Field(process,"predicted_ads_fraction_bits",checked(cg+0x34c),4);
                        Field(process,"cg_frame_msec",checked(cg+0xae4c4),4);
                        Field(process,"cg_time",checked(cg+0xae4c8),4);
                        Field(process,"compass_north_yaw_bits",checked(cg+0xbdb28),4);
                        Field(process,"compass_material",checked(cg+0xbdb34),4);
                        uint compassMaterial=Read(process,checked(cg+0xbdb34),4);
                        // Material GetAssetName 0x483d30 reads header+0x10.
                        if(compassMaterial>=0x10000)IdentityField(process,"compass_material_name",checked(compassMaterial+0x10));
                        Field(process,"compass_upper_left_x_bits",checked(cg+0xbdb38),4);
                        Field(process,"compass_upper_left_y_bits",checked(cg+0xbdb3c),4);
                        Field(process,"compass_width_bits",checked(cg+0xbdb40),4);
                        Field(process,"compass_height_bits",checked(cg+0xbdb44),4);
                        // Binary J offsets exclude the public snapshot's CRC.
                        const uint aarRecord=0x0c95be0c+4;
                        Field(process,"aar_winning_team",aarRecord+3833,1);
                        Field(process,"aar_duration",aarRecord+47,4);
                        Field(process,"aar_map_id",aarRecord+51,4);
                        Field(process,"aar_mode_id",aarRecord+55,4);
                        for(uint team=1;team<=2;team++)Field(process,"aar_team_score_"+team,aarRecord+9778+team*4,4);
                        for(uint client=0;client<19;client++)Field(process,"aar_client_team_"+client,aarRecord+260+client*198,1);
                        Console.Write("\"aar_rows\":[");
                        for(uint client=0;client<19;client++) {
                            byte[] name=ReadBytes(process,aarRecord+166+client*198,65);
                            int end=Array.IndexOf(name,(byte)0);if(end<0)end=name.Length;
                            if(client>0)Console.Write(",");
                            Console.Write(new System.Web.Script.Serialization.JavaScriptSerializer().Serialize(new {
                                slot=client,name=Encoding.UTF8.GetString(name,0,end),
                                team=Read(process,aarRecord+260+client*198,1),
                                is_local=Read(process,aarRecord+157+client*198,1),
                                score=(int)Read(process,aarRecord+244+client*198,4),
                                kills=Read(process,aarRecord+158+client*198,4),
                                deaths=Read(process,aarRecord+137+client*198,4)
                            }));
                        }
                        Console.WriteLine("],");
                        uint bg=Read(process,0x07817b80,4);
                        if(bg>=0x10000 && Read(process,0x01a2e7ec,4)==10) {
                            for(uint client=0;client<12;client++) {
                                uint ci=checked(bg+0x96e20+client*0x3b8);
                                Field(process,"game_client_valid_"+client,ci,4);
                                Field(process,"game_client_team_"+client,checked(ci+0x80),4);
                            }
                        }
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
                // 0x557390: two24-byte first-person records, last anim at+12.
                uint weaponViewSlot=Read(process,0x07817bb8,4);
                if(weaponViewSlot<256) {
                    uint views=Read(process,checked(0x01a19510+weaponViewSlot*4),4)^0x39641402;
                    if(views>=0x10000 && views<0xff000000) {
                        Field(process,"viewmodel_last_animation_0",checked(views+12),4);
                        Field(process,"viewmodel_last_animation_1",checked(views+24+12),4);
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
                    Field(process,"last_local_cmd_buttons",checked(active+0x365c+(cmd&255)*44+4),4);
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
                    Field(process,"current_clip_ammo_0",checked(active+0xc+0x3e0),4);
                    Field(process,"current_reserve_ammo_0",checked(active+0xc+0x368),4);
                    Field(process,"current_weapon_time",checked(active+0xc+0x270),4);
                    Field(process,"current_weapon",checked(active+0xc+0x340),4);
                    Field(process,"current_paired_weapon",checked(active+0xc+0x344),4);
                    Field(process,"current_weapon_fire_count",checked(active+0xc+0x284),4);
                    Field(process,"current_inventory_weapon_0",checked(active+0xc+0x2a4),4);
                    Field(process,"current_inventory_needs_rechamber_0",checked(active+0xc+0x2e3),1);
                    Field(process,"current_camera_reference_entity",checked(active+0xc+0x200),4);
                    Field(process,"current_command_time",checked(active+0xc),4);
                    Field(process,"current_origin_x_bits",checked(active+0xc+0x1c),4);
                    Field(process,"current_origin_y_bits",checked(active+0xc+0x20),4);
                    Field(process,"current_origin_z_bits",checked(active+0xc+0x24),4);
                    Field(process,"current_ads_fraction_bits",checked(active+0xc+0x34c),4);
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
                    Field(process,"room_native_options_pointer",checked(roomController+0xf738),4);
                    // FString::c_str 0xb2d8c0 returns its stored char pointer.
                    IdentityField(process,"room_native_mode",checked(roomController+0xf730));
                    IdentityField(process,"room_native_map",checked(roomController+0xf738));
                    Field(process,"room_native_playlist",checked(roomController+0xf73c),1);
                    Field(process,"room_minimum_players",checked(roomController+0xf73d),1);
                    Field(process,"room_maximum_players",checked(roomController+0xf73e),1);
                    // Native room consumers: 0x6e74d0 / 0x6dfae0.
                    Field(process,"room_native_countdown",checked(roomController+0x2e0),4);
                    Field(process,"room_native_special_flag",checked(roomController+0xf754),1);
                    Field(process,"room_member_count",checked(roomController+0xf718),4);
                    uint memberCount=Read(process,checked(roomController+0xf718),4);
                    if(memberCount<=32) {
                        Console.Write("\"room_members\":[");
                        for(uint i=0;i<memberCount;i++) {
                            uint member=checked(roomController+0xda98+i*0x180);
                            ulong uid=BitConverter.ToUInt64(ReadBytes(process,member,8),0);
                            byte[] name=ReadBytes(process,checked(member+8),64);
                            int end=Array.IndexOf(name,(byte)0);if(end<0)end=name.Length;
                            byte[] clan=ReadBytes(process,checked(member+0x48),64);
                            int clanEnd=Array.IndexOf(clan,(byte)0);if(clanEnd<0)clanEnd=clan.Length;
                            if(i>0)Console.Write(",");
                            Console.Write(new System.Web.Script.Serialization.JavaScriptSerializer().Serialize(new {
                                slot=i, bot_client=uid>0x7fff434f444f0000UL && uid<=0x7fff434f444f000bUL ? (int)(uid-0x7fff434f444f0000UL) : 0,
                                name=Encoding.ASCII.GetString(name,0,end), clan=Encoding.ASCII.GetString(clan,0,clanEnd),
                                room_team=Read(process,checked(member+0x88),4), ready=Read(process,checked(member+0x8c),1), party=Read(process,checked(member+0xa8),4),
                                dlc_level=Read(process,checked(member+0x170),4)
                            }));
                        }
                        Console.WriteLine("],");
                    }
                    Field(process,"room_is_host",checked(roomController+0x260),1);
                    Field(process,"room_join_request_handler",0x078b0760+6*4,4);
                    uint channel=Read(process,checked(roomController+0x10),4);
                    Console.WriteLine("\"room_channel_pointer\":"+channel+",");
                    if(channel>=0x0c8032b0 && channel<0x0c80a420) {
                        Field(process,"room_channel_active",channel,1);
                        Field(process,"room_channel_subscribed",checked(channel+1),1);
                        Field(process,"room_channel_member_count",checked(channel+0x18),4);
                        Field(process,"room_channel_message_handler",checked(channel+0x25b8),4);
                        Field(process,"room_channel_receiver",checked(channel+0x25c8),4);
                    }
                }
                Field(process,"native_minimum_players",checked(Read(process,0x01a323c4,4)+12),4);
                Field(process,"native_maximum_players",checked(Read(process,0x01a32368,4)+12),4);
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
