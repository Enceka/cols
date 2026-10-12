#!/usr/bin/env python3
"""Prepare all native TDM map tasks, then verify ready resources as one batch.

Shared settings are generated once. Missing per-map resources remain explicit;
only real initialized preparations enter the server/native test catalogs.
"""
import argparse
import hashlib
import json
import re
from pathlib import Path
import subprocess

from clientlib import REPO
from prepare_game_state import CLIENT_SHA256
from verify_native_maps import Client, prepare as short_catalog, run as native_run


def read(path):
    return json.loads(path.read_text())


def write(path, value):
    path.write_text(json.dumps(value, indent=2, allow_nan=False)+'\n')


def prepare(catalog_path, inventory_path, output):
    registry=read(REPO/'fixtures/protocol/map-registry-3.3.3.3.json')
    inventory=read(inventory_path)
    catalog=read(catalog_path)
    if (registry['client_sha256']!=CLIENT_SHA256 or inventory.get('client_sha256')!=CLIENT_SHA256
            or inventory.get('read_only') is not True or catalog['schema_version']!=1):
        raise ValueError('batch preparation requires current-client map identities and resource observations')
    existing={entry['map']:(catalog_path.parent/entry['file']).resolve() for entry in catalog['entries']}
    observed={entry['name']:entry for entry in inventory['maps']}
    output.mkdir(parents=True,exist_ok=False)
    shared=dict(mode='war',zone='mp',score_limit=7500,time_limit_msec=600000,
                maximum_players=12,bot_count=11,bot_name_format='Bot {n}')
    write(output/'shared.json',shared)
    tasks=[]
    ready=[]
    for entry in registry['maps']:
        name=entry['name']
        task=dict(map=name,native_id=entry['native_id'],mode='war',shared_configuration='shared.json',
                  maximum_players=min(12,entry['maximum_players']),
                  bot_count=min(12,entry['maximum_players'])-1,
                  observed_fastfiles=observed.get(name,{}).get('observed_fastfiles',{}),
                  resource_inputs_required=['initialized_constants','map_entities','collision','arsenal',
                                            'characters','minimap_material'],status='needs_resources')
        if name in existing:
            source=existing[name]
            state=read(source)
            if state['map']!=name or state['mode']!='war' or state['client_sha256']!=CLIENT_SHA256:
                raise ValueError('prepared map identity differs from the native registry')
            task.update(status='ready',preparation=str(source),
                        preparation_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),ncs_crc=state['ncs_crc'])
            ready.append(dict(map=name,mode='war',file=str(source)))
        elif not any(task['observed_fastfiles'].values()):
            task['status']='needs_resource_audit'
        write(output/(name+'.json'),task)
        tasks.append(task)
    ready_path=output/'ready-catalog.json'
    write(ready_path,dict(schema_version=1,entries=ready))
    diagnostic=short_catalog(ready_path,output/'diagnostic') if ready else None
    queue=dict(schema_version=1,client_sha256=CLIENT_SHA256,shared_configuration='shared.json',
               ready_catalog=str(ready_path),diagnostic_catalog=str(diagnostic) if diagnostic else None,
               resource_observations_sha256=hashlib.sha256(inventory_path.read_bytes()).hexdigest(),maps=tasks)
    write(output/'queue.json',queue)
    return queue


def native_failure(experiment, output, name, error):
    diagnostics=dict(error=str(error),experiment=str(experiment),screenshots=0)
    state={}
    try:
        if experiment is None:raise RuntimeError('native experiment unavailable')
        client=Client(experiment)
        state=client.state()
        write(output/(name+'-failure-state.json'),state)
        if state['connection_state']==10:
            for mode in ('map-check','replay'):
                try:
                    path,_=client.capture(mode)
                    diagnostics[mode]=path
                except (RuntimeError,subprocess.SubprocessError):
                    pass
    except (RuntimeError,ValueError,OSError,KeyError,subprocess.SubprocessError):
        diagnostics['client_unavailable']=True
    write(output/(name+'-failure-diagnostics.json'),diagnostics)
    return state


