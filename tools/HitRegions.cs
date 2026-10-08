// Read-only current-client bone regions. Invoked after ReadClientState's EXE
// identity check; this class never calls code inside the game or writes memory.
using System;
using System.Collections.Generic;
using System.Text;
using System.Web.Script.Serialization;

class HitRegions {
    const string ClientHash="e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b";
    const string BodyName="mp_fullbody_juggernaut_explosive_so";
    readonly IntPtr process;
    HitRegions(IntPtr process) { this.process=process; }
    byte[] Read(uint address,int size) { return ReadClientState.ReadBytes(process,address,size); }
    uint Word(uint address) { return BitConverter.ToUInt32(Read(address,4),0); }
    static Dictionary<string,object> Obj(params object[] pairs) {
        var result=new Dictionary<string,object>();
        for(int i=0;i<pairs.Length;i+=2)result[(string)pairs[i]]=pairs[i+1];
        return result;
    }
    static float[] Floats(byte[] bytes,int offset,int count) {
        var result=new float[count];
        for(int i=0;i<count;i++) {
            result[i]=BitConverter.ToSingle(bytes,offset+i*4);
            if(Single.IsNaN(result[i])||Single.IsInfinity(result[i]))throw new Exception("nonfinite bone geometry");
        }
        return result;
    }
    string Name(uint address) {
        if(address<0x10000)throw new Exception("missing bone/model name");
        var result=new List<byte>();
        for(int block=0;block<4;block++)foreach(byte value in Read(address+(uint)block*64,64)) {
            if(value==0)return Encoding.ASCII.GetString(result.ToArray());
            if(value<32||value>126)throw new Exception("invalid bone/model identifier");
            result.Add(value);
        }
        throw new Exception("bone/model name exceeds limit");
    }
    uint Context(uint slotAddress,uint mask) {
        uint slot=Word(slotAddress);
        if(slot>=256)throw new Exception("invalid native context slot");
        uint pointer=Word(0x01a19510+slot*4)^mask;
        if(pointer<0x10000)throw new Exception("native context unavailable");
        return pointer;
    }
    float[] EntityOrigin(uint state) {
        var origin=new float[3];
        // Current protected trBase reader0x412ce0; masks remain internal.
        for(uint axis=0;axis<3;axis++) {
            uint address=state+0x18+axis*4;
            uint bits=~Word(0x00cda23c+axis*4)^Word(address)^address;
            origin[axis]=BitConverter.ToSingle(BitConverter.GetBytes(bits),0);
            if(Single.IsNaN(origin[axis])||Single.IsInfinity(origin[axis])||Math.Abs(origin[axis])>32768)
                throw new Exception("invalid NPC origin");
        }
        return origin;
    }
    static float[] LocalVector(float[] vector,double cosine,double sine) {
        return new float[]{(float)(cosine*vector[0]+sine*vector[1]),(float)(-sine*vector[0]+cosine*vector[1]),vector[2]};
    }
    // Axis columns from the exact quaternion/translationWeight expressions
    // in current0x645dd0. They transform bone-local vectors to pose space.
    static float[][] Axes(float[] matrix) {
        double x=matrix[0],y=matrix[1],z=matrix[2],w=matrix[3],weight=matrix[7];
        return new float[][] {
            new float[]{(float)(1-weight*(y*y+z*z)),(float)(weight*(x*y+w*z)),(float)(weight*(x*z-w*y))},
            new float[]{(float)(weight*(x*y-w*z)),(float)(1-weight*(x*x+z*z)),(float)(weight*(y*z+w*x))},
            new float[]{(float)(weight*(x*z+w*y)),(float)(weight*(y*z-w*x)),(float)(1-weight*(x*x+y*y))}
        };
    }
    static bool PoseValid(float[] matrix,float[][] axes) {
        double norm=0;
        for(int i=0;i<4;i++)norm+=(double)matrix[i]*matrix[i];
        if(norm<1e-10||matrix[7]<0)return false;
        for(int i=0;i<3;i++)for(int j=i;j<3;j++) {
            double dot=0;for(int k=0;k<3;k++)dot+=(double)axes[i][k]*axes[j][k];
            if(Math.Abs(dot-(i==j?1:0))>0.01)return false;
        }
        return true;
    }
    object CandidateWeapon(uint index) {
        if(index<1||index>=900)return null;
        uint definition=Word(0x017f5a40+index*4);
        if(definition<0x10000)return null;
        byte[] raw=Read(definition+0xbf0,80);var bits=new List<uint>();
        for(int i=0;i<20;i++)bits.Add(BitConverter.ToUInt32(raw,i*4));
        return Obj("weapon",index,"name",Name(Word(definition)),"offset",0xbf0,"bits",bits,
            "status","inferred_only_name_to_offset_not_proven");
    }
    object Capture() {
        if(Word(0x01a2e7ec)!=10)throw new Exception("load the map before hit-region capture");
        uint crc=Word(0x02df7d90),cgs=Context(0x07817bac,0xce9af8cf),cg=Context(0x07817ba8,0xfbb2c97a);
        uint entity=Context(0x07817bb0,0x2168f672)+0x238,state=entity+0xf0;
        if(Word(state+4)!=1||(Word(entity+0x204)&1)==0)throw new Exception("live PLAYER entity1 required");
        uint slot=BitConverter.ToUInt16(Read(0x02ef4858+2,2),0);
        if(slot==0||slot>=2048)throw new Exception("NPC DObj unavailable");
        uint dobj=0x02e8e850+slot*0xcc,models=Word(dobj+0xc0);
        int modelCount=Read(dobj+9,1)[0];
        if(modelCount<1||modelCount>32||models<0x10000)throw new Exception("invalid NPC DObj models");
        uint model=Word(models),registered=Word(cgs+0xf0+109*4);
        if(model!=registered||Name(Word(model))!=BodyName)throw new Exception("NPC body root is not registered model109");
        byte[] header=Read(model,0x168);int count=header[4],roots=header[5];
        if(count<1||count>253||roots<1||roots>count)throw new Exception("invalid NPC bone counts");
        uint namePointer=BitConverter.ToUInt32(header,0x2c),locationPointer=BitConverter.ToUInt32(header,0x3c);
        uint parentPointer=BitConverter.ToUInt32(header,0x30),boundsPointer=BitConverter.ToUInt32(header,0x138);
        uint bindPointer=BitConverter.ToUInt32(header,0x40),stringPool=Word(0x02f37164);
        byte[] ids=Read(namePointer,count*2),locations=Read(locationPointer,count);
        byte[] parents=roots==count?new byte[0]:Read(parentPointer,count-roots);
        byte[] bounds=Read(boundsPointer,count*28),bind=Read(bindPointer,count*32);
        var names=new string[count];
        for(int i=0;i<count;i++)names[i]=Name(stringPool+(uint)BitConverter.ToUInt16(ids,i*2)*20+4);
        if(names[0]!="tag_origin"||Array.IndexOf(names,"j_head")<0||Array.IndexOf(names,"j_spineupper")<0)
            throw new Exception("NPC root/head/torso skeleton identity mismatch");
        uint posePointer=Word(dobj+0x98),time=0;byte[] pose=null;float[] reference=null,origin=null,angles=null;
        if(posePointer<0x10000)throw new Exception("NPC pose not calculated; keep it visible and retry");
        for(int attempt=0;attempt<5;attempt++) {
            time=Word(cg+0xae4c8);
            reference=Floats(Read(cg+0xae538,12),0,3);
            origin=EntityOrigin(state);angles=Floats(Read(entity+0x30,12),0,3);
            pose=Read(posePointer,count*32);
            if(time==Word(cg+0xae4c8)&&posePointer==Word(dobj+0x98))break;
            pose=null;
        }
        if(pose==null)throw new Exception("NPC pose advanced during capture; retry");
        double yaw=angles[1]*Math.PI/180,cosine=Math.Cos(yaw),sine=Math.Sin(yaw);
        float[] rootMatrix=Floats(pose,0,8);double rootError=0;
        for(int axis=0;axis<3;axis++) {
            double error=(double)rootMatrix[4+axis]+reference[axis]-origin[axis];rootError+=error*error;
        }
        if(rootError>4)throw new Exception("NPC is moving between snapshot and rendered pose; capture it while stationary");
        var bones=new List<object>();
        for(int i=0;i<count;i++) {
            if(locations[i]>=20)throw new Exception("bone hit-location exceeds current enum");
            int? parent=null;
            if(i>=roots) {
                int distance=parents[i-roots];
                if(distance<1||distance>i)throw new Exception("invalid NPC bone parent distance");
                parent=i-distance;
            }
            float[] center=Floats(bounds,i*28,3),half=Floats(bounds,i*28+12,3);
            float radius=Floats(bounds,i*28+24,1)[0];
            for(int axis=0;axis<3;axis++)if(half[axis]<0)throw new Exception("negative bone half-extent");
            if(radius<0)throw new Exception("negative bone radius squared");
            float[] matrix=Floats(pose,i*32,8),bindMatrix=Floats(bind,i*32,8);
            float[][] worldAxes=Axes(matrix),localAxes=new float[3][];
            for(int axis=0;axis<3;axis++)localAxes[axis]=LocalVector(worldAxes[axis],cosine,sine);
            float[] relative=new float[3];
            for(int axis=0;axis<3;axis++)relative[axis]=matrix[4+axis]+reference[axis]-origin[axis];
            bones.Add(Obj("index",i,"name",names[i],"hit_location",locations[i],"parent_index",parent,
                "bounds_center",center,"bounds_half",half,"radius_squared",radius,"pose_matrix",matrix,
                "pose_valid",PoseValid(matrix,worldAxes),"pose_translation_local",LocalVector(relative,cosine,sine),
                "pose_axes_local",localAxes,"bind_pose_matrix",bindMatrix));
        }
        var weapons=new List<object>();uint npcWeapon=Word(state+0xd0)&65535;
        object npc=CandidateWeapon(npcWeapon);if(npc!=null)weapons.Add(npc);
        uint playerWeapon=Word(cg+0x340);if(playerWeapon!=npcWeapon) {
            object player=CandidateWeapon(playerWeapon);if(player!=null)weapons.Add(player);
        }
        if(Word(0x01a2e7ec)!=10||Word(state+4)!=1||crc!=Word(0x02df7d90)||slot!=BitConverter.ToUInt16(Read(0x02ef4858+2,2),0))
            throw new Exception("NPC/map identity changed during capture");
        return Obj("schema_version",1,"read_only",true,"client_sha256",ClientHash,"connection_state",10,
            "ncs_crc",crc,"entity_number",1,"entity_type",1,"model",109,"model_name",BodyName,
            "pose_kind","captured_static_pose","capture_time_msec",time,"capture_origin",origin,"capture_angles",angles,
            "capture_view_offset",reference,"bone_count",count,"root_bone_count",roots,"bones",bones,
            "weapon_candidate_multipliers",weapons);
    }
    public static void Export(IntPtr process) {
        var serializer=new JavaScriptSerializer();serializer.MaxJsonLength=1024*1024;
        Console.WriteLine(serializer.Serialize(new HitRegions(process).Capture()));
    }
}
