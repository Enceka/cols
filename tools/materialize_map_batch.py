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
from pathlib import Path
import shlex
import shutil
import signal
import subprocess
import sys
import time

from clientlib import REPO
from prepare_game_state import compute, CLIENT_SHA256
from prepare_map_batch import read, write, prepare as prepare_batch, verify
from verify_native_maps import Client


class Lab:
    def __init__(self, directory, service, prefix, base_catalog, initial_state):
        self.directory,self.service,self.prefix=directory,service,prefix
        self.catalog,self.initial_state=base_catalog,initial_state
        self.process=None
        self.experiment=None
        self.finished=[]

    def start(self, name, catalog, stage):
        if self.process is not None:
            raise RuntimeError('previous owned experiment must stop before startup')
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
        if stage not in ('native','restore'):command+=['--resource-capture-map',name]
        self.process=subprocess.Popen(command,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
        log.close()
        for _ in range(150):
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
            # Preparation revalidates SHA, map, CRC, real textures and all
            # resource identities; cached metadata alone cannot pass.
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
            metadata=dict(schema_version=1,client_sha256=CLIENT_SHA256,map=name,mode='war',zone='mp',playlist=0,
                          capture_sha256=hashlib.sha256(constants.read_bytes()).hexdigest(),**summary)
            metadata_path=self.directory/(name+'-metadata.json')
            write(metadata_path,metadata)
            initialized=read(self.catalog)
            initialized['entries'].append(dict(map=name,mode='war',file=str(metadata_path)))
            initialized_path=self.directory/(name+'-initialization-catalog.json')
            write(initialized_path,initialized)
            write(self.directory/(name+'-raw-paths.json'),paths)
        client=self.start(name,initialized_path,'initialized')
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


    def prepare_captures(self, name, template, captures):
        write(self.directory/(name+'-resource-paths.json'),captures)
        preparation=self.directory/(name+'-state.json')
        command=[sys.executable,str(REPO/'tools/prepare_tdm_map.py'),'--template',str(template),'--map',name,
                 '--constants',captures['net-constants'],'--entities',captures['entities'],
                 '--collision',captures['collision'],'--arsenal',captures['arsenal'],
                 '--characters',captures['characters'],'--native-characters',captures['outfits'],
                 '--output',str(preparation)]
        with (self.directory/(name+'-preparation.log')).open('w') as log:
            subprocess.run(command,stdout=log,stderr=subprocess.STDOUT,check=True)
        state=read(preparation)
        if state.get('appearance_file'):
            state['appearance_file']=str((template.parent/state['appearance_file']).resolve())
        write(preparation,state)
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
    parser.add_argument('--native-maps',nargs='+',help='limit native acceptance to these ready maps')
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
        result=verify(configured/'queue.json',output/'acceptance',names=args.native_maps,
                      native_lab=None if args.skip_native else lab)
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
