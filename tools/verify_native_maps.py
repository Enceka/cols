#!/usr/bin/env python3
"""Reuse an owned client/room to sample a prepared map catalog without screenshots.

Use a separate diagnostic catalog with short score/time limits and isolated
service data. The supplied running experiment must use that exact catalog.
The tool posts only ordinary, bounded UI/game inputs through game-window.
"""
import argparse
import json
import math
from pathlib import Path
import subprocess
import struct
import sys
import time

from clientlib import REPO


class Client:
    def __init__(self, experiment):
        self.experiment = experiment.resolve()

    def state(self):
        result = subprocess.run([sys.executable, str(REPO/'tools/client_state.py'),
                                 '--experiment', str(self.experiment)], text=True,capture_output=True)
        if result.returncode:
            raise subprocess.CalledProcessError(result.returncode,result.args,
                                                output=result.stdout,stderr=result.stderr)
        return json.loads(result.stdout)['state']

    def action(self, kind, *args):
        return json.loads(subprocess.check_output([str(REPO/'.tools/game-window'),
            str(self.experiment), kind, *map(str, args)], text=True))

    def click(self, x, y, content=True):
        b = self.action('info')['bounds']
        h, canvas = b['Height'], min(b['Height'], b['Width']*.625)
        self.action('click', x, (h-canvas+canvas*y)/h if content else y)
        time.sleep(1.1)

    def wait(self, predicate, seconds):
        deadline = time.monotonic()+seconds
        state={}
        while time.monotonic()<deadline:
            try:
                state = self.state()
            except subprocess.SubprocessError as error:
                # Startup and map teardown release the probe's current context.
                # The enclosing deadline bounds retries through those changes.
                info=json.loads(self.experiment.read_text())
                if info.get('finished_unix'):
                    raise RuntimeError('owned client ended: '+str(info.get('harness_error','client exited'))) from error
                time.sleep(.25)
                continue
            if predicate(state):
                return state
            time.sleep(.25)
        raise RuntimeError('native state timeout: '+json.dumps({k:state.get(k) for k in
            ('connection_state','frontend_state','room_native_map','room_member_count')}))

    def capture(self, mode):
        for attempt in range(3):
            result = subprocess.run([sys.executable,str(REPO/'tools/client_state.py'),
                '--experiment',str(self.experiment),'--'+mode],text=True,capture_output=True)
            if result.returncode==0:
                output=json.loads(result.stdout)['output']
                return output,json.loads(Path(output).read_text())
            if attempt<2:
                # Streaming models and the final replay change pointers during
                # reads. Retry bounded read-only captures, never change resources.
                time.sleep(1)
        raise RuntimeError('native '+mode+' capture failed: '+result.stderr[-600:])

    def return_to_room(self):
        # Connection state0 arrives before the after-action button accepts input.
        # Scoreboard rows are retained in the lobby, so they are not a menu flag.
        time.sleep(2)
        for attempt in range(3):
            self.click(.765,.91,False)
            try:
                return self.wait(lambda s:s['connection_state']==0 and s['frontend_state']==5
                                 and s['room_is_host']==1 and s['room_member_count']==12,4)
            except RuntimeError:
                if attempt==2:raise

    def select_map(self, target, direction='right'):
        arrows={'right':.886,'left':.698}
        if direction not in (*arrows,'auto'):
            raise ValueError('native map direction must be left, right or auto')
        # Observed native room order. This is only a routing hint: actual map
        # changes and membership are checked after every normal arrow input.
        observed=['mp_firingrange','mp_cargoship','mp_subbase_sh','mp_bog_sh','mp_crash_tropical',
                  'mp_cross_fire','mp_estate_tropical','mp_fav_tropical','mp_nuked','mp_killhouse_sh',
                  'mp_overgrown','mp_rust_long','mp_highrise','mp_storm_spring','mp_mountain',
                  'mp_terminal_cls','mp_abandon_sh_mk','mp_seatown','mp_ambush_sh','mp_afghan']
        seen=set()
        member_count=None
        selected=direction
        for _ in range(60):
            state=self.state()
            current=state['room_native_map']
            if state['room_member_count']!=member_count:
                # Room population changes during native Bot admission. A map
                # seen before the join is not proof of a stable selector cycle.
                seen.clear()
                member_count=state['room_member_count']
            print('select',current,state['room_member_count'],flush=True)
            if current==target:
                return self.wait(lambda s:s['room_native_map']==target
                                 and s['room_member_count']==12,20)
            if current in seen:
                raise RuntimeError('map selector cycled without finding the prepared map')
            seen.add(current)
            # Choose one direction for this cycle. Recomputing it from an
            # incomplete order can bounce between adjacent maps forever.
            if selected=='auto':
                if current in observed and target in observed:
                    a,b=observed.index(current),observed.index(target)
                    selected='right' if (b-a)%len(observed)<=(a-b)%len(observed) else 'left'
                else:selected='right'
            # A same-map result can be a pending UI transition, not a full cycle.
            for attempt in range(3):
                self.click(arrows[selected],.765)
                after=self.state()
                if after['room_native_map']!=current:
                    break
            else:
                raise RuntimeError('native map arrow did not change the room map')
        raise RuntimeError('native map selection incomplete')

    def movement(self):
        attempts=[]
        # Spawn headings can face a wall. Check another ordinary direction
        # before classifying a stationary sample as a stuck player.
        for key in ('w','d','s'):
            before=self.wait(lambda s:s['connection_state']==10 and s['predicted_pm_type']==0
                            and s['predicted_health']>0,15)
            start=origin(before)
            self.action('hold',key,.7)
            after=self.state()
            same_life=before['predicted_spawn_count']==after['predicted_spawn_count']
            distance=math.dist(start[:2],origin(after)[:2])
            attempts.append(dict(key=key,before=start,after=origin(after),
                                 same_life=same_life,distance_units=round(distance,2)))
            if after['connection_state']==10 and after['predicted_pm_type']==0 and same_life and distance>=8:
                return distance,attempts
        raise RuntimeError('bounded movement did not establish movement in the same life: '+json.dumps(attempts))


