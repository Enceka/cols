"""Owned native capture workflow; no game resources are downloaded or published."""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import sys
import time
from contextlib import ExitStack
import uuid

from clientlib import REPO, matches_client_image, sha256
from prepare_game_state import CLIENT_SHA256, compute
from prepare_resources import command, read, registry, write
from verify_native_maps import Client


class Session:
    def __init__(self, root, client, prefix, service, server, certificates, port_offset=0, reuse_stun=False):
        self.root,self.client,self.prefix,self.service,self.server,self.certificates=root,client,prefix,service,server,certificates
        self.process=None;self.experiment=None
        self.reuse_stun=reuse_stun
        self.port_offset=port_offset
        self.handle='res'+uuid.uuid4().hex[:12]

    def start(self, name=None, metadata=None, mode='war', compass=None):
        if self.process is not None:raise RuntimeError('previous capture must stop first')
        output=self.root/'sessions';output.mkdir(exist_ok=True)
        args=[sys.executable,str(REPO/'tools/gamehub_run.py'),'client','--client-dir',str(self.client),
            '--prefix-name',self.prefix.name,'--output-root',str(output),'--seconds','900','--title-id-hex','3ea',
            '--rust-auth','--local-auth-tickets','--rust-lobby','--local-services','--trust-lab-cert',
            '--quiet-network','--service-data-dir',str(self.service),'--server-binary',str(self.server),
            '--certificate-dir',str(self.certificates),'--handle',self.handle,'--managed-stdin',
            '--client-number',str(self.port_offset or 1),'--port-offset',str(self.port_offset)]
        if not self.reuse_stun:args.append('--rust-stun')
        marker=read(self.prefix/'.codol-prefix.json');args+=['--engine-id',marker['engine_id']]
        if name:
            args+=['--rust-game','--continuous-play','--resource-capture-map',name,'--resource-capture-mode',mode,
                   '--resource-capture-only','--resource-capture-playlist','0']
        if metadata:args+=['--game-state',str(metadata)]
        if compass:args+=['--resource-compass-probe',str(compass)]
        log=(self.root/'native.log').open('ab')
        self.process=subprocess.Popen(args,stdin=subprocess.PIPE,stdout=log,stderr=subprocess.STDOUT)
        log.close()
        deadline=time.monotonic()+300
        while time.monotonic()<deadline:
            for path in output.glob('client-*/experiment.json'):
                try:info=read(path,1024*1024)
                except json.JSONDecodeError:continue
                if info.get('prefix')!=str(self.prefix) or info.get('finished_unix') or not info.get('pid'):continue
                actual=subprocess.run(['ps','-p',str(info['pid']),'-o','comm='],text=True,capture_output=True)
                if not matches_client_image(actual.stdout.strip(),info['argv'][1],self.prefix):continue
                self.experiment=path
                return Client(path, self.root/"probe", self.root/"probe/game-window")
            if self.process.poll() is not None:raise RuntimeError('capture client startup failed; see native.log')
            time.sleep(.2)
        raise RuntimeError('capture client startup timed out')

    def stop(self):
        if self.process is None:return
        if self.process.poll() is None:self.process.send_signal(signal.SIGTERM)
        try:self.process.wait(timeout=100)
        except subprocess.TimeoutExpired:raise RuntimeError('owned client cleanup timed out; capture cannot be marked complete')
        if self.experiment:
            info=read(self.experiment,1024*1024)
            if not info.get('finished_unix') or info.get('lab_certificate_restored') is not True:
                raise RuntimeError('owned client certificate cleanup incomplete')
        if self.process.stdin is not None:self.process.stdin.close()
        self.process=None;self.experiment=None


def choose_prefix(client_source=None):
    config=REPO.parent/'launcher/profiles/native.local.json'
    candidates=[]
    if config.is_file():
        value=read(config,65536).get('client',{}).get('wine_prefix')
        if value:candidates.append((config.parent/value).resolve())
    candidates.append(REPO/'client-work/wine10-prefix')
    if client_source:candidates.append(Path(client_source).parent/'wine10-prefix')
    candidates+=sorted((REPO/'client-work').glob('*'))
    if client_source:candidates+=sorted(Path(client_source).parent.glob('wine10-*'))
    for path in candidates:
        marker=path/'.codol-prefix.json'
        if not path.is_dir() or not marker.is_file():continue
        if read(marker,4096).get('engine_id')!='10000033':continue
        # Refuse a prefix currently owned by another operation; never stop it.
        lock=path/'.codol-lock'
        if lock.is_file():
            with lock.open('rb') as stream:
                try:fcntl.flock(stream,fcntl.LOCK_EX|fcntl.LOCK_NB)
                except BlockingIOError:continue
                fcntl.flock(stream,fcntl.LOCK_UN)
        return path.resolve()
    raise ValueError('a prepared project-owned GameHub Wine10 environment is required')