def verify(queue_path, output, experiment=None, names=None, direction='auto', native_lab=None):
    queue=read(queue_path)
    if queue['client_sha256']!=CLIENT_SHA256 or queue['schema_version']!=1:
        raise ValueError('unsupported map queue')
    output.mkdir(parents=True,exist_ok=False)
    ready=[entry for entry in queue['maps'] if entry['status']=='ready']
    if not ready:
        raise ValueError('no real prepared resources to verify')
    for entry in ready:
        if hashlib.sha256(Path(entry['preparation']).read_bytes()).hexdigest()!=entry['preparation_sha256']:
            raise ValueError('preparation changed; regenerate the batch configuration')
    server_report=output/'server.json'
    command=[str(REPO/'target/release/codol-server'),'verify-maps','--catalog',queue['ready_catalog'],
             '--output',str(server_report),'--seconds','60']
    with (output/'server.log').open('w') as log:
        completed=subprocess.run(command,stdout=log,stderr=subprocess.STDOUT)
    if not server_report.exists():
        # A malformed spawn/geometry binding can fail catalog import before
        # simulation starts. Isolate that map and keep checking the other maps.
        rows=[]
        for entry in ready:
            single_catalog=output/(entry['map']+'-catalog.json')
            single_report=output/(entry['map']+'-server.json')
            single_log=output/(entry['map']+'-server.log')
            write(single_catalog,dict(schema_version=1,entries=[dict(map=entry['map'],mode='war',file=entry['preparation'])]))
            with single_log.open('w') as log:
                subprocess.run([str(REPO/'target/release/codol-server'),'verify-maps','--catalog',str(single_catalog),
                                '--output',str(single_report),'--seconds','60'],stdout=log,stderr=subprocess.STDOUT)
            if single_report.exists():rows.extend(read(single_report)['maps'])
            else:rows.append(dict(map=entry['map'],passed=False,errors=[single_log.read_text()[-1200:]]))
        write(server_report,dict(schema_version=1,client_sha256=CLIENT_SHA256,native_client_used=False,
                                passed=all(row['passed'] for row in rows),maps=rows))
    server=read(server_report)
    outcomes={entry['map']:dict(server_passed=entry['passed'],errors=entry['errors']) for entry in server['maps']}
    failures=[entry['map'] for entry in server['maps'] if not entry['passed']]
    native_error=None
    selected=names or [entry['map'] for entry in ready]
    if experiment or native_lab:
        native_catalog=Path(queue['diagnostic_catalog'])
        if native_lab:
            # Invalid candidates cannot prevent a passing map's native startup.
            passed_catalog=output/'native-ready-catalog.json'
            write(passed_catalog,dict(schema_version=1,entries=[
                dict(map=e['map'],mode='war',file=e['preparation']) for e in ready
                if outcomes[e['map']]['server_passed']]))
            if any(o['server_passed'] for o in outcomes.values()):
                native_catalog=short_catalog(passed_catalog,output/'native-diagnostic')
        for name in selected:
            if name not in outcomes:
                raise ValueError('native target is absent from the ready catalog')
            if not outcomes[name]['server_passed']:
                continue
            rules=next(m for m in read(REPO/'fixtures/protocol/map-registry-3.3.3.3.json')['maps'] if m['name']==name)
            quick=rules.get('no_custom_room') or rules['maximum_players']<12
            selected_catalog=native_catalog
            if quick:
                if not native_lab:
                    raise ValueError('quick native maps require an owned lab and a fresh client per map')
                native_lab.stop();experiment=None
                source=read(native_catalog)
                entry=next(e for e in source['entries'] if e['map']==name)
                version=re.search(rb'(?m)^version ([0-9]+)$',
                    (native_lab.service/'publisher/mp_playlists_shipRetail_release.info').read_bytes())
                if not version:raise ValueError('native publisher version is missing')
                selected_catalog=output/(name+'-quick-catalog.json')
                write(selected_catalog,dict(schema_version=1,entries=[dict(entry,
                    file=str((native_catalog.parent/entry['file']).resolve()))],rotations=[dict(
                    version=int(version.group(1)),playlist=0,maps=[dict(map=name,mode='war')])]))
            for attempt in range(2):
                label=name+('-retry' if attempt else '')
                native_report=output/(label+'-native.json')
                try:
                    if experiment is None:
                        experiment=native_lab.start(label,selected_catalog,'native').experiment
                    native_run(selected_catalog,experiment,native_report,[name],direction,
                               **({'quick_playlist':0} if quick else {}))
                    outcomes[name].update(native_passed=True,native_report=str(native_report),native_retries=attempt)
                    if quick:native_lab.stop();experiment=None
                    break
                except (RuntimeError,ValueError,subprocess.SubprocessError,KeyError) as error:
                    state=native_failure(experiment,output,label,error)
                    if native_lab:
                        # Diagnose unknown frontend state without blind inputs.
                        native_lab.stop()
                        experiment=None
                    report=read(native_report) if native_report.exists() else {}
                    loaded=any('ncs_crc' in row for row in report.get('maps',[]))
                    retry=(native_lab is not None and attempt==0 and not loaded
                           and state.get('connection_state')==0 and state.get('frontend_state')==5)
                    if retry:
                        outcomes[name]['native_initial_error']=str(error)
                        continue
                    native_error=str(error)
                    failures.append(name)
                    outcomes[name].update(native_passed=False,native_retries=attempt)
                    outcomes[name]['errors'].append(str(error))
                    break
    pending=[entry['map'] for entry in queue['maps'] if entry['status']!='ready']
    all_passed=not pending and not failures and native_error is None and bool(experiment or native_lab)
    all_passed=all_passed and all(outcomes[entry['map']].get('native_passed') is True for entry in ready)
    result=dict(schema_version=1,client_sha256=CLIENT_SHA256,
                all_maps_passed=all_passed,ready_server_passed=server['passed'],
                native_error=native_error,
                ready_maps=len(ready),maps=outcomes,targeted_resource_collection=sorted(set(failures)),
                pending_resources=pending)
    write(output/'result.json',result)
    return result


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    commands=parser.add_subparsers(dest='command',required=True)
    create=commands.add_parser('prepare')
    create.add_argument('--catalog',type=Path,required=True)
    create.add_argument('--inventory',type=Path,required=True)
    create.add_argument('--output',type=Path,required=True)
    check=commands.add_parser('verify')
    check.add_argument('--queue',type=Path,required=True)
    check.add_argument('--output',type=Path,required=True)
    check.add_argument('--experiment',type=Path)
    check.add_argument('--maps',nargs='+')
    check.add_argument('--direction',choices=('left','right','auto'),default='auto')
    args=parser.parse_args()
    if args.command=='prepare':
        result=prepare(args.catalog.resolve(),args.inventory.resolve(),args.output.resolve())
        print(json.dumps(dict(configured_maps=len(result['maps']),ready=sum(e['status']=='ready' for e in result['maps']))))
    else:
        result=verify(args.queue.resolve(),args.output.resolve(),args.experiment.resolve() if args.experiment else None,
                      args.maps,args.direction)
        print(json.dumps({k:result[k] for k in ('ready_server_passed','ready_maps','targeted_resource_collection','pending_resources')}))
        if result['targeted_resource_collection'] or result['native_error']:raise SystemExit(1)


if __name__=='__main__':
    main()
