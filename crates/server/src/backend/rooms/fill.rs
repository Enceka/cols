//! Optional lobby Bot seats; real authenticated players replace them on join.
use super::*;
use crate::backend::rooms::bots::FillPolicy;

fn deliver(channel: &Channel, actor: Identity, body: &[u8], out: &mut TaskResult) {
    for human in channel.humans.values() {
        out.push_to(actor, human.identity, body.to_vec());
    }
}
fn bot(client: u8) -> Human {
    let uid = codol_protocol::room::messages::bot_id(client);
    Human {
        identity: Identity {
            title: 0,
            user: uid,
            seed: 0,
        },
        expires: u32::MAX,
        pending_until: None,
        name: STANDARD.encode(codol_protocol::room::messages::bot_name(client)),
        key: [0; 24],
    }
}
pub(super) fn remove_bot(
    channel: &mut Channel,
    id: u64,
    client: u8,
    actor: Identity,
    out: &mut TaskResult,
) {
    channel.fill_bots.remove(&client);
    channel.fill_sent.remove(&client);
    channel
        .member_teams
        .remove(&codol_protocol::room::messages::bot_id(client));
    let message = channel.membership_event(id, &bot(client), false);
    deliver(channel, actor, &message, out);
}
fn message(
    channel: &Channel,
    id: u64,
    client: u8,
    record: &[u8],
    actor: Identity,
    out: &mut TaskResult,
) -> Result<()> {
    for human in channel.humans.values() {
        let mut w = Writer::default();
        w.u32(7);
        w.u64(codol_protocol::room::messages::bot_id(client));
        w.string(&STANDARD.encode(codol_protocol::room::messages::bot_name(client)));
        w.u64(id);
        w.blob(&codol_protocol::room::messages::to_member(
            record,
            human.identity.user,
        )?)?;
        w.u32(1);
        out.push_to(actor, human.identity, w.0);
    }
    Ok(())
}
pub(super) fn sync(
    channel: &mut Channel,
    id: u64,
    fill: &FillPolicy,
    actor: Identity,
    out: &mut TaskResult,
) -> Result<()> {
    if channel.humans.values().any(|h| h.pending_until.is_some()) {
        return Ok(());
    }
    let map = channel
        .bot_selection(&fill.policy)
        .filter(|(_, mode)| mode == "war")
        .map(|(map, _)| map.clone());
    let wanted = if map.is_some() {
        usize::from(fill.target).saturating_sub(channel.humans.len())
    } else {
        0
    };
    while channel.fill_bots.len() > wanted {
        let client = *channel.fill_bots.iter().next_back().unwrap();
        remove_bot(channel, id, client, actor, out);
    }
    while channel.fill_bots.len() < wanted {
        let client = (1..=11)
            .find(|c| !channel.fill_bots.contains(c))
            .ok_or_else(|| anyhow::anyhow!("bot fill exceeds native seats"))?;
        channel.fill_bots.insert(client);
        let joined = channel.membership_event(id, &bot(client), true);
        deliver(channel, actor, &joined, out);
        // Only the real host accepts the native JoinRequest and assigns a team.
        let record = codol_protocol::room::messages::bot_join(client, fill.policy.version)?;
        let mut w = Writer::default();
        w.u32(7);
        w.u64(codol_protocol::room::messages::bot_id(client));
        w.string(&STANDARD.encode(codol_protocol::room::messages::bot_name(client)));
        w.u64(id);
        w.blob(&codol_protocol::room::messages::to_member(
            &record,
            channel.owner,
        )?)?;
        w.u32(1);
        out.push_to(actor, channel.humans[&channel.owner].identity, w.0);
    }
    let Some(map) = map else {
        return Ok(());
    };
    if !channel.roster_requested
        && let Some(client) = channel.fill_bots.iter().copied().find(|c| {
            channel
                .member_teams
                .contains_key(&codol_protocol::room::messages::bot_id(*c))
        })
    {
        out.push_to(
            actor,
            channel.humans[&channel.owner].identity,
            super::bot_message(
                client,
                id,
                channel.owner,
                &codol_protocol::room::messages::roster_request(),
            )?,
        );
        channel.roster_requested = true;
    }
    for client in channel.fill_bots.clone() {
        let uid = codol_protocol::room::messages::bot_id(client);
        let Some(team @ (1 | 2)) = channel.member_teams.get(&uid).copied() else {
            continue;
        };
        let signature = (map.clone(), team, channel.owner_ready);
        if channel.fill_sent.get(&client) == Some(&signature) {
            continue;
        }
        message(
            channel,
            id,
            client,
            &codol_protocol::room::messages::bot_ready(channel.owner_ready),
            actor,
            out,
        )?;
        if let Some(loadouts) = fill.policy.loadouts.get(&(map.clone(), "war".into())) {
            message(
                channel,
                id,
                client,
                &loadouts[usize::from(client - 1)][usize::from(2 - team)].record(team)?,
                actor,
                out,
            )?;
        }
        channel.fill_sent.insert(client, signature);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepted_bot_requests_the_hosts_complete_roster_once() {
        let mut rooms = Rooms::default();
        let owner = Session {
            title_id: 1002,
            user_id: 42,
            iv_seed: 1,
            expires_at: 1000,
            key: [0; 24],
        };
        let (id, _) = super::super::directory_tests::room(&mut rooms, &owner, 12);
        let c = rooms.channels.get_mut(&id).unwrap();
        c.room_info = Some(codol_protocol::room::messages::RoomInfo {
            mode: "war".into(),
            map: "mp_afghan".into(),
            playlist: 255,
            minimum: 4,
            maximum: 12,
        });
        c.fill_bots.extend([1, 2, 3]);
        c.member_teams
            .insert(codol_protocol::room::messages::bot_id(1), 2);
        let policy = FillPolicy {
            target: 4,
            policy: crate::backend::rooms::bots::Policy {
                version: 7,
                maps: std::collections::BTreeSet::from([("mp_afghan".into(), "war".into())]),
                loadouts: Default::default(),
            },
        };
        let mut result = TaskResult::success("test", vec![]);
        sync(c, id, &policy, Identity::from(&owner), &mut result).unwrap();
        assert!(c.roster_requested);
        assert_eq!(result.pushes.len(), 2);
        let first = result.pushes.len();
        sync(c, id, &policy, Identity::from(&owner), &mut result).unwrap();
        assert_eq!(result.pushes.len(), first);
    }
}