def certificates(root, source=None):
    if source:
        source=Path(source)
        if not all((source/n).is_file() for n in ['server.pem','key.pem']):raise ValueError('local TLS identity is incomplete')
        return source
    target=root/'tls';target.mkdir()
    config=target/'openssl.cnf'
    config.write_text('[req]\ndistinguished_name=dn\nx509_extensions=extensions\nprompt=no\n[dn]\nCN=localhost\n[extensions]\nsubjectAltName=DNS:localhost,IP:127.0.0.1\nbasicConstraints=critical,CA:TRUE\nkeyUsage=critical,digitalSignature,keyEncipherment,keyCertSign\n')
    executable=next((p for p in ['/opt/homebrew/bin/openssl','/usr/bin/openssl'] if Path(p).is_file()),None)
    if executable is None:raise ValueError('OpenSSL is required to generate local capture certificates')
    command([executable,'req','-x509','-newkey','rsa:2048','-nodes','-sha256','-days','365','-config',config,
             '-keyout',target/'key.pem','-out',target/'server.pem'],root/'certificate.log')
    (target/'key.pem').chmod(0o600)
    return target


def quick_entry(client):
    client.wait(lambda s:s.get('connection_state')==0 and s.get('frontend_state')==5 and s.get('active_inventory_count',0)>0,100)
    for x,y in [(.297,.05),(.14,.125),(.156,.255),(.215,.328),(.763,.854)]:client.click(x,y)
    state=client.wait(lambda s:s.get('room_native_playlist')==0 and (s.get('room_member_count',0)>0 or s.get('connection_state',0)>=6),30)
    if state['connection_state']==0:client.click(.848,.916)


def bootstrap(session,root,progress):
    progress(0,1,'初始化隔离采集账号并读取本机资源定义')
    client=session.start()
    try:
        client.wait(lambda s:s.get('frontend_state')==5,100)
        deadline=time.monotonic()+90
        while True:
            profiles_path,profiles=client.capture('profiles')
            uid=profiles.get('user_id')
            if str(uid).isdigit() and 0<int(uid)<2**64:break
            if time.monotonic()>deadline:raise RuntimeError('native authentication identity did not become ready')
            time.sleep(1)
        public=session.service/'profiles/1002'/str(uid)/'public.bin'
        deadline=time.monotonic()+30
        selections=0
        while not public.is_file() and time.monotonic()<deadline:
            if selections<3:
                client.click(.50,.55)
                client.click(.855,.865)
                selections+=1
                time.sleep(8)
            else:time.sleep(.2)
        if not public.is_file():raise RuntimeError('native character selection did not upload its public profile')
        assets_path,asset_data=client.capture('assets')
        for name in ['mp/playerdata.def','mp/playersecuredata.def']:
            if not asset_data.get('schemas',{}).get(name,{}).get('found'):raise ValueError('native profile definitions did not load after character selection')
        assets=root/'assets.json';shutil.copyfile(assets_path,assets)
        snapshot=root/'profiles.json';shutil.copyfile(profiles_path,snapshot)
    finally:session.stop()
    command([session.server,'prepare-private-profile','--assets',assets,'--snapshot',snapshot,'--service-data-dir',session.service,'--training-complete'],root/'bootstrap.log')
    command([sys.executable,REPO/'tools/prepare_inventory.py','--assets',assets,'--public-profile',public,'--service-data-dir',session.service],root/'bootstrap.log')
    from prepare_player_template import prepare as starter
    import base64
    template=starter(read(assets),public.read_bytes(),int(uid))
    write(root/'new-player-template.json',template)
    public.write_bytes(base64.b64decode(template['public_base64'],validate=True))
    command([session.server,'boost-test-account','--service-data-dir',session.service,'--user-id',str(uid)],root/'bootstrap.log')
    return assets


# This fixed startup IFS has a documented Wine/APFS file-object stall.
BOOTSTRAP_FILE=('IIPS/IIPSDownload/ff_init_common_gfx_V43.ifs',26504032,
                'a4bac3f12ef090773ee5fc5017e5f88dae600f2d81005e0f67257c02b78ff667')