def origin(state):
    values=[struct.unpack('<f',struct.pack('<I',state['predicted_origin_'+axis+'_bits']))[0]
            for axis in ('x','y','z')]
    if not all(math.isfinite(value) for value in values):
        raise RuntimeError('non-finite native player position')
    return values


def prepare(catalog_path, output):
    catalog = json.loads(catalog_path.read_text())
    output.mkdir(parents=True, exist_ok=False)
    for entry in catalog['entries']:
        source = (catalog_path.parent/entry['file']).resolve()
        state = json.loads(source.read_text())
        if state['map'] != entry['map'] or state['mode'] != entry['mode']:
            raise ValueError('catalog map identity mismatch')
        for key in ('arsenal_file','collision_file','appearance_file'):
            if state.get(key):
                state[key] = str((source.parent/state[key]).resolve())
        state['match_config'].update(score_limit=1000, time_limit_msec=60000)
        name = entry['map']+'.json'
        (output/name).write_text(json.dumps(state,indent=2)+'\n')
        entry['file'] = name
    path = output/'catalog.json'
    path.write_text(json.dumps(catalog,indent=2)+'\n')
    return path


def run(catalog_path, experiment, output, map_names=None, map_direction='right'):
    info = json.loads(experiment.read_text())
    argv = info['tls_argv']
    used = Path(argv[argv.index('--game-map-catalog')+1]).resolve()
    service = Path(argv[argv.index('--service-data-dir')+1]).resolve()
    if used != catalog_path.resolve() or service == (REPO/'client-work/server-data').resolve():
        raise ValueError('native batch requires this catalog and isolated service data')
    if not service.is_relative_to((REPO/'client-work').resolve()) or info.get('finished_unix'):
        raise ValueError('native batch requires a running project-owned diagnostic experiment')
    if info.get('resource_capture_map'):
        raise ValueError('resource collection cannot substitute for native gameplay')
    catalog = json.loads(catalog_path.read_text())
    if catalog.get('schema_version')!=1 or not catalog.get('entries'):
        raise ValueError('unsupported or empty diagnostic catalog')
    if map_names:
        known={e['map'] for e in catalog['entries']}
        if not set(map_names)<=known:raise ValueError('requested map is absent from the diagnostic catalog')
        entries={e['map']:e for e in catalog['entries']}
        catalog['entries']=[entries[name] for name in dict.fromkeys(map_names)]
    for entry in catalog['entries']:
        source=json.loads((catalog_path.parent/entry['file']).read_text())
        if source['map']!=entry['map'] or source['mode']!=entry['mode']:
            raise ValueError('diagnostic catalog map identity mismatch')
        limits=source['match_config']
        if not 10000<=limits['time_limit_msec']<=60000 or not 100<=limits['score_limit']<=1000:
            raise ValueError('native batch requires short diagnostic limits')
    client = Client(experiment)
    report = dict(schema_version=1,client_sha256=info['client_sha256'],native_client_used=True,
                  experiment=str(experiment.resolve()),screenshots=0,maps=[],
                  requested_maps=[entry['map'] for entry in catalog['entries']],complete=False,passed=False,
                  coverage='native loading, movement, character resources, combat and after-action data',
                  limitations=['full visual/audio acceptance is separate',
                               'replay/showcase samples are recorded but not required by this smoke check'])
    started=time.monotonic()
    for entry in catalog['entries']:
        map_started=time.monotonic()
        row=dict(map=entry['map'],passed=False,errors=[])
        print('native map',entry['map'],flush=True)
        try:
            source=json.loads((catalog_path.parent/entry['file']).read_text())
            if source['match_config']['time_limit_msec']>60000:
                raise ValueError('native batch requires short diagnostic limits')
            state=client.wait(lambda s:s['connection_state']==0 and s['frontend_state']==5
                and s['active_inventory_count']>0,100)
            if state['room_member_count']==0:
                client.click(.297,.05);client.click(.14,.125)
                client.click(.437,.255);client.click(.763,.854)
                state=client.wait(lambda s:s['room_member_count']>0,15)
            print('room',state['room_native_map'],state['room_member_count'],flush=True)
            client.select_map(entry['map'],map_direction)
            client.click(.848,.916)
            live=client.wait(lambda s:s['connection_state']==10 and s['current_snapshot_valid']==1,60)
            if live['net_constants_crc']!=source['ncs_crc']:
                raise RuntimeError('native map CRC differs from preparation')
            row['ncs_crc']=live['net_constants_crc']
            row['initial_state']={k:live[k] for k in ('predicted_health','predicted_weapon','room_member_count')}
            distance,attempts=client.movement()
            row['player_movement_units']=round(distance,2)
            row['movement_same_life']=True
            row['movement_attempts']=attempts
            path,characters=client.capture('map-check')
            row['characters_sample']=path
            row['native_characters']=sum(bool(c['dobj_models']) and c['animation_tree_ready']
                                         for c in characters['clients'])
            if characters['map']!='maps/mp/'+entry['map']+'.d3dbsp' or characters['ncs_crc']!=source['ncs_crc']:
                raise RuntimeError('native character resources belong to another map')
            if row['native_characters']!=12:
                raise RuntimeError('native character models or animation trees missing')
            if characters['compass_name']!=source['minimap']['material']:
                raise RuntimeError('native compass material differs from preparation')
            material=characters['compass_material']
            row['compass_material']=material
            if not material.get('found') or material.get('render_default',True) or not any(
                    t['width']>16 and t['height']>16 and t['texture'] for t in material.get('textures',[])):
                raise RuntimeError('native compass uses a missing/default texture')
            row.update(samples=[],sample_frames=0,live_frames=0,replay_frames=0,intermission_frames=0)
            deadline=time.monotonic()+130
            while client.state()['connection_state']==10 and time.monotonic()<deadline:
                frame_path,frames=client.capture('replay')
                row['samples'].append(frame_path)
                row['sample_frames']+=len(frames['frames'])
                row['live_frames']+=sum(f['pm_type']==0 and not f['active'] for f in frames['frames'])
                row['replay_frames']+=sum(bool(f['active']) for f in frames['frames'])
                row['intermission_frames']+=sum(f['pm_type']==6 and not f['active'] for f in frames['frames'])
            if row['live_frames']==0:
                raise RuntimeError('no ordinary gameplay frames sampled')
            finished=client.wait(lambda s:s['connection_state']==0 and s['frontend_state']==5,10)
            row['scoreboard_rows']=finished['ui_scoreboard_row_count']
            row['aar']={k:finished[k] for k in ('aar_map_id','aar_mode_id','aar_duration',
                        'aar_winning_team','aar_team_score_1','aar_team_score_2','aar_rows')}
            rows=[r for r in finished['aar_rows'] if r['name']]
            registry=json.loads((REPO/'fixtures/protocol/map-registry-3.3.3.3.json').read_text())
            native_ids={r['native_id']:r['name'] for r in registry['maps']}
            if native_ids.get(finished['aar_map_id'])!=entry['map']:
                raise RuntimeError('native after-action map ID mismatch')
            if row['scoreboard_rows']!=12 or len(rows)!=12 or sum(r['is_local'] for r in rows)!=1:
                raise RuntimeError('native after-action roster is incomplete or repeats the local player')
            if sum(r['kills'] for r in rows)==0:
                raise RuntimeError('native after-action contains no combat kills')
            client.return_to_room()
            row['passed']=True
        except (RuntimeError,ValueError,subprocess.SubprocessError,KeyError) as error:
            row['errors'].append(str(error))
            row['elapsed_seconds']=round(time.monotonic()-map_started,2)
            report['maps'].append(row)
            report['passed']=False
            report['elapsed_seconds']=round(time.monotonic()-started,2)
            output.write_text(json.dumps(report,indent=2)+'\n')
            raise
        row['elapsed_seconds']=round(time.monotonic()-map_started,2)
        report['maps'].append(row)
        report['complete']=len(report['maps'])==len(catalog['entries'])
        report['passed']=report['complete'] and all(r['passed'] for r in report['maps'])
        report['elapsed_seconds']=round(time.monotonic()-started,2)
        output.write_text(json.dumps(report,indent=2)+'\n')
        print(entry['map'],'passed',row['elapsed_seconds'],'seconds',flush=True)
    return report


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--catalog',type=Path,required=True)
    parser.add_argument('--prepare',type=Path,help='create a separate short-match diagnostic catalog')
    parser.add_argument('--experiment',type=Path,help='running owned client with the diagnostic catalog')
    parser.add_argument('--output',type=Path,help='new result JSON path')
    parser.add_argument('--maps',nargs='+',help='native room maps to run; other catalog maps stay unchecked')
    parser.add_argument('--direction',choices=('right','left','auto'),default='auto',
                        help='native arrow direction for the selected map sequence')
    args=parser.parse_args()
    if args.prepare:
        if args.experiment or args.output:parser.error('prepare is separate from native execution')
        print(prepare(args.catalog.resolve(),args.prepare.resolve()))
    else:
        if not args.experiment or not args.output or args.output.exists():
            parser.error('native run needs experiment and a new result path')
        result=run(args.catalog.resolve(),args.experiment.resolve(),args.output.resolve(),args.maps,args.direction)
        print(json.dumps({'maps':len(result['maps']),'passed':result['passed'],
                          'seconds':result['elapsed_seconds']}))


if __name__=='__main__':
    main()
