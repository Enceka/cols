use super::*;

impl Connection {
    pub(crate) fn refresh_profile(
        &mut self,
        template: &crate::game::session::prepared::PreparedGameState,
        public: &[u8],
        owned: &[u32],
        rank: (u8, u8),
    ) -> Result<()> {
        if !template.dynamic_loadout()
            || !self.gamestate_acknowledged
            || self.pending_loadout.is_some()
        {
            return Ok(());
        }
        let Some(current) = &self.world.game_state else {
            return Ok(());
        };
        let hash: [u8; 32] = Sha256::digest(public).into();
        if current.profile_hash == Some(hash)
            && current
                .player_card
                .is_none_or(|c| (c.rank, c.prestige) == rank)
            && self.requested_backpack.is_none()
        {
            return Ok(());
        }
        // Attribute resolution uses a temporary valid slot; the persistent
        // registry then assigns the client's canonical name/index identity.
        let slot = self.requested_backpack.or(self.active_backpack);
        let selected = slot
            .map(|slot| template.for_backpack(public, slot))
            .transpose()?;
        let mut source = selected.as_ref().unwrap_or(template).clone();
        source.preserve_match_appearance(current);
        let mut state = match source.for_profile_at(public, owned, template.first_weapon_index()) {
            Ok(state) => state,
            Err(error) if self.requested_backpack.is_some() => {
                self.requested_backpack = None;
                tracing::warn!(%error,"selected backpack is unavailable");
                self.feedback
                    .push_back("Selected backpack has no usable weapon".into());
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        state.set_card_rank(rank);
        state.preserve_match_appearance(current);
        self.active_backpack = slot;
        self.requested_backpack = None;
        tracing::info!(slot,global_class=?state.selected_backpack_class(),"selected in-game backpack");
        if current.selected_weapon_names() == state.selected_weapon_names() {
            let unchanged = current.same_spawn_equipment(&state);
            if current.player_card != state.player_card {
                self.world_history.clear();
            }
            Arc::make_mut(self.world.game_state.as_mut().unwrap()).update_profile_selection(&state);
            self.pending_loadout = None;
            if !unchanged {
                // Same guns need no new resource registration, but passive
                // skills and offhands still belong to the next life.
                self.next_spawn_loadout = Some(Arc::new(state));
                self.feedback
                    .push_back("Backpack will change on next spawn".into());
            } else if self.next_spawn_loadout.take().is_some() {
                self.feedback
                    .push_back("Current backpack kept for next spawn".into());
            }
            return Ok(());
        }
        let mut commands = self.server_commands.clone();
        let mut known = self.known_weapons.clone();
        let mut next = self.next_weapon;
        let shared = self.shared_registry.clone();
        let mut guard = shared
            .as_ref()
            .map(|r| {
                r.lock()
                    .map_err(|_| anyhow::anyhow!("weapon registry poisoned"))
            })
            .transpose()?;
        let mut registry = guard.as_ref().map_or_else(
            || crate::game::session::weapon_registry::WeaponRegistry {
                names: known.clone(),
                next,
            },
            |r| (**r).clone(),
        );
        state.register_history(&mut registry)?;
        known = registry.names.clone();
        next = registry.next;
        let additions = state
            .weapon_configstrings()
            .into_iter()
            .filter(|(id, name)| self.known_weapons.get(name) != Some(&(id - 5443)))
            .collect::<Vec<_>>();
        let new_resources = !additions.is_empty();
        for (index, name) in additions {
            commands.push(format!("d {index} {name}").as_bytes())?;
        }
        if new_resources {
            commands.push(format!("d 6800 {next}").as_bytes())?;
        }
        self.pending_loadout = Some((commands.sequence(), Arc::new(state)));
        self.server_commands = commands;
        self.known_weapons = known;
        self.next_weapon = next;
        if let Some(guard) = &mut guard {
            **guard = registry;
        }
        tracing::info!("queued saved backpack update for client resource acknowledgement");
        Ok(())
    }
    /// Client resources for a backpack are acknowledged; equip it now, or at
    /// the next training spawn while the current life is still in progress.
    pub(super) fn apply_acknowledged_loadout(&mut self) {
        if self
            .pending_loadout
            .as_ref()
            .is_none_or(|(seq, _)| self.server_commands.acknowledged() < *seq)
        {
            return;
        }
        let (_, state) = self.pending_loadout.take().unwrap();
        if (self.world.bot.is_some() || self.world.tdm.is_some()) && self.world.player.is_some() {
            // A training life keeps its weapons; respawn equips the choice.
            self.next_spawn_loadout = Some(state);
            self.feedback
                .push_back("Backpack will change on next spawn".into());
            tracing::info!("deferred acknowledged player backpack update to respawn");
        } else {
            if let Some(primary) = state.primary_player() {
                if let Some(player) = &mut self.world.player {
                    player.replace_loadout(primary, state.secondary_player());
                } else {
                    self.world.player = state.player();
                }
            }
            self.replace_game_state(state);
            tracing::info!("applied acknowledged player backpack update");
        }
    }
    pub(super) fn replace_game_state(
        &mut self,
        mut state: Arc<crate::game::session::prepared::PreparedGameState>,
    ) {
        if let Some(current) = &self.world.game_state
            && current.collision.is_some()
        {
            Arc::make_mut(&mut state).preserve_match_collision(current);
        }
        self.world.game_state = Some(state);
    }
    pub(super) fn equip_spawn_loadout(&mut self) {
        if let Some(tdm) = &mut self.world.tdm {
            tdm.streaks.spawn(0);
        }
        if let Some(next) = self.next_spawn_loadout.take() {
            self.replace_game_state(next);
            tracing::info!("applied deferred player backpack at respawn");
        }
        if let (Some(tdm), Some(state)) = (&mut self.world.tdm, &self.world.game_state)
            && state.radar_resources().is_some()
        {
            tdm.streaks.configure_radios(0, state.streak_previews());
        }
    }
    pub(crate) fn with_weapon_registry(
        mut self,
        registry: Arc<std::sync::Mutex<crate::game::session::weapon_registry::WeaponRegistry>>,
    ) -> Self {
        self.shared_registry = Some(registry);
        self
    }
}
