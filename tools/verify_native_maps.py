#!/usr/bin/env python3
"""Reuse an owned client/room to sample a prepared map catalog without screenshots.

Use a separate diagnostic catalog with short score/time limits and isolated
service data. The supplied running experiment must use that exact catalog.
The tool posts only ordinary, bounded UI/game inputs through game-window.
"""
import argparse
import concurrent.futures
import json
import math
from pathlib import Path
import subprocess
import struct
import sys
import time

from clientlib import REPO


class Client:
    def __init__(self, experiment, probe_dir=None, window_binary=None):
        self.experiment = experiment.resolve()
        self.window_binary = window_binary or REPO/".tools/game-window"
        self.probe_args = ["--probe-dir", str(probe_dir)] if probe_dir else []

    def state(self):
        result = subprocess.run([sys.executable, str(REPO/'tools/client_state.py'),
                                 '--experiment', str(self.experiment), *self.probe_args], text=True,capture_output=True)
        if result.returncode:
            raise subprocess.CalledProcessError(result.returncode,result.args,
                                                output=result.stdout,stderr=result.stderr)
        return json.loads(result.stdout)['state']

    def action(self, kind, *args):
        return json.loads(subprocess.check_output([str(self.window_binary),
            str(self.experiment), kind, *map(str, args)], text=True))

    def click(self, x, y, content=True):
        b = self.action('info')['bounds']
        h, canvas = b['Height'], min(b['Height'], b['Width']*.625)
        fraction=(h-canvas+canvas*y)/h if content else y
        self.last_click=dict(bounds=b,fraction=[x,fraction],content=content)
        self.last_click['receipt']=self.action('click', x, fraction)
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

    def capture(self, mode, ready=None):
        for attempt in range(3):
            command=[sys.executable,str(REPO/'tools/client_state.py'),
                '--experiment',str(self.experiment),'--'+mode, *self.probe_args]
            if ready is not None:command+=['--sample-ready',str(ready)]
            result = subprocess.run(command,text=True,capture_output=True)
            if result.returncode==0:
                output=json.loads(result.stdout)['output']
                return output,json.loads(Path(output).read_text())
            if ready is not None:break
            if attempt<2:
                # Streaming models and the final replay change pointers during
                # reads. Retry bounded read-only captures, never change resources.
                time.sleep(1)
        raise RuntimeError('native '+mode+' capture failed: '+result.stderr[-600:])

    def create_room(self, mode='war'):
        if mode not in ('war', 'cha'):
            raise ValueError('unsupported native team room mode')
        self.click(.297,.05);self.click(.14,.125)
        self.click(.437,.255)
        if mode == 'cha':
            self.action('scroll',.25,.70,-7)
            self.click(.22,.66)
        self.click(.763,.854)
        return self.wait(lambda s:s['room_member_count']>0 and s['room_is_host']==1
                         and s['room_native_mode']==mode,15)

    def return_to_room(self):
        # Connection state0 arrives before the after-action button accepts input.
        # AAR also retains the host controller and its twelve members. Confirm
        # the room's actual controls with an arrow round trip before reusing it.
        def room(state):
            return (state['connection_state']==0 and state['frontend_state']==5
                    and state['room_is_host']==1 and state['room_member_count']==12)
        def lobby(state):
            return (state['connection_state']==0 and state['frontend_state']==5
                    and state['room_is_host']==0 and state['room_member_count']==0
                    and state.get('room_channel_pointer')==0
                    and state.get('room_join_request_handler')==0
                    and state.get('active_inventory_count',0)>0)
        self.return_recreated_room=False
        time.sleep(2)
        for attempt in range(3):
            current=self.state()
            if lobby(current):
                selected=current['room_native_map']
                self.create_room(current.get('room_native_mode','war'))
                self.select_map(selected,'auto')
                self.return_recreated_room=True
            else:
                self.click(.765,.91,False)
            try:
                before=self.wait(lambda s:room(s) or lobby(s),4)
            except RuntimeError:
                if attempt==2:raise
                continue
            if lobby(before):continue
            selected=before['room_native_map']
            for outward,back in [(.878,.707),(.707,.878)]:
                self.click(outward,.769)
                after=self.state()
                if not room(after):
                    raise RuntimeError('native client left room while confirming after-action return')
                if after['room_native_map']!=selected:
                    self.click(back,.769)
                    return self.wait(lambda s:room(s) and s['room_native_map']==selected,4)
            time.sleep(2)
        raise RuntimeError('native room controls unavailable after after-action return')

    def select_map(self, target, direction='right'):
        # Use the center of each native chevron, away from its narrow tips.
        arrows={'right':.878,'left':.707}
        if direction not in (*arrows,'auto'):
            raise ValueError('native map direction must be left, right or auto')
        # Observed native room order. This is only a routing hint: actual map
        # changes and membership are checked after every normal arrow input.
        observed=['mp_firingrange','mp_cargoship','mp_subbase_sh','mp_bog_sh','mp_crash_tropical',
                  'mp_cross_fire','mp_estate_tropical','mp_fav_tropical','mp_nuked','mp_killhouse_sh',
                  'mp_overgrown','mp_rust_long','mp_highrise','mp_storm_spring','mp_mountain',
                  'mp_terminal_cls','mp_abandon_sh_mk','mp_seatown','mp_ambush_sh','mp_afghan']
        observed_charge=['mp_highrise','mp_mountain','mp_terminal_cls','mp_ambush_sh',
                         'mp_subbase_sh','mp_boomtown','mp_nuked','mp_rust_long',
                         'mp_shipment_long','mp_cha_quad','mp_mideast']
        seen=set()
        member_count=None
        selected=direction
        reversed_once=False
        target_reannounced=False
        selection_started=time.monotonic()
        trace=dict(target=target,direction=direction,screenshots=0,steps=[])
        trace_path=(self.experiment.parent/('map-selection-'+str(time.time_ns())+'.json')) if hasattr(self,'experiment') else None
        def record(state, event):
            trace['steps'].append(dict(event=event,elapsed_seconds=round(time.monotonic()-selection_started,3),**{k:state.get(k) for k in
                ('connection_state','frontend_state','room_native_map','room_native_mode','room_member_count','room_is_host',
                 'mouse_raw_x_0','mouse_raw_y_0','mouse_raw_x_1','mouse_raw_y_1','mouse_raw_bucket')},
                members=[{k:m.get(k) for k in ('slot','bot_client','ready','dlc_level')}
                         for m in state.get('room_members',[])],
                last_click=getattr(self,'last_click',None)))
            if trace_path:trace_path.write_text(json.dumps(trace,indent=2)+'\n')
        def target_started(state):
            return (state.get('room_native_map')==target
                    and state.get('room_member_count')==12
                    and 3<=state.get('connection_state',0)<=10)
        for _ in range(80):
            state=self.state()
            record(state,'room')
            if target_started(state):return state
            if state.get('connection_state',0)!=0:
                raise RuntimeError('native client left room during map selection: '+json.dumps(
                    {k:state.get(k) for k in ('connection_state','room_native_map','room_member_count')}))
            current=state['room_native_map']
            if state['room_member_count']!=member_count:
                # Room population changes during native Bot admission. A map
                # seen before the join is not proof of a stable selector cycle.
                seen.clear()
                member_count=state['room_member_count']
            print('select',current,state['room_member_count'],flush=True)
            if current==target:
                if state['room_member_count']==1 and not target_reannounced:
                    # Creation can install the room receiver without sending
                    # its first broadcast. A native arrow round trip publishes
                    # the selected map and allows the server's Bot admission.
                    target_reannounced=True
                    for outward,back in [('right','left'),('left','right')]:
                        self.click(arrows[outward],.769)
                        after=self.state();record(after,'announce-away-'+outward)
                        if after.get('connection_state',0)!=0:
                            raise RuntimeError('native client left room while announcing target map')
                        if after['room_native_map']!=target:
                            self.click(arrows[back],.769)
                            after=self.state();record(after,'announce-return-'+back)
                            if target_started(after):return after
                            if after.get('connection_state',0)!=0:
                                raise RuntimeError('native client left room while returning to target map')
                            break
                    # Admission can rebuild the native map list between the
                    # outward and return inputs. Route from the actual result.
                    continue
                return self.wait(lambda s:s['room_native_map']==target
                                 and s['room_member_count']==12
                                 and (s.get('connection_state',0)==0 or target_started(s)),20)
            if current in seen:
                if direction!='auto' or reversed_once:
                    raise RuntimeError('map selector cycled without finding the prepared map')
                selected='left' if selected=='right' else 'right'
                reversed_once=True
                seen.clear()
                record(state,'reverse-after-cycle')
            seen.add(current)
            # Choose one direction for this cycle. Recomputing it from an
            # incomplete order can bounce between adjacent maps forever.
            if selected=='auto':
                order=observed_charge if state.get('room_native_mode')=='cha' else observed
                if current in order and target in order:
                    a,b=order.index(current),order.index(target)
                    selected='right' if (b-a)%len(order)<=(a-b)%len(order) else 'left'
                else:selected='right'
            # A same-map result can be a pending UI transition, not a full cycle.
            for attempt in range(3):
                self.click(arrows[selected],.769)
                after=self.state()
                record(after,'arrow-'+selected)
                if target_started(after):return after
                if after.get('connection_state',0)!=0:
                    raise RuntimeError('native client left room during map selection: '+json.dumps(
                        {k:after.get(k) for k in ('connection_state','room_native_map','room_member_count')}))
                if after['room_native_map']!=current:
                    break
            else:
                if direction!='auto' or reversed_once:
                    raise RuntimeError('native map arrow did not change the room map')
                selected='left' if selected=='right' else 'right'
                reversed_once=True
                seen.clear()
                record(after,'reverse-at-boundary')
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

    def sampled_movement(self):
        attempts=[]
        for key in ('w','d','s'):
            ready=self.experiment.parent/('movement-ready-'+str(time.time_ns())+'.json')
            with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
                sampling=pool.submit(self.capture,'replay',ready)
                deadline=time.monotonic()+20
                while not ready.exists():
                    if sampling.done():
                        sampling.result()
                        raise RuntimeError('native movement sampling ended before readiness')
                    if time.monotonic()>=deadline:
                        raise RuntimeError('native movement sampling readiness timeout')
                    time.sleep(.025)
                receipt=self.action('hold',key,.7)
                path,data=sampling.result()
            result=movement_frames(data['frames'])
            attempts.append(dict(key=key,sample=path,input_receipt=receipt,same_life=bool(result)))
            if result is not None:
                attempts[-1].update(result)
                return result['distance_units'],attempts
        raise RuntimeError('continuous native frames did not establish same-life movement')


