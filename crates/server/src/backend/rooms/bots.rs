//! Lobby participation for the eleven players simulated by TdmMatch.
use anyhow::{Result, ensure};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
pub(crate) struct Policy {
    pub version: u32,
    pub maps: BTreeSet<(String, String)>,
    /// Map-specific prepared roster; each pair is Axis/Allies, unlike room teams.
    pub loadouts: BTreeMap<(String, String), Vec<[codol_protocol::room::messages::Loadout; 2]>>,
}
impl Policy {
    pub fn accepts(&self, info: &codol_protocol::room::messages::RoomInfo) -> bool {
        info.playlist == 255
            && info.maximum == 12
            && info.minimum <= 12
            && self.maps.contains(&(info.map.clone(), info.mode.clone()))
    }
}

impl super::Channel {
    pub(super) fn sync_bot_loadouts(
        &mut self,
        policy: &Policy,
        channel_id: u64,
        force: bool,
    ) -> Result<Vec<Vec<u8>>> {
        if !self.bots_announced {
            return Ok(Vec::new());
        }
        let Some((map, mode)) = self.bot_selection(policy).cloned() else {
            return Ok(Vec::new());
        };
        let Some(roster) = policy.loadouts.get(&(map.clone(), mode.clone())) else {
            return Ok(Vec::new()); // Older preparations have no frontend item IDs.
        };
        ensure!(
            roster.len() == usize::from(codol_protocol::room::messages::BOT_CLIENTS),
            "room avatar roster must contain eleven bots"
        );
        let mut pushes = Vec::new();
        for client in 1..=codol_protocol::room::messages::BOT_CLIENTS {
            let uid = codol_protocol::room::messages::bot_id(client);
            // Only the host's member records establish acceptance and team.
            // Never infer them from JoinRequest order or from the game roster.
            let Some(&team @ (1 | 2)) = self.member_teams.get(&uid) else {
                self.avatar_sent.remove(&client);
                continue;
            };
            let loadout = roster[usize::from(client - 1)][usize::from(2 - team)];
            let identity = (map.clone(), mode.clone(), team, loadout);
            if !force && self.avatar_sent.get(&client) == Some(&identity) {
                continue;
            }
            pushes.push(super::bot_message(
                client,
                channel_id,
                self.owner,
                &loadout.record(team)?,
            )?);
            self.avatar_sent.insert(client, identity);
            tracing::debug!(
                client,
                team,
                map,
                character = loadout.clothing[0],
                weapon = loadout.weapons[0][0],
                "native room bot avatar queued"
            );
        }
        Ok(pushes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bots_only_join_supported_custom_tdm_with_the_real_roster_capacity() {
        let policy = Policy {
            version: 7,
            maps: BTreeSet::from([
                ("mp_afghan".into(), "war".into()),
                ("mp_killhouse_sh".into(), "war".into()),
            ]),
            loadouts: Default::default(),
        };
        let mut info = codol_protocol::room::messages::RoomInfo {
            mode: "war".into(),
            map: "mp_killhouse_sh".into(),
            playlist: 255,
            minimum: 4,
            maximum: 12,
        };
        assert!(policy.accepts(&info));
        info.playlist = 0;
        assert!(!policy.accepts(&info));
        info.playlist = 255;
        info.mode = "sd".into();
        assert!(!policy.accepts(&info));
        info.mode = "cha".into();
        assert!(!policy.accepts(&info)); // Preparing war on a map cannot admit cha.
        let mut charge = policy.clone();
        charge.maps.insert((info.map.clone(), "cha".into()));
        assert!(charge.accepts(&info));
        info.mode = "war".into();
        info.map = "mp_killhouse".into();
        assert!(!policy.accepts(&info));
        info.map = "mp_afghan".into();
        info.maximum = 18;
        assert!(!policy.accepts(&info));
        info.maximum = 12;
        info.minimum = 13;
        assert!(!policy.accepts(&info));
    }
}