def isolate_bootstrap_file(source,client):
    relative,size,expected=BOOTSTRAP_FILE
    origin=source/relative;destination=client/relative
    if origin.is_symlink():raise ValueError('启动IFS是外部资源链接，请提供可独立读取的完整客户端副本')
    if not origin.is_file() or origin.stat().st_size!=size:raise ValueError('启动IFS缺失或版本不符')
    temporary=destination.with_suffix('.codol-new')
    try:
        digest=hashlib.sha256();copied=0
        with origin.open('rb') as incoming,temporary.open('xb') as outgoing:
            for block in iter(lambda:incoming.read(65536),b''):
                copied+=len(block)
                if copied>size:raise ValueError('启动IFS在准备期间发生变化')
                digest.update(block);outgoing.write(block)
            outgoing.flush();os.fsync(outgoing.fileno())
        if copied!=size or digest.hexdigest()!=expected:raise ValueError('启动IFS校验和不匹配')
        destination.rename(destination.with_suffix('.codol-before'))
        temporary.rename(destination)
    finally:
        temporary.unlink(missing_ok=True)


def choose_port_offset():
    for _ in range(64):
        offset=10000+int(uuid.uuid4().hex[:4],16)%10000
        with ExitStack() as sockets:
            try:
                for kind,ports in [(socket.SOCK_STREAM,[39001,39002,39003]),(socket.SOCK_DGRAM,[39004])]:
                    for port in ports:
                        probe=sockets.enter_context(socket.socket(socket.AF_INET,kind))
                        probe.bind(('127.0.0.1',port+offset))
            except OSError:continue
            return offset
    raise ValueError('没有可用于隔离采集的空闲端口组')


def check_stun(reuse):
    if reuse:
        for port in [3074,3078]:
            deadline=time.monotonic()+30
            while True:
                try:
                    with socket.socket(socket.AF_INET,socket.SOCK_DGRAM) as probe:
                        probe.bind(('127.0.0.1',0));probe.settimeout(2)
                        probe.sendto(bytes([30,2,0]),('127.0.0.1',port))
                        data,address=probe.recvfrom(64)
                        expected=probe.getsockname()[1].to_bytes(2,'little')
                        if address!=('127.0.0.1',port) or len(data)!=15 or data[:7]!=bytes([31,2,0,127,0,0,1]) or data[7:9]!=expected:
                            raise ValueError('已有STUN服务未通过本版发现协议校验')
                    break
                except (TimeoutError,ConnectionRefusedError) as error:
                    if time.monotonic()>=deadline:raise ValueError('本机STUN发现服务未就绪：'+str(port)) from error
                    time.sleep(.5)
    else:
        with ExitStack() as sockets:
            for port in [3074,3078]:
                probe=sockets.enter_context(socket.socket(socket.AF_INET,socket.SOCK_DGRAM))
                try:probe.bind(('127.0.0.1',port))
                except OSError as error:raise ValueError('原生STUN端口被占用；可显式选择共用已有本机发现服务：'+str(port)) from error