def origin(state):
    values=[struct.unpack('<f',struct.pack('<I',state['predicted_origin_'+axis+'_bits']))[0]
            for axis in ('x','y','z')]
    if not all(math.isfinite(value) for value in values):
        raise RuntimeError('non-finite native player position')
    return values


def movement_frames(frames):
    """Require bounded XY displacement within one ordinary, living PS segment."""
    anchor=None
    for frame in frames:
        if (frame.get('active') or frame.get('client')!=0 or frame.get('pm_type')!=0
                or frame.get('health',0)<=0 or not frame.get('stable_command')):
            anchor=None
            continue
        point=frame['origin']
        if len(point)!=3 or not all(math.isfinite(x) for x in point):
            anchor=None
            continue
        if (anchor is None or frame['spawn_count']!=anchor['spawn_count']
                or frame['command_time']<anchor['command_time']
                or frame['command_time']-anchor['command_time']>1000):
            anchor=frame
            continue
        if frame['command_time']==anchor['command_time']:
            continue
        distance=math.dist(anchor['origin'][:2],point[:2])
        if distance>128:
            anchor=frame
            continue
        if distance>=8:
            return dict(distance_units=round(distance,2),spawn_count=frame['spawn_count'],
                before=anchor['origin'],after=point,command_times=[anchor['command_time'],frame['command_time']])
    return None


