#!/usr/bin/env python3
"""Fill a prepared map queue from an owned native client, without screenshots.

Each missing map receives its own raw and initialized captures. The entire
ready catalog is verified after collection; failures retain targeted evidence.
Normal service data is never opened for writing or used by the experiment.
"""
import argparse
import concurrent.futures
import hashlib
import json
import os
import re
from pathlib import Path
import shlex
import shutil
import signal
import subprocess
import sys
import time

from clientlib import REPO
from prepare_game_state import compute, CLIENT_SHA256
from prepare_map_hud import minimap, minimap_geometry
from prepare_map_batch import read, write, prepare as prepare_batch, verify
from verify_native_maps import Client


def native_map(name):
    return next(m for m in read(REPO/'fixtures/protocol/map-registry-3.3.3.3.json')['maps'] if m['name']==name)


def single_capture_playlist(source, name, maximum_players=12):
    """Keep the owned publisher's first native bot playlist and bind one map."""
    if maximum_players not in (4,12):
        raise ValueError('native bot playlist requires four or twelve players')
    starts=list(re.finditer(rb'(?m)^playlist\s+([0-9]+)\s*$', source))
    if not starts or starts[0].group(1)!=b'1':
        raise ValueError('resource capture requires native playlist0 as the first publisher entry')
    start=starts[0].start();end=starts[1].start() if len(starts)>1 else len(source)
    section=source[start:end]
    if not re.search(rb'(?m)^combattraining\s*$',section):
        raise ValueError('resource capture requires the measured native bot playlist')
    maps=list(re.finditer(rb'(?m)^(?:mp|za)_[a-z0-9_]+,[a-z0-9_]+,[0-9]+\s*$',section))
    if not maps:
        raise ValueError('resource capture playlist has no real map entry')
    section=section[:maps[0].start()]+name.encode('ascii')+b',tdm,1\n'+section[maps[-1].end():]
    section,count=re.subn(rb'(?m)^rule 3265 [0-9]+$',b'rule 3265 '+str(maximum_players).encode(),section)
    if count!=1:raise ValueError('native bot playlist maximum rule is missing or repeated')
    return source[:start]+section+source[end:]


