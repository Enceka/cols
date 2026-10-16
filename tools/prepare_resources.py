#!/usr/bin/env python3
"""Prepare server resources from the user's own local captures or game client.

Outputs remain local. This tool does not download, publish or license game data.
Only the final runtime directory is intended for the user's server deployment.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import threading

from clientlib import REPO
from prepare_game_state import CLIENT_SHA256, compute


def read(path, limit=256*1024*1024):
    path = Path(path)
    if not path.is_file() or path.stat().st_size > limit:
        raise ValueError('missing or oversized input: '+str(path))
    with path.open('rb') as stream:raw=stream.read(limit+1)
    if len(raw)>limit:raise ValueError('input grew beyond budget: '+str(path))
    return json.loads(raw)


def write(path, value):
    path = Path(path)
    with path.open('x') as stream:
        json.dump(value, stream, ensure_ascii=False, allow_nan=False, separators=(",", ":"))
        stream.write('\n')


def command(argv, log):
    with Path(log).open('ab') as stream:
        with subprocess.Popen([str(a) for a in argv], stdin=subprocess.DEVNULL,
                              stdout=stream, stderr=subprocess.STDOUT) as child:
            try:
                code = child.wait()
            except BaseException:
                child.terminate()
                try: child.wait(timeout=15)
                except subprocess.TimeoutExpired: child.kill(); child.wait()
                raise
    if code:
        tail=Path(log).read_bytes()[-4096:].decode('utf-8',errors='replace')
        raise RuntimeError('resource preparation failed: '+tail)


def registry(mode):
    name='charge-map-registry-3.3.3.3.json' if mode=='cha' else 'map-registry-3.3.3.3.json'
    return {m['name']:m for m in read(REPO/'fixtures/protocol'/name)['maps']}


def plan(source, mode):
    if mode not in ("war", "cha"): raise ValueError("unsupported mode")
    source=Path(source).resolve()
    path=source if source.is_file() else source/'capture-plan.json'
    if path.is_file():
        value=read(path,1024*1024)
        if value.get('schema_version')!=1 or value.get('client_sha256')!=CLIENT_SHA256:
            raise ValueError('capture plan identity/version mismatch')
        base=path.parent
        return value,base
    manifests=sorted(source.glob('*-resource-paths.json'))
    assets=next((p for p in [source/'assets.json',source/'client-assets.json'] if p.is_file()),None)
    if not manifests or assets is None:
        raise ValueError('choose capture-plan.json or a cache with assets.json and map resource manifests')
    maps=[]
    for manifest in manifests:
        name=manifest.name[:-len('-resource-paths.json')]
        if name not in registry(mode):raise ValueError('unknown capture map: '+name)
        captures=read(manifest,65536)
        maps.append(dict(map=name,mode=mode,captures={k:str((manifest.parent/v).resolve()) for k,v in captures.items() if k in ('net-constants','entities','collision','arsenal','characters','outfits','voice','props','rewards')}))
    return dict(schema_version=1,client_sha256=CLIENT_SHA256,assets=str(assets),maps=maps),source


def resolve(base, value):
    if not isinstance(value,str) or not value:raise ValueError('capture path missing')
    return (base/value).resolve()


def blueprint(constants, characters, characters_raw):
    """Construct only server rules; all model, animation and weapon IDs are rebound."""
    summary=compute(constants,'m4_mp',True,True,'viewhands_tf141')
    if not summary['observed_crc_verified']:raise ValueError('initialized native constants are required')
    bodies=[m for m in characters['models'] if m['name']=='mp_fullbody_juggernaut_explosive_so']
    if len(bodies)!=1:raise ValueError('captured human body unavailable or ambiguous')
    body=bodies[0]
    state=dict(schema_version=1,client_sha256=CLIENT_SHA256,map='mp_afghan',mode='war',zone='mp',playlist=0,
        capture_sha256='0'*64,**summary)
    state['match_config']=dict(score_limit=7500,time_limit_msec=600000)
    state['training_enemy']=dict(model=body['model'],origin=[0,0,0],bounds_min=body['bounds_min'],bounds_max=body['bounds_max'],
        idle_animation=1,walk_animation=1,fire_animation=1,death_animation=1,
        move_speed=190,angles=[0,0,0],weapon=summary['bootstrap_weapon']['base_index'],
        source_sha256=hashlib.sha256(characters_raw).hexdigest(),bot=True)
    return state


def prepare_captures(source, output, server, mode='war', maps=None, progress=lambda *args:None):
    from prepare_tdm_map import prepare
    from prepare_bot_roster import NativeBotCharacters
    from prepare_player_appearance import prepare as appearance
    from prepare_room_bot_loadouts import bind
    value,base=plan(source,mode)
    assets_path=resolve(base,value.get('assets'))
    assets=read(assets_path,16*1024*1024)
    if assets.get('read_only') is not True or assets.get('client_sha256')!=CLIENT_SHA256:
        raise ValueError('global asset capture identity mismatch')
    rows=value.get('maps')
    if not isinstance(rows,list) or not 1<=len(rows)<=64:raise ValueError('capture plan needs 1..64 maps')
    rows=[r for r in rows if r.get('mode',mode)==mode]
    if not rows:raise ValueError('capture plan has no maps for selected mode')
    if maps:
        missing=set(maps)-{r['map'] for r in rows}
        if missing:raise ValueError('requested maps have no captures: '+','.join(sorted(missing)))
        rows=[r for r in rows if r['map'] in maps]
    work=Path(output)/'prepared';work.mkdir(parents=True,exist_ok=False)
    entries=[];seen=set()
    for ordinal,row in enumerate(rows):
        name,native_mode=row['map'],row.get('mode',mode)
        if native_mode not in ('war','cha') or name not in registry(native_mode) or (name,native_mode) in seen:
            raise ValueError('unknown or duplicate map/mode')
        seen.add((name,native_mode));progress(ordinal,len(rows),'从只读采集生成 '+name+' / '+native_mode)
        paths={k:resolve(base,v) for k,v in row['captures'].items()}
        required=['net-constants','entities','collision','arsenal','characters','outfits']
        if any(k not in paths for k in required):raise ValueError('missing captures for '+name+': '+','.join(k for k in required if k not in paths))
        captures={k:read(paths[k]) for k in required}
        if captures['collision'].get('world_membership',{}).get('bsp_root')!=0:
            raise ValueError('collision needs a current BSP-world capture: '+name)
        template=blueprint(captures['net-constants'],captures['characters'],paths['characters'].read_bytes())
        template.update(map=name,mode=native_mode)
        directory=work/(name+'-'+native_mode);directory.mkdir()
        character=appearance(captures['outfits'],assets)
        character['sources_sha256']=[hashlib.sha256(paths['outfits'].read_bytes()).hexdigest(),hashlib.sha256(assets_path.read_bytes()).hexdigest()]
        appearance_path=directory/'appearance.json';write(appearance_path,character)
        template['appearance_file']=str(appearance_path);template['appearance_first_model']=1
        state,_=prepare(template,name,captures['net-constants'],captures['entities'],captures['collision'],captures['arsenal'],captures['characters'],
            material_capture_sha256=hashlib.sha256(paths['characters'].read_bytes()).hexdigest(),native_characters=NativeBotCharacters(captures['outfits']),mode=native_mode)
        state.update(capture_sha256=hashlib.sha256(paths['net-constants'].read_bytes()).hexdigest(),map_entities_sha256=hashlib.sha256(paths['entities'].read_bytes()).hexdigest(),arsenal_file=str(paths['arsenal']),collision_file=str(paths['collision']))
        bind(state,character,captures['arsenal'])
        candidate=directory/'candidate.json';write(candidate,state)
        loadout=directory/'loadout.json'
        command([sys.executable,REPO/'tools/prepare_player_loadout.py','--state',candidate,'--assets',assets_path,'--arsenal',paths['arsenal'],'--collision',paths['collision'],'--output',loadout],directory/'prepare.log')
        grounded=directory/'state.json'
        command([server,'prepare-map-starts','--game-state',loadout,'--output',grounded,'--report',directory/'starts.json'],directory/'prepare.log')
        state=read(grounded)
        if paths.get('voice'):
            from prepare_voice import prepare as voice
            resource=voice(paths['net-constants'].read_bytes(),grounded.read_bytes(),paths['voice'].read_bytes())
            write(directory/'voice.json',resource);state['voice_file']=str(directory/'voice.json')
        for capture_key,tool,field in [('props','prepare_props.py','props_file'),('rewards','prepare_radars.py','radar_file')]:
            if capture_key not in paths:continue
            destination=directory/(capture_key+'.json')
            if capture_key=='props':args=['--assets',paths[capture_key],'--entities',paths['entities'],'--constants',paths['net-constants']]
            else:args=['--assets',paths[capture_key],'--ncs',paths['net-constants']]
            command([sys.executable,REPO/'tools'/tool,*args,'--output',destination],directory/'prepare.log')
            state[field]=str(destination)
        final=directory/'ready.json';write(final,state)
        entries.append(dict(map=name,mode=native_mode,file=str(final)))
    catalog=work/'catalog.json';write(catalog,dict(schema_version=1,entries=entries))
    return catalog


def main():
    os.umask(0o077)
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('kind',choices=['captures','client'])
    p.add_argument('--source',type=Path,required=True)
    p.add_argument('--output',type=Path,required=True)
    p.add_argument('--server',type=Path,required=True)
    p.add_argument('--mode',choices=['war','cha'],default='war')
    p.add_argument('--maps',nargs='+')
    p.add_argument('--report',type=Path)
    p.add_argument('--progress-file',type=Path)
    p.add_argument('--certificate-dir',type=Path)
    p.add_argument('--managed-stdin',action='store_true')
    p.add_argument('--reuse-stun',action='store_true',help='explicitly share the existing loopback STUN service on 3074/3078')
    args=p.parse_args()
    if args.reuse_stun and args.kind!="client":p.error("reuse-stun is only for native client collection")
    for path in [args.output,args.report,args.progress_file]:
        if path is not None and path.exists():p.error('choose new output/report paths; existing files are preserved')
    args.output=args.output.resolve();args.server=args.server.resolve();args.source=args.source.resolve()
    if args.kind=='client' and args.output.is_relative_to(args.source):
        p.error('native capture output must be outside the source client')
    args.output.mkdir(parents=True,exist_ok=False)
    def cancelled(signum,frame):raise KeyboardInterrupt('resource preparation cancelled')
    signal.signal(signal.SIGTERM,cancelled)
    if args.managed_stdin:
        def parent_closed():
            while os.read(0,1):pass
            os.kill(os.getpid(),signal.SIGTERM)
        threading.Thread(target=parent_closed,daemon=True).start()
    def progress(completed,total,message):
        print(message,file=sys.stderr,flush=True)
        if args.progress_file:
            temporary=args.progress_file.with_suffix('.tmp')
            temporary.write_text(json.dumps(dict(completed=completed,total=total,message=message),ensure_ascii=False))
            os.replace(temporary,args.progress_file)
    source=args.source
    if args.kind=='client':
        from resource_capture import collect
        source=collect(args.source,args.output,args.server,args.certificate_dir,args.maps,args.mode,progress,args.reuse_stun)
    catalog=prepare_captures(source,args.output,args.server,args.mode,args.maps,progress)
    report=args.report or args.output/'result.json'
    command([args.server,'prepare-resources','--source',catalog,'--output',args.output/'runtime','--report',report],args.output/'bundle.log')
    result=read(report,65536)
    print(json.dumps(result,ensure_ascii=False))


if __name__=='__main__':
    try:main()
    except (ValueError,RuntimeError,OSError,subprocess.SubprocessError,KeyboardInterrupt) as error:
        print(str(error),file=sys.stderr,flush=True)
        raise SystemExit(1)
