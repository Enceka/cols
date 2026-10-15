use super::*;
use crate::game::session::shared::SharedMatch;

impl GameServer {
    pub(super) async fn human_world(
        &self,
        admission: &Admission,
        template: &Arc<crate::game::session::prepared::PreparedGameState>,
    ) -> Result<Arc<Mutex<SharedMatch>>> {
        let services = self
            .services
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("human match requires account services"))?;
        let base = template.for_local_world();
        let mut registry =
            crate::game::session::weapon_registry::WeaponRegistry::new(base.first_weapon_index());
        let mut players = Vec::new();
        let mut members = admission.participants.clone();
        members.sort_by_key(|p| p.user_id);
        for (ordinal, member) in members.into_iter().enumerate() {
            let (name, mut state) = if let Some(client) = member.bot {
                (
                    codol_protocol::room::messages::bot_name(client),
                    base.shared_bot_profile(member.team, client, ordinal)?,
                )
            } else {
                let profile = services
                    .game_profile(admission.title, member.user_id)
                    .await?;
                let name = services
                    .player_nickname(crate::contracts::account::AccountId {
                        title_id: admission.title,
                        user_id: member.user_id,
                    })
                    .await?
                    .unwrap_or_else(|| format!("Player {}", ordinal + 1));
                let mut state =
                    base.human_profile(member.team, ordinal, &profile.public, &profile.owned)?;
                state.set_card_rank(profile.rank);
                (name, state)
            };
            state.register_history(&mut registry)?;
            players.push((member, name, state));
        }
        let models = players
            .iter()
            .flat_map(|(_, _, s)| s.model_configstrings())
            .collect::<Vec<_>>();
        let mut ready = Vec::new();
        for (member, name, mut state) in players {
            state.register_history(&mut registry)?;
            state.include_human_models(&models)?;
            ready.push((member, name, Arc::new(state)));
        }
        Ok(Arc::new(Mutex::new(SharedMatch::new(ready)?)))
    }
}

pub(super) fn challenge_user(payload: &[u8]) -> Result<u64> {
    let text = std::str::from_utf8(
        payload
            .strip_prefix(b"\xff\xff\xff\xffgetchallenge 0 \"")
            .ok_or_else(|| anyhow::anyhow!("human game identity requires challenge"))?,
    )?;
    let id = text
        .strip_suffix('\0')
        .unwrap_or(text)
        .strip_suffix('"')
        .ok_or_else(|| anyhow::anyhow!("invalid challenge identity"))?;
    ensure!(
        id.len() == 16 && id.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid human identity"
    );
    Ok(u64::from_str_radix(id, 16)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires isolated native test players and captured map via CODOL_HUMAN_LAB"]
    async fn captured_human_world_loads_two_backpacks_two_bots_and_encodes_all_native_views() {
        let lab = PathBuf::from(std::env::var_os("CODOL_HUMAN_LAB").expect("set CODOL_HUMAN_LAB"));
        let state = Arc::new(
            crate::game::session::prepared::PreparedGameState::load(&lab.join("map.json")).unwrap(),
        );
        let services = Arc::new(
            crate::app::services::LocalServices::open(lab.join("service-data"))
                .await
                .unwrap(),
        );
        let temp = tempfile::tempdir().unwrap();
        let server = GameServer::bind("127.0.0.1:0".parse().unwrap(), temp.path().join("udp"))
            .await
            .unwrap()
            .with_services(Some(services));
        let mut members = Vec::new();
        for (index, label) in ["a", "b"].iter().enumerate() {
            let identity: serde_json::Value = serde_json::from_slice(
                &std::fs::read(lab.join(format!("identity-{label}.json"))).unwrap(),
            )
            .unwrap();
            members.push(crate::contracts::match_session::Participant {
                user_id: identity["user_id"].as_str().unwrap().parse().unwrap(),
                seed: 1,
                team: index as u8 + 1,
                bot: None,
            });
        }
        for client in 1..=2 {
            members.push(crate::contracts::match_session::Participant {
                user_id: codol_protocol::room::messages::bot_id(client),
                seed: 0,
                team: client,
                bot: Some(client),
            });
        }
        let admission = Admission {
            title: 1002,
            owner: members[0].user_id,
            seed: 1,
            build: -688906341,
            expires: u32::MAX,
            key: [0; 16],
            booked: true,
            finished: false,
            playlist_version: None,
            start_command: None,
            room_teams: None,
            story_channel: None,
            participants: members.clone(),
        };
        let world = server.human_world(&admission, &state).await.unwrap();
        let mut world = world.lock().unwrap();
        for member in &members {
            if member.bot.is_none() {
                world.attach(member.user_id).unwrap();
                world.ready(member.user_id).unwrap();
            }
        }
        let now = std::time::Instant::now();
        world.advance(now).unwrap();
        world
            .advance(now + std::time::Duration::from_millis(50))
            .unwrap();
        for member in &members {
            let view = world.view(member.user_id).unwrap();
            let state = world.state(member.user_id).unwrap();
            let names = world.names(member.user_id).unwrap();
            let gamestate = state.body_roster(1, None, Some(&names)).unwrap();
            let snapshot = state
                .human_snapshot(&view, state.spawn_angles(), None, 0, false)
                .unwrap();
            assert!(!gamestate.is_empty());
            assert!(!snapshot.is_empty());
            assert_eq!(view.players.len(), 3);
            assert_eq!(view.scoreboard.rows[0].user_id, member.user_id);
            println!(
                "human native view: gamestate={} snapshot={} players={}",
                gamestate.len(),
                snapshot.len(),
                view.players.len() + 1
            );
        }
    }
}