def require_completed_gameplay(row):
    if row['live_frames']==0:
        raise RuntimeError('no ordinary gameplay frames sampled')
    if row['intermission_frames']==0:
        raise RuntimeError('no completed-match frames sampled; retained after-action data may be stale')


def prepare(catalog_path, output):
    catalog = json.loads(catalog_path.read_text())
    output.mkdir(parents=True, exist_ok=False)
    for entry in catalog['entries']:
        source = (catalog_path.parent/entry['file']).resolve()
        state = json.loads(source.read_text())
        if state['map'] != entry['map'] or state['mode'] != entry['mode']:
            raise ValueError('catalog map identity mismatch')
        for key in ('arsenal_file','collision_file','appearance_file','voice_file','nuke_file','radar_file','props_file'):
            if state.get(key):
                state[key] = str((source.parent/state[key]).resolve())
        state['match_config'].update(score_limit=1000, time_limit_msec=60000)
        name = entry['map']+('-cha' if entry['mode']=='cha' else '')+'.json'
        (output/name).write_text(json.dumps(state,indent=2)+'\n')
        entry['file'] = name
    path = output/'catalog.json'
    path.write_text(json.dumps(catalog,indent=2)+'\n')
    return path


def run(catalog_path, experiment, output, map_names=None, map_direction='right', quick_playlist=None, native_mode=None):
    info = json.loads(experiment.read_text())
    argv = info['tls_argv']
    used = Path(argv[argv.index('--game-map-catalog')+1]).resolve()
    service = Path(argv[argv.index('--service-data-dir')+1]).resolve()
    if used != catalog_path.resolve() or service.is_relative_to((REPO/'client-work/server-data').resolve()):
        raise ValueError('native batch requires this catalog and isolated service data')
    if not any(service.is_relative_to((REPO/root).resolve()) for root in ('client-work','research-output')) or info.get('finished_unix'):
        raise ValueError('native batch requires a running project-owned diagnostic experiment')
    if info.get('resource_capture_map'):
        raise ValueError('resource collection cannot substitute for native gameplay')
    if quick_playlist is not None and quick_playlist != 0:
        raise ValueError('quick native check requires the measured single-entry bot playlist0')
    catalog = json.loads(catalog_path.read_text())
    if catalog.get('schema_version')!=1 or not catalog.get('entries'):
        raise ValueError('unsupported or empty diagnostic catalog')
    if native_mode is not None:
        if native_mode not in ('war','cha'):
            raise ValueError('unsupported native team mode')
        catalog['entries']=[e for e in catalog['entries'] if e['mode']==native_mode]
        if not catalog['entries']:
            raise ValueError('requested mode is absent from the diagnostic catalog')
    if map_names:
        known={e['map'] for e in catalog['entries']}
        if not set(map_names)<=known:raise ValueError('requested map is absent from the diagnostic catalog')
        entries=catalog['entries']
        catalog['entries']=[e for name in dict.fromkeys(map_names) for e in entries if e['map']==name]
    if len({e['mode'] for e in catalog['entries']})!=1:
        raise ValueError('native room check requires one mode; select --mode war or cha')
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
        row=dict(map=entry['map'],mode=entry['mode'],passed=False,errors=[])
        print('native map',entry['map'],flush=True)
        try:
            source=json.loads((catalog_path.parent/entry['file']).read_text())
            player_count=source['match_config'].get('player_count',12)
            if source['match_config']['time_limit_msec']>60000:
                raise ValueError('native batch requires short diagnostic limits')
            state=client.wait(lambda s:(s['connection_state']==0 and s['frontend_state']==5
                and s['active_inventory_count']>0) or (s['connection_state']>=3
                and s['room_native_map']==entry['map'] and s['room_native_mode']==entry['mode']),100)
            if state['connection_state']>=3:
                row['resumed_native_loading']=True
            else:
                if quick_playlist is not None:
                    if state['room_member_count']:
                        raise ValueError('single-entry quick check requires a fresh owned client')
                    client.click(.297,.05);client.click(.14,.125);client.click(.156,.255)
                    client.click(.215,.328);client.click(.763,.854)
                    state=client.wait(lambda s:s['room_native_map']==entry['map'] and s['room_native_playlist']==0
                        and (s['room_member_count']>0 or s['connection_state']>=6),30)
                    row['room_kind']='native_quick_playlist'
                elif state['room_member_count']==0:
                    state=client.create_room(entry['mode'])
                if quick_playlist is None and state['room_native_mode'] != entry['mode']:
                    raise RuntimeError('native room mode differs from preparation')
                print('room',state['room_native_map'],state['room_member_count'],flush=True)
                if quick_playlist is None:
                    state=client.select_map(entry['map'],map_direction)
                if state['connection_state']==0:client.click(.848,.916)
            live=client.wait(lambda s:s['connection_state']==10 and s['current_snapshot_valid']==1,300)
            if live['net_constants_crc']!=source['ncs_crc']:
                raise RuntimeError('native map CRC differs from preparation')
            row['ncs_crc']=live['net_constants_crc']
            row['initial_state']={k:live[k] for k in ('predicted_health','predicted_weapon','room_member_count')}
            if source['match_config'].get('preparation_msec',0):
                client.wait(lambda s:s['connection_state']==10 and not s['predicted_pm_flags'] & 0x800,15)
            if player_count==4:
                distance,attempts=client.sampled_movement()
            else:
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
            if row['native_characters']!=player_count:
                raise RuntimeError('native character models or animation trees missing')
            if characters['compass_name']!=source['minimap']['material']:
                raise RuntimeError('native compass material differs from preparation')
            material=characters['compass_material']
            row['compass_material']=material
            if not material.get('found') or material.get('render_default',True) or not any(
                    t['width']>16 and t['height']>16 and t['texture'] for t in material.get('textures',[])):
                raise RuntimeError('native compass uses a missing/default texture')
            row.update(samples=[],sample_frames=0,live_frames=0,replay_frames=0,intermission_frames=0)
            # Continuous movement already sampled this real match. A short
            # arena can end before the later loop; retain and count its frames.
            for attempt in attempts:
                if not attempt.get('sample'):
                    continue
                frames=json.loads(Path(attempt['sample']).read_text())
                row['samples'].append(attempt['sample'])
                row['sample_frames']+=len(frames['frames'])
                row['live_frames']+=sum(f['pm_type']==0 and not f['active'] for f in frames['frames'])
                row['replay_frames']+=sum(bool(f['active']) for f in frames['frames'])
                row['intermission_frames']+=sum(f['pm_type']==6 and not f['active'] for f in frames['frames'])
            deadline=time.monotonic()+130
            while client.state()['connection_state']==10 and time.monotonic()<deadline:
                frame_path,frames=client.capture('replay')
                row['samples'].append(frame_path)
                row['sample_frames']+=len(frames['frames'])
                row['live_frames']+=sum(f['pm_type']==0 and not f['active'] for f in frames['frames'])
                row['replay_frames']+=sum(bool(f['active']) for f in frames['frames'])
                row['intermission_frames']+=sum(f['pm_type']==6 and not f['active'] for f in frames['frames'])
            require_completed_gameplay(row)
            finished=client.wait(lambda s:s['connection_state']==0 and s['frontend_state']==5,10)
            row['scoreboard_rows']=finished['ui_scoreboard_row_count']
            row['aar']={k:finished[k] for k in ('aar_map_id','aar_mode_id','aar_duration',
                        'aar_winning_team','aar_team_score_1','aar_team_score_2','aar_rows')}
            rows=[r for r in finished['aar_rows'] if r['name']]
            registry_name='charge-map-registry-3.3.3.3.json' if entry['mode']=='cha' else 'map-registry-3.3.3.3.json'
            registry=json.loads((REPO/'fixtures/protocol'/registry_name).read_text())
            native_ids={r['native_id']:r['name'] for r in registry['maps']}
            if native_ids.get(finished['aar_map_id'])!=entry['map']:
                raise RuntimeError('native after-action map ID mismatch')
            if finished['aar_mode_id'] != {'war':1,'cha':15}[entry['mode']]:
                raise RuntimeError('native after-action mode ID mismatch')
            if row['scoreboard_rows']!=player_count or len(rows)!=player_count or sum(r['is_local'] for r in rows)!=1:
                raise RuntimeError('native after-action roster is incomplete or repeats the local player')
            if sum(r['kills'] for r in rows)==0:
                raise RuntimeError('native after-action contains no combat kills')
            if quick_playlist is None:
                client.return_to_room()
                row['room_recreated_after_action']=client.return_recreated_room
            else:
                row['room_reused_after_action']=False
                row['return_scope']='quick playlist has native voting; each map uses a fresh client'
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
    parser.add_argument('--mode',choices=('war','cha'),help='select one native team mode from a mixed catalog')
    parser.add_argument('--direction',choices=('right','left','auto'),default='auto',
                        help='native arrow direction for the selected map sequence')
    args=parser.parse_args()
    if args.prepare:
        if args.experiment or args.output:parser.error('prepare is separate from native execution')
        print(prepare(args.catalog.resolve(),args.prepare.resolve()))
    else:
        if not args.experiment or not args.output or args.output.exists():
            parser.error('native run needs experiment and a new result path')
        result=run(args.catalog.resolve(),args.experiment.resolve(),args.output.resolve(),args.maps,args.direction,
                   native_mode=args.mode)
        print(json.dumps({'maps':len(result['maps']),'passed':result['passed'],
                          'seconds':result['elapsed_seconds']}))


if __name__=='__main__':
    main()