def collect(source,root,server,certificate_dir,maps,mode,progress,reuse_stun=False):
    if sys.platform!='darwin':raise ValueError('native collection currently requires macOS/GameHub; import local captures on Linux')
    if mode!='war':raise ValueError('automatic native playlist collection currently supports war; cha can use retained captures')
    source=Path(source).resolve();root=Path(root).resolve()
    for directory,folders,files in os.walk(source):
        for name in folders+files:
            if (Path(directory)/name).is_symlink():raise ValueError('客户端含软链接，请先准备完整本地副本：'+str((Path(directory)/name).relative_to(source)))
    if sha256(source/'codoMP_client_shipRetail.exe')!=CLIENT_SHA256:raise ValueError('client differs from the fixed 3.3.3.3 baseline')
    available=registry(mode)
    selected=list(dict.fromkeys(maps or ['mp_afghan']))
    if any(name not in available for name in selected):raise ValueError('unknown native map requested')
    progress(0,1,'检查本机发现服务')
    check_stun(reuse_stun)
    port_offset=choose_port_offset()
    prefix_source=choose_prefix(source)
    identity=uuid.uuid4().hex[:12]
    # Owned Wine prefixes stay under this tool checkout; captures follow the selected output.
    (REPO/'client-work').mkdir(exist_ok=True)
    progress(0,1,'创建本次采集的隔离客户端与Wine环境')
    owned=REPO/'client-work'/('resource-capture-'+identity);owned.mkdir()
    client=owned/'CODOL';prefix=REPO/'client-work'/('resource-prefix-'+identity)
    # Hold both existing source locks while cloning; never terminate their owners.
    from gamehub_run import existing_client_owner, LAB_MARKER
    with ExitStack() as locks:
        for lock_path in [source/'.codol-client-lock', prefix_source/'.codol-lock']:
            if lock_path.exists():
                stream=locks.enter_context(lock_path.open('rb'))
                fcntl.flock(stream,fcntl.LOCK_EX|fcntl.LOCK_NB)
        if existing_client_owner(source) is not None:raise ValueError('source client is running; close it before collecting')
        for origin,destination in [(source,client),(prefix_source,prefix)]:
            flag='-cR' if origin.stat().st_dev==destination.parent.stat().st_dev else '-R'
            command(['cp',flag,origin,destination],root/'clone.log')
    progress(0,1,'校验并准备隔离副本的原生启动资源')
    isolate_bootstrap_file(source,client)
    (client/LAB_MARKER).write_text(json.dumps(dict(purpose='owned local resource preparation',source=str(source))))
    command([sys.executable,REPO/'tools/client_state.py','--build','--prefix-name',prefix.name,'--probe-dir',root/'probe'],root/'probe-build.log')
    command(['sh',REPO/'tools/build_game_window.sh',root/'probe'],root/'probe-build.log')
    service=root/'capture-service';service.mkdir();(service/'.codol-resource-capture').write_text('local-only\n')
    command([server,'prepare-publisher','--output-dir',service/'publisher'],root/'bootstrap.log')
    session=Session(root,client,prefix,service,server,certificates(root,certificate_dir),port_offset,reuse_stun)
    try:
        assets=bootstrap(session,root,progress)
        rows=[]
        for ordinal,name in enumerate(selected):
            progress(ordinal,len(selected),'在用户本机采集 '+name+'：读取原生常量与实体')
            from materialize_map_batch import single_capture_playlist
            publisher=service/'publisher/mp_playlists_shipRetail_release.info'
            publisher.write_bytes(single_capture_playlist(publisher.read_bytes(),name,min(12,available[name]['maximum_players'])))
            client_api=session.start(name,mode=mode)
            try:
                quick_entry(client_api)
                deadline=time.monotonic()+45
                while True:
                    entities_path,entities=client_api.capture('map-entities')
                    if any(m['name']=='maps/mp/'+name+'.d3dbsp' for m in entities['maps']):break
                    if time.monotonic()>deadline:raise RuntimeError('requested map entities did not load')
                    time.sleep(.5)
                constants_path,constants=client_api.capture('net-constants')
                summary=compute(constants)
                metadata=root/(name+'-metadata.json')
                write(metadata,dict(schema_version=1,client_sha256=CLIENT_SHA256,capture_sha256=hashlib.sha256(Path(constants_path).read_bytes()).hexdigest(),map=name,mode=mode,zone='mp',playlist=0,**summary))
                from prepare_map_hud import CAPTURED_ALIASES,minimap_geometry
                compass=root/(name+'-compass.json')
                write(compass,dict(schema_version=1,client_sha256=CLIENT_SHA256,map=name,ncs_crc=summary['ncs_crc'],entities_sha256=hashlib.sha256(Path(entities_path).read_bytes()).hexdigest(),minimap=dict(material=CAPTURED_ALIASES.get(name,'compass_map_'+name),**minimap_geometry(entities,name))))
            finally:session.stop()
            progress(ordinal,len(selected),'在用户本机采集 '+name+'：初始化并导出世界、武器与角色')
            client_api=session.start(name,metadata,mode,compass if name in {'mp_highrise','mp_subbase_sh','mp_dr_box','mp_dr_cistern','za_coliseum','mp_ambush_sh','mp_boomtown'} else None)
            try:
                quick_entry(client_api)
                client_api.wait(lambda s:s.get('connection_state')==9 and s.get('net_constants_initialized')==1 and s.get('net_constants_crc')==summary['ncs_crc'],60)
                captures={'entities':str(entities_path)}
                for key,flag in [('net-constants','net-constants'),('collision','collision'),('arsenal','arsenal'),('characters','characters'),('outfits','outfits')]:
                    path,_=client_api.capture(flag);captures[key]=str(path)
                rows.append(dict(map=name,mode=mode,captures=captures))
                write(root/(name+'-resource-paths.json'),captures)
            finally:session.stop()
        plan=root/'capture-plan.json';write(plan,dict(schema_version=1,client_sha256=CLIENT_SHA256,assets=str(assets),maps=rows))
        write(root/'capture-cleanup.json',dict(owned_client=str(client),owned_prefix=str(prefix),original_client_modified=False,all_owned_sessions_stopped=True))
        return plan
    finally:
        session.stop()