class Lab:
    def __init__(self, directory, service, prefix, base_catalog, initial_state):
        self.directory,self.service,self.prefix=directory,service,prefix
        self.catalog,self.initial_state=base_catalog,initial_state
        self.process=None
        self.experiment=None
        self.finished=[]

    def start(self, name, catalog, stage, compass_probe=None, capture_playlist=None):
        if compass_probe is not None and stage in ('native','restore'):
            raise ValueError('compass candidate requests require isolated resource collection')
        if self.process is not None:
            raise RuntimeError('previous owned experiment must stop before startup')
        if stage not in ('native','restore') and (native_map(name).get('no_custom_room')
                                                  or native_map(name)['maximum_players']<12):
            capture_playlist=0 if capture_playlist is None else capture_playlist
            if capture_playlist!=0 or self.service.resolve()==(REPO/'client-work/server-data').resolve():
                raise ValueError('quick resource capture requires native playlist0 and isolated service data')
            publisher=self.service/'publisher/mp_playlists_shipRetail_release.info'
            before=publisher.read_bytes();after=single_capture_playlist(before,name,min(12,native_map(name)['maximum_players']))
            publisher.write_bytes(after)
            write(self.directory/(name+'-publisher-selection.json'),dict(map=name,playlist=0,
                before_sha256=hashlib.sha256(before).hexdigest(),after_sha256=hashlib.sha256(after).hexdigest()))
        if stage=='native':
            entries=read(catalog)['entries']
            target=next((e['map'] for e in entries if name in (e['map'],e['map']+'-retry')),None)
            if target and (native_map(target).get('no_custom_room') or native_map(target)['maximum_players']<12):
                if self.service.resolve()==(REPO/'client-work/server-data').resolve() or len(entries)!=1:
                    raise ValueError('quick native checks require one map and isolated service data')
                state=read((catalog.parent/entries[0]['file']).resolve())
                publisher=self.service/'publisher/mp_playlists_shipRetail_release.info'
                publisher.write_bytes(single_capture_playlist(publisher.read_bytes(),target,
                    state['match_config'].get('player_count',12)))
        log=(self.directory/(name+'-'+stage+'-harness.log')).open('w')
        started=time.time()
        initial_state=self.initial_state
        if stage=='native':
            initial_state=(catalog.parent/read(catalog)['entries'][0]['file']).resolve()
        command=[sys.executable,str(REPO/'tools/gamehub_run.py'),'client','--prefix-name',self.prefix,
                 '--seconds','0','--continuous-play','--title-id-hex','3ea','--rust-auth','--release-server',
                 '--local-auth-tickets','--rust-lobby','--rust-stun','--rust-game','--local-services',
                 '--trust-lab-cert','--quiet-network','--native-bot-rooms','--service-data-dir',str(self.service),
                 '--game-state',str(initial_state),'--game-map-catalog',str(catalog)]
        if capture_playlist is not None and stage in ('native','restore'):
            raise ValueError('resource playlist is limited to isolated collection')
        if stage not in ('native','restore'):command+=['--resource-capture-map',name]
        if capture_playlist is not None:command+=['--resource-capture-playlist',str(capture_playlist)]
        if compass_probe is not None:
            command+=['--resource-compass-probe',str(compass_probe)]
        self.process=subprocess.Popen(command,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
        log.close()
        deadline=time.monotonic()+240
        while time.monotonic()<deadline:
            for path in (REPO/'research-output/gamehub').glob('client-*/experiment.json'):
                if path.stat().st_mtime<started:continue
                try:info=read(path)
                except (OSError,json.JSONDecodeError):continue
                if info.get('prefix')!=str(REPO/'client-work'/self.prefix):continue
                self.experiment=path
                if not info.get('pid'):continue
                actual=subprocess.run(['ps','-p',str(info['pid']),'-o','comm='],text=True,capture_output=True)
                if actual.stdout.strip()!=info['argv'][1]:continue
                self.experiment=path
                write(self.directory/'current-experiment.json',dict(harness=self.process.pid,experiment=str(path),map=name,stage=stage))
                return Client(path)
            if self.process.poll() is not None:raise RuntimeError('owned harness failed during startup')
            time.sleep(.2)
        raise RuntimeError('owned client manifest timeout')

    def record_failure(self, client, name, stage):
        diagnostic=dict(map=name,stage=stage,experiment=str(client.experiment),screenshots=0)
        try:diagnostic['state']=client.state()
        except (RuntimeError,ValueError,OSError,subprocess.SubprocessError) as error:
            diagnostic['probe_error']=str(error)
        write(self.directory/(name+'-'+stage+'-failure.json'),diagnostic)

    def stop(self):
        if self.process is None:return
        if self.process.poll() is None:
            actual=subprocess.check_output(['ps','-p',str(self.process.pid),'-o','command='],text=True)
            if str(REPO/'tools/gamehub_run.py') not in actual or '--service-data-dir '+str(self.service) not in actual:
                raise RuntimeError('owned harness PID mismatch')
            os.kill(self.process.pid,signal.SIGTERM)
        self.process.wait(timeout=30)
        if self.experiment:
            info=read(self.experiment)
            if not info.get('finished_unix') or info.get('lab_certificate_restored') is not True:
                raise RuntimeError('owned experiment certificate cleanup incomplete')
            self.finished.append(dict(experiment=str(self.experiment),finished_unix=info['finished_unix'],
                                      lab_certificate_restored=True))
            write(self.directory/'diagnostic-cleanup.json',dict(experiments=self.finished))
        self.process,self.experiment=None,None

    def enter(self, client, name):
        client.wait(lambda s:s['connection_state']==0 and s['frontend_state']==5
                    and s['active_inventory_count']>0,100)
        if native_map(name).get('no_custom_room') or native_map(name)['maximum_players']<12:
            if read(client.experiment).get('resource_capture_playlist')!=0:
                raise ValueError('map prohibits native custom rooms; use its explicit quick capture binding')
            client.click(.297,.05);client.click(.14,.125);client.click(.156,.255)
            client.click(.215,.328);client.click(.763,.854)
            state=client.wait(lambda s:s['room_native_map']==name and s['room_native_playlist']==0
                and (s['room_member_count']>0 or s['connection_state']>=6),30)
            if state['connection_state']==0:client.click(.848,.916)
            return
        client.click(.297,.05);client.click(.14,.125)
        client.click(.437,.255);client.click(.763,.854)
        client.wait(lambda s:s['room_member_count']>0 and s['room_is_host']==1,15)
        client.select_map(name,'auto')
        client.click(.848,.916)

    def captures(self, client, modes):
        def capture(mode):
            path,_=client.capture(mode)
            return mode,path
        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as workers:
            return dict(workers.map(capture,modes))

    def collect(self, name, template, cache=None):
        paths={}
        complete=cache/(name+'-resource-paths.json') if cache else None
        if complete and complete.is_file():
            captures=read(complete)
            if name in ('mp_highrise','mp_subbase_sh','mp_dr_box','mp_dr_cistern','za_coliseum'):
                try:minimap(read(Path(captures['entities'])),name,read(Path(captures['characters'])))
                except ValueError:captures=self.probe_compass(name,captures)
            # Preparation revalidates SHA, map, CRC, real textures and all
            # resource identities; cached metadata alone cannot pass.
            collision=read(Path(captures['collision']))
            if collision.get('world_membership',{}).get('bsp_root')!=0:
                return self.refresh_collision(name,template,captures)
            return self.prepare_captures(name,template,captures)
        cached=cache/(name+'-raw-paths.json') if cache else None
        if cached and cached.is_file():
            paths=read(cached)
            entities=read(Path(paths['map-entities']))
            if not any(m['name']=='maps/mp/'+name+'.d3dbsp' for m in entities['maps']):
                raise ValueError('cached raw entities belong to another map')
            summary=compute(read(Path(paths['net-constants'])))
        else:
            client=self.start(name,self.catalog,'raw')
            try:
                self.enter(client,name)
                for attempt in range(8):
                    paths=self.captures(client,['net-constants','map-entities'])
                    entities=read(Path(paths['map-entities']))
                    if any(m['name']=='maps/mp/'+name+'.d3dbsp' for m in entities['maps']):
                        summary=compute(read(Path(paths['net-constants'])))
                        break
                    time.sleep(.5)
                else:raise RuntimeError('requested raw map resources did not load')
            except Exception:
                self.record_failure(client,name,'raw')
                raise
            finally:self.stop()
        if paths:
            constants=Path(paths['net-constants'])
            initialized_path=self.initialization_catalog(name,constants,summary)
            write(self.directory/(name+'-raw-paths.json'),paths)
        probe=(self.compass_candidate(name,Path(paths['map-entities']),summary['ncs_crc'])
               if name in ('mp_highrise','mp_subbase_sh','mp_dr_box','mp_dr_cistern','za_coliseum') else None)
        client=self.start(name,initialized_path,'initialized',probe)
        try:
            self.enter(client,name)
            client.wait(lambda s:s['connection_state']==9 and s['net_constants_initialized']==1
                         and s['net_constants_crc']==summary['ncs_crc'],45)
            captures=self.captures(client,['net-constants','collision','arsenal','characters','outfits'])
            captures['entities']=paths['map-entities']
            captures['experiment']=str(self.experiment)
            write(self.directory/(name+'-resource-paths.json'),captures)
            return self.prepare_captures(name,template,captures)
        except Exception:
            self.record_failure(client,name,'initialized')
            raise
        finally:self.stop()


    def initialization_catalog(self, name, constants, summary):
        metadata=dict(schema_version=1,client_sha256=CLIENT_SHA256,map=name,mode='war',zone='mp',playlist=0,
                      capture_sha256=hashlib.sha256(constants.read_bytes()).hexdigest(),**summary)
        metadata_path=self.directory/(name+'-metadata.json');write(metadata_path,metadata)
        initialized=read(self.catalog)
        initialized['entries']=[e for e in initialized['entries'] if e['map']!=name]
        initialized['entries'].append(dict(map=name,mode='war',file=str(metadata_path)))
        path=self.directory/(name+'-initialization-catalog.json');write(path,initialized)
        return path

    def compass_candidate(self, name, entities, crc):
        path=self.directory/(name+'-compass-probe.json')
        write(path,dict(schema_version=1,client_sha256=CLIENT_SHA256,map=name,ncs_crc=crc,
            entities_sha256=hashlib.sha256(entities.read_bytes()).hexdigest(),
            minimap=dict(material='compass_map_'+name,**minimap_geometry(read(entities),name))))
        return path

    def probe_compass(self, name, captures):
        constants=Path(captures['net-constants']);summary=compute(read(constants))
        catalog=self.initialization_catalog(name,constants,summary)
        entities=Path(captures['entities'])
        candidate=self.compass_candidate(name,entities,summary['ncs_crc'])
        client=self.start(name,catalog,'compass',candidate)
        try:
            self.enter(client,name)
            client.wait(lambda s:s['connection_state']==9 and s['net_constants_initialized']==1
                        and s['net_constants_crc']==summary['ncs_crc'],45)
            for attempt in range(3):
                path,data=client.capture('characters')
                try:minimap(read(entities),name,data,hashlib.sha256(Path(path).read_bytes()).hexdigest())
                except ValueError:
                    if attempt==2:raise
                    time.sleep(5)
                    continue
                sources=dict(captures,characters=path)
                write(self.directory/(name+'-compass-paths.json'),dict(candidate=str(candidate),
                      previous_characters=captures['characters'],characters=path,experiment=str(self.experiment)))
                return sources
        except Exception:
            self.record_failure(client,name,'compass')
            raise
        finally:self.stop()

    def refresh_collision(self, name, template, captures):
        # Legacy exports traversed the whole pool, including inline models at
        # their local origins. Refresh only geometry; reuse the remaining data.
        constants=Path(captures['net-constants']);summary=compute(read(constants))
        catalog=self.initialization_catalog(name,constants,summary)
        client=self.start(name,catalog,'world-refresh')
        try:
            self.enter(client,name)
            client.wait(lambda s:s['connection_state']==9 and s['net_constants_initialized']==1
                        and s['net_constants_crc']==summary['ncs_crc'],45)
            collision,data=client.capture('collision')
            if data.get('world_membership',{}).get('bsp_root')!=0:
                raise ValueError('refreshed collision has no native world membership')
            sources=dict(captures,collision=collision,experiment=str(self.experiment))
            write(self.directory/(name+'-world-refresh.json'),dict(map=name,
                previous_collision=captures['collision'],collision=collision,
                world_membership=data['world_membership']))
            return self.prepare_captures(name,template,sources)
        except Exception:
            self.record_failure(client,name,'world-refresh')
            raise
        finally:self.stop()


    def prepare_captures(self, name, template, captures):
        write(self.directory/(name+'-resource-paths.json'),captures)
        preparation=self.directory/(name+'-state.json')
        candidate=self.directory/(name+'-captured-starts.json')
        command=[sys.executable,str(REPO/'tools/prepare_tdm_map.py'),'--template',str(template),'--map',name,
                 '--constants',captures['net-constants'],'--entities',captures['entities'],
                 '--collision',captures['collision'],'--arsenal',captures['arsenal'],
                 '--characters',captures['characters'],'--native-characters',captures['outfits'],
                 '--output',str(candidate)]
        with (self.directory/(name+'-preparation.log')).open('w') as log:
            subprocess.run(command,stdout=log,stderr=subprocess.STDOUT,check=True)
        state=read(candidate)
        if state.get('appearance_file'):
            state['appearance_file']=str((template.parent/state['appearance_file']).resolve())
        write(candidate,state)
        with (self.directory/(name+'-start-selection.log')).open('w') as log:
            subprocess.run([str(REPO/'target/release/codol-server'),'prepare-map-starts',
                            '--game-state',str(candidate),'--output',str(preparation),
                            '--report',str(self.directory/(name+'-start-selection.json'))],
                           stdout=log,stderr=subprocess.STDOUT,check=True)
        return preparation


def service_hashes(service):
    return {str(p.relative_to(service)):hashlib.sha256(p.read_bytes()).hexdigest()
            for p in service.rglob('*') if p.is_file()}


def stop_owned_harness(pid, experiment, service, prefix):
    command=shlex.split(subprocess.check_output(['ps','-p',str(pid),'-o','command='],text=True))
    required=[str(REPO/'tools/gamehub_run.py'),'client','--service-data-dir',str(service),
              '--prefix-name',prefix]
    if any(token not in command for token in required):
        raise RuntimeError('owned harness identity mismatch')
    for flag,value in (('--service-data-dir',str(service)),('--prefix-name',prefix)):
        if command[command.index(flag)+1]!=value:raise RuntimeError('owned harness scope mismatch')
    os.kill(pid,signal.SIGTERM)
    deadline=time.monotonic()+30
    while time.monotonic()<deadline:
        info=read(experiment)
        if info.get('finished_unix') and info.get('lab_certificate_restored') is True:return
        time.sleep(.2)
    raise RuntimeError('owned experiment certificate cleanup incomplete')


class NormalSession:
    """Suspend the supplied normal harness, then restore its exact resources."""
    def __init__(self, experiment, directory):
        self.experiment=experiment.resolve();self.directory=directory
        info=read(self.experiment)
        if (not self.experiment.is_relative_to((REPO/'research-output/gamehub').resolve())
                or info.get('client_sha256')!=CLIENT_SHA256 or info.get('finished_unix')
                or info.get('resource_capture_map')):
            raise ValueError('normal restore requires a running project-owned gameplay manifest')
        argv=info['tls_argv']
        self.service=Path(argv[argv.index('--service-data-dir')+1]).resolve()
        if self.service!=(REPO/'client-work/server-data').resolve():
            raise ValueError('supplied normal experiment uses another service directory')
        self.catalog=Path(argv[argv.index('--game-map-catalog')+1]).resolve()
        self.initial_state=Path(argv[argv.index('--game-state')+1]).resolve()
        prefix=Path(info['prefix']).resolve()
        if prefix.parent!=(REPO/'client-work').resolve():raise ValueError('normal prefix ownership mismatch')
        self.prefix=prefix.name
        actual=subprocess.check_output(['ps','-p',str(info['pid']),'-o','comm='],text=True).strip()
        if actual!=info['argv'][1]:raise ValueError('normal client PID mismatch')
        self.harness=int(subprocess.check_output(['ps','-p',str(info['pid']),'-o','ppid='],text=True))
        self.suspended=False

    def suspend(self):
        stop_owned_harness(self.harness,self.experiment,self.service,self.prefix)
        self.suspended=True
        self.hashes=service_hashes(self.service)
        self.resources={p:hashlib.sha256(p.read_bytes()).hexdigest() for p in
                        [self.catalog,self.initial_state]+[
                            (self.catalog.parent/e['file']).resolve() for e in read(self.catalog)['entries']]}
        write(self.directory/'normal-before.json',dict(experiment=str(self.experiment),hashes=self.hashes))

    def restore(self):
        if not self.suspended:return
        unchanged=service_hashes(self.service)==self.hashes
        resources_unchanged=all(hashlib.sha256(p.read_bytes()).hexdigest()==h for p,h in self.resources.items())
        proof=dict(normal_unchanged_during_batch=unchanged,normal_resources_unchanged=resources_unchanged,
                   finished_experiment=str(self.experiment),lab_certificate_restored=read(self.experiment).get('lab_certificate_restored'))
        write(self.directory/'normal-restoration.json',proof)
        lab=Lab(self.directory,self.service,self.prefix,self.catalog,self.initial_state)
        client=lab.start('normal-restored',self.catalog,'restore')
        proof.update(normal_experiment=str(client.experiment),normal_harness=lab.process.pid)
        write(self.directory/'normal-restoration.json',proof)
        state=client.wait(lambda s:s['connection_state']==0 and s['frontend_state']==5
                          and s['active_inventory_count']>0,100)
        restored_hashes=service_hashes(self.service)
        changed=[p for p in set(self.hashes)|set(restored_hashes)
                 if self.hashes.get(p)!=restored_hashes.get(p)]
        proof.update(normal_restored=True,active_inventory_count=state['active_inventory_count'],
                     normal_restore_changed_files=changed)
        write(self.directory/'normal-restoration.json',proof)
        if not unchanged or not resources_unchanged or any(not p.startswith('users/') for p in changed):
            raise RuntimeError('normal account/resources changed; see normal-restoration.json')


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--queue',type=Path,required=True)
    parser.add_argument('--template',type=Path,required=True)
    parser.add_argument('--inventory',type=Path,required=True)
    parser.add_argument('--service-data-dir',type=Path,help='isolated data; default: create a copy for this batch')
    parser.add_argument('--prefix-name',default='wine10-throwables-20261011')
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--maps',nargs='+',help='collect a subset; default is all observed pending maps')
    parser.add_argument('--cache',type=Path,help='reuse this map\'s verified raw captures for targeted retries')
    parser.add_argument('--normal-experiment',type=Path,help='suspend this owned normal client and automatically restore it')
    native_targets=parser.add_mutually_exclusive_group()
    native_targets.add_argument('--native-maps',nargs='+',help='limit native acceptance to these ready maps')
    native_targets.add_argument('--native-new-only',action='store_true',help='only smoke-test preparations created in this batch')
    parser.add_argument('--skip-native',action='store_true',help='only prepare resources and verify on the server')
    parser.add_argument('--skip-collection',action='store_true',help='verify the queue\'s existing preparations')
    args=parser.parse_args()
    output=args.output.resolve()
    service=(args.service_data_dir or REPO/'client-work'/('map-batch-'+output.name)).resolve()
    normal=(REPO/'client-work/server-data').resolve()
    if (service==normal or service.is_relative_to(normal) or not service.is_relative_to((REPO/'client-work').resolve())
            or output==normal or output.is_relative_to(normal)):
        parser.error('resource collection requires isolated project service data')
    queue=read(args.queue)
    tasks=[m for m in queue['maps'] if m['status']=='needs_resources']
    if args.maps:
        by_name={m['map']:m for m in tasks}
        if not set(args.maps)<=set(by_name):parser.error('requested maps are not pending observed resources')
        tasks=[by_name[name] for name in dict.fromkeys(args.maps)]
    if args.skip_collection:tasks=[]
    output.mkdir(parents=True,exist_ok=False)
    catalog=read(Path(queue['ready_catalog']))
    catalog_path=output/'collected-catalog.json'
    write(catalog_path,catalog)
    # Newly generated candidates must not poison the catalog needed to collect
    # later maps. Capture uses only the previously verified base preparations.
    base=verify(args.queue.resolve(),output/'preflight')
    capture_catalog=output/'capture-base-catalog.json'
    capture_entries=[e for e in catalog['entries'] if base['maps'][e['map']]['server_passed']]
    if not capture_entries:raise ValueError('no passing base preparation for native startup')
    write(capture_catalog,dict(schema_version=1,entries=capture_entries))
    lab=Lab(output,service,args.prefix_name,capture_catalog,Path(capture_entries[0]['file']))
    report=dict(schema_version=1,client_sha256=CLIENT_SHA256,maps=[],complete=False)
    normal_session=NormalSession(args.normal_experiment,output) if args.normal_experiment else None
    try:
        if normal_session:normal_session.suspend()
        if not service.exists():shutil.copytree(normal,service)
        for ordinal,entry in enumerate(tasks,1):
            name=entry['map'];print(f'collect {ordinal}/{len(tasks)} {name}',flush=True)
            row=dict(map=name,prepared=False,errors=[])
            try:
                preparation=lab.collect(name,args.template.resolve(),args.cache.resolve() if args.cache else None)
                catalog['entries'].append(dict(map=name,mode='war',file=str(preparation)))
                write(catalog_path,catalog)
                row.update(prepared=True,preparation=str(preparation))
            except (RuntimeError,ValueError,KeyError,OSError,subprocess.SubprocessError) as error:
                row['errors'].append(str(error))
                print('targeted collection failed',name,str(error)[-200:],flush=True)
            report['maps'].append(row);write(output/'collection.json',report)
        report['complete']=True;write(output/'collection.json',report)
        configured=output/'configuration'
        prepare_batch(catalog_path,args.inventory.resolve(),configured)
        native_names=args.native_maps
        if args.native_new_only:
            native_names=[m['map'] for m in report['maps'] if m['prepared']]
        native_enabled=not args.skip_native and (not args.native_new_only or bool(native_names))
        result=verify(configured/'queue.json',output/'acceptance',names=native_names,
                      native_lab=lab if native_enabled else None)
        print(json.dumps(dict(prepared=sum(m['prepared'] for m in report['maps']),
                              failed=[m['map'] for m in report['maps'] if not m['prepared']],
                              ready_server_passed=result['ready_server_passed'],
                              all_maps_passed=result['all_maps_passed'])),flush=True)
        failed=bool(result['targeted_resource_collection'] or any(not m['prepared'] for m in report['maps']))
    finally:
        lab.stop()
        if normal_session:normal_session.restore()
    if failed:raise SystemExit(1)


if __name__=='__main__':
    main()
