use super::*;

impl PreparedGameState {
    pub(crate) fn progression_item(&self, index: u16) -> Option<u32> {
        self.bootstrap_weapon
            .iter()
            .chain(&self.secondary_weapon)
            .find_map(|w| {
                if w.index == index {
                    w.marketplace_item_id
                } else {
                    w.alternate
                        .as_ref()
                        .filter(|a| a.index == index)
                        .and_then(|a| a.marketplace_item_id)
                }
            })
    }
    pub(crate) fn for_profile(&self, public: &[u8], owned: &[u32]) -> Result<Self> {
        let mut state = self.for_profile_at(
            public,
            owned,
            self.bootstrap_weapon
                .as_ref()
                .map_or(self.next_weapon_index(), |w| w.index),
        )?;
        let mut next = self.bootstrap_weapon.as_ref().map_or(1, |w| w.index);
        let mut names = std::collections::BTreeMap::new();
        state.register_weapons(&mut names, &mut next)?;
        state.registry_strings = names
            .into_iter()
            .map(|(name, index)| (5443 + index, name))
            .collect();
        state.registry_strings.sort_by_key(|v| v.0);
        Ok(state)
    }
    pub(crate) fn for_backpack(&self, public: &[u8], slot: u8) -> Result<Self> {
        let mut state = self.clone();
        state.player_loadout = Some(
            self.player_loadout
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("missing backpack schema"))?
                .selected_for(public, slot)?,
        );
        Ok(state)
    }
    pub(crate) fn selected_backpack_class(&self) -> Option<u8> {
        self.player_loadout
            .as_ref()
            .map(|loadout| loadout.class_index())
    }
    pub(crate) fn for_initial_backpack(&self, public: &[u8]) -> Result<Self> {
        let slot = self
            .player_loadout
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing backpack schema"))?
            .class_index()
            % crate::game::player::class_selection::BACKPACKS_PER_CHARACTER;
        self.for_backpack(public, slot)
    }
    pub(crate) fn update_profile_selection(&mut self, selected: &Self) {
        self.player_loadout = selected.player_loadout.clone();
        self.profile_hash = selected.profile_hash;
        self.player_card = selected.player_card;
    }
    pub(crate) fn preserve_match_appearance(&mut self, current: &Self) {
        // Character changes belong to the next match. Existing actor/corpse
        // baselines and historical replays retain the current model identities.
        self.player_appearance = current.player_appearance.clone();
    }
    pub(crate) fn set_card_rank(&mut self, (rank, prestige): (u8, u8)) {
        if let Some(card) = &mut self.player_card {
            card.rank = rank;
            card.prestige = prestige;
        }
    }
    pub(crate) fn first_weapon_index(&self) -> u16 {
        self.bootstrap_weapon.as_ref().map_or(1, |w| w.index)
    }
    pub(crate) fn for_profile_at(
        &self,
        public: &[u8],
        owned: &[u32],
        next_index: u16,
    ) -> Result<Self> {
        let loadout = self
            .player_loadout
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing loadout catalogue"))?;
        let mut state = self.clone();
        state.player_card = Some(loadout.card(public)?);
        state.player_grenades = crate::game::weapons::grenade::resolve(
            &loadout.profile_snapshot(public)?,
            loadout.class_index(),
            owned,
        )?;
        state.player_perks = crate::game::status::perks::Perks::from_profile(
            &loadout.profile_snapshot(public)?,
            loadout.class_index(),
            owned,
        )?;
        state.player_streak_previews = crate::game::rewards::killstreaks::Previews::from_profile(
            &loadout.profile_snapshot(public)?,
            loadout.class_index(),
            owned,
            state.player_perks.value("modKillstreakShift", 0.0),
        )?;
        tracing::info!(slots=?state.player_perks.slots, masks=?state.player_perks.abilities.masks,
            "resolved saved passive skills");
        if let Some(catalog) = &self.appearance_catalog
            && self.player_appearance.is_none()
        {
            let snapshot = loadout.profile_snapshot(public)?;
            let mut appearance = catalog.compose(
                &snapshot,
                owned,
                self.match_config.as_ref().map_or(2, |c| c.teams[0]),
            )?;
            appearance.register(
                self.match_config
                    .as_ref()
                    .map_or(&[], |c| c.models.as_slice()),
                self.appearance_first_model.unwrap(),
            )?;
            tracing::info!(package=appearance.package,models=?appearance.models,"resolved saved third-person character");
            state.player_appearance = Some(appearance);
        }
        if let Some(arsenal) = &self.arsenal {
            let selections = loadout.selections(public)?;
            let hands = self
                .bootstrap_weapon
                .as_ref()
                .and_then(|w| w.viewmodel_index);
            state.bootstrap_weapon = Some(Self::resolved_with_alternate(
                arsenal,
                arsenal.resolve(selections[0].0, &selections[0].1, owned, next_index)?,
                hands,
            )?);
            if let Some(weapon) = &mut state.bootstrap_weapon {
                weapon.marketplace_item_id = Some(selections[0].0);
                if let Some(alternate) = &mut weapon.alternate {
                    alternate.marketplace_item_id = weapon.marketplace_item_id;
                }
            }
            state.secondary_weapon = if selections[1].0 == 0 {
                None
            } else {
                Some(Self::resolved_with_alternate(
                    arsenal,
                    arsenal.resolve(selections[1].0, &selections[1].1, owned, next_index + 2)?,
                    hands,
                )?)
            };
            if let Some(weapon) = &mut state.secondary_weapon {
                weapon.marketplace_item_id = Some(selections[1].0);
                if let Some(alternate) = &mut weapon.alternate {
                    alternate.marketplace_item_id = weapon.marketplace_item_id;
                }
            }
            tracing::info!(primary=?state.bootstrap_weapon.as_ref().and_then(|w|w.composed_name.as_deref()),secondary=?state.secondary_weapon.as_ref().and_then(|w|w.composed_name.as_deref()),"composed current player backpack from resource attributes");
            if let Some(config) = &self.match_config {
                state.match_weapon = Some(Self::resolved_weapon(
                    arsenal.bot_rifle(next_index + 4)?,
                    &arsenal.source_sha256,
                    hands,
                )?);
                state.roster_weapons = config
                    .roster
                    .iter()
                    .enumerate()
                    .map(|(i, bot)| {
                        Self::resolved_weapon(
                            arsenal.bot_weapon(&bot.weapon_name, next_index + 5 + i as u16)?,
                            &arsenal.source_sha256,
                            hands,
                        )
                    })
                    .collect::<Result<_>>()?;
            }
        } else {
            state.bootstrap_weapon = Some(loadout.resolve(self, public, owned)?);
            state.player_loadout = None;
        }
        state.profile_hash = Some(Sha256::digest(public).into());
        state.validate()?;
        Ok(state)
    }
    pub(super) fn resolved_with_alternate(
        arsenal: &crate::content::weapons::arsenal::Arsenal,
        resolved: crate::content::weapons::arsenal::ResolvedWeapon,
        hands: Option<u16>,
    ) -> Result<BootstrapWeapon> {
        let alternate = arsenal.resolve_alternate(&resolved)?;
        let mut weapon = Self::resolved_weapon(resolved, &arsenal.source_sha256, hands)?;
        if let Some(resolved) = alternate {
            // Target 0x42c270: +0xdad aliases the parent's magazine key.
            ensure!(
                resolved.definition[0xdad] != 0,
                "alternate with independent clip is not yet supported"
            );
            let mut alternate = Self::resolved_weapon(resolved, &arsenal.source_sha256, hands)?;
            let parent_ammo = weapon.ammo.as_ref().unwrap();
            let ammo = alternate.ammo.as_mut().unwrap();
            ammo.clip_key = parent_ammo.clip_key;
            ammo.clip = parent_ammo.clip;
            ammo.reserve_key = parent_ammo.reserve_key;
            ammo.reserve = parent_ammo.reserve;
            weapon.alternate = Some(Box::new(alternate));
        }
        Ok(weapon)
    }
    pub(super) fn resolved_weapon(
        w: crate::content::weapons::arsenal::ResolvedWeapon,
        source: &str,
        hands: Option<u16>,
    ) -> Result<BootstrapWeapon> {
        let word = |at: usize| u32::from_le_bytes(w.definition[at..at + 4].try_into().unwrap());
        let time = |at: usize| -> Result<u16> {
            let n = word(at);
            ensure!(n <= 15000, "weapon duration exceeds supported range");
            Ok(n as u16)
        };
        let normal = time(0x71c)?;
        let empty = time(0x728)?;
        let add = time(0x730)?;
        let empty_add = time(0x738)?;
        ensure!(
            word(0xd08) > 0 && word(0xd08) <= 511 && word(0x6b8) <= 1023,
            "weapon ammo limits"
        );
        ensure!(
            word(0x6f0) == 0,
            "delayed-fire weapon needs its firing state machine"
        );
        let ammo = BootstrapAmmo {
            limits: Some(crate::game::weapons::held::AmmoLimits {
                contribution: word(0x6c4).saturating_mul(if w.definition[0xdac] != 0 {
                    word(0xd08)
                } else {
                    1
                }),
                shared: if word(0x6d0) as i32 >= 0 {
                    Some([word(0x6d0), word(0x6d4)])
                } else {
                    None
                },
                clip_only: w.definition[0xd7b] != 0,
                scavenger_allowed: w.definition[0x8c8] == 0
                    && w.definition[0xd7c] == 0
                    && word(0x480) != 12
                    && word(0x47c) != 3,
            }),
            clip_key: w.index,
            reserve_key: word(0x6b8) as u16,
            clip: word(0xd08) as u16,
            reserve: (word(0xd08) * 3).min(2047) as u16,
            source_sha256: source.into(),
            fire_interval_msec: Some(time(0xd10)?),
            reload: Some(crate::game::weapons::held::ReloadTiming {
                normal_msec: normal,
                empty_msec: empty,
                add_msec: if add == 0 { normal } else { add.min(normal) },
                empty_add_msec: if empty_add == 0 {
                    empty
                } else {
                    empty_add.min(empty)
                },
                segmented: (w.definition[0xd86] != 0).then_some(
                    crate::game::weapons::held::SegmentedReload {
                        start_msec: time(0x740)?,
                        start_add_msec: match (time(0x740)?, time(0x744)?) {
                            (start, add) if add != 0 && add < start => add,
                            _ => 0,
                        },
                        end_msec: time(0x748)?,
                    },
                ),
            }),
            ads: if w.definition[0xd76] == 0 {
                None
            } else {
                Some(crate::game::weapons::held::AdsTiming {
                    in_rate: f32::from_bits(word(0xbac)),
                    out_rate: f32::from_bits(word(0xbb0)),
                    reload_allowed: w.definition[0xd78] != 0,
                    reload_interruptible: w.definition[0xd86] != 0,
                    reload_end_msec: time(0xb38)?,
                })
            },
        };
        let drop = time(0x74c)?;
        let raise = time(0x750)?;
        let switch = crate::game::weapons::held::SwitchTiming {
            drop_msec: drop,
            raise_msec: raise,
            empty_drop_msec: match time(0x768)? {
                0 => drop,
                v => v,
            },
            empty_raise_msec: match time(0x764)? {
                0 => raise,
                v => v,
            },
        };
        let scale = f32::from_bits(word(0x7c8));
        let ads_scale = f32::from_bits(word(0x7cc));
        let fire_type = word(0x48c);
        // 0x42c270 / 0x42dff0 register a composed alternate immediately after
        // its parent. This inherited index identifies the base alt definition.
        let alternate_base = match word(0xd24) {
            0 => None,
            n if n < 900 => Some(n as u16),
            _ => anyhow::bail!("alternate weapon base outside registry"),
        };
        // Type names at0xce3f78: bullet0/grenade1/projectile2/shield3/bomb4/melee5.
        // Crossbows and ballistic knives use rifle/pistol classes, so class
        // alone cannot select hitscan. The native gun consumer0x55e45f also
        // gates its bullet path on WeaponDef+0x47c==0.
        let ballistics = if word(0x47c) != 0
            || matches!(word(0x480), 6 | 7 | 11 | 12)
            || w.definition[0xd79] != 0
        {
            None
        } else {
            let damage = word(0xbb4);
            let near_range = f32::from_bits(word(0xbcc));
            ensure!(
                damage <= u32::from(u16::MAX),
                "weapon damage outside supported limits"
            );
            let multipliers: [f32; 20] =
                std::array::from_fn(|i| f32::from_bits(word(0xbf0 + i * 4)));
            let value = crate::game::combat::Ballistics {
                damage: damage as u16,
                range: if word(0x480) == 4 {
                    f32::from_bits(word(0xbe0))
                } else {
                    8192.0
                },
                near_range,
                // The block is all zero for knives/launchers; keep base damage.
                location_multipliers: multipliers.iter().any(|m| *m > 0.0).then_some(multipliers),
                pellets: word(0x6c8).clamp(1, 32) as u8,
                // Standing hip spread min/max (+0x810/+0x81c), in degrees.
                spread_degrees: if word(0x6c8) > 1 {
                    f32::from_bits(word(0x810))
                } else {
                    0.0
                },
                falloff: crate::game::combat::DamageFalloff::from_stages(
                    std::array::from_fn(|i| word(0xbb4 + i * 4)),
                    std::array::from_fn(|i| f32::from_bits(word(0xbcc + i * 4))),
                ),
            };
            if value.valid() { Some(value) } else { None }
        };
        let alternate_timing = Some(crate::game::weapons::held::AlternateTiming {
            drop_msec: time(0x754)?,
            raise_msec: time(0xd28)?,
        });
        let rechamber = if w.definition[0xd74] != 0 {
            Some(crate::game::weapons::held::RechamberTiming {
                duration_msec: time(0x700)?,
                eject_msec: time(0x708)?,
            })
        } else {
            None
        };
        let melee = if word(0x47c) == 0 && word(0x6e0) > 0 {
            let timing = crate::game::weapons::held::MeleeTiming {
                duration_msec: time(0x714)?,
                hit_msec: time(0x6f4)?,
                recover_msec: time(0x75c)?,
                damage: u16::try_from(word(0x6e0))?,
                charge: if w.melee_charge_animation && word(0x718) > 0 && word(0x6f8) > 0 {
                    Some(crate::game::weapons::held::MeleeChargeTiming {
                        duration_msec: time(0x718)?,
                        hit_msec: time(0x6f8)?,
                    })
                } else {
                    None
                },
            };
            ensure!(timing.valid(), "invalid held-gun melee parameters");
            Some(timing)
        } else {
            None
        };
        let spread = crate::game::weapons::held::SpreadTiming {
            minimum: std::array::from_fn(|i| f32::from_bits(word(0x810 + i * 4))),
            maximum: std::array::from_fn(|i| f32::from_bits(word(0x81c + i * 4))),
            decay: f32::from_bits(word(0x828)),
            fire_add: f32::from_bits(word(0x82c)),
            turn_add: f32::from_bits(word(0x830)),
            move_add: f32::from_bits(word(0x834)),
            stance_decay: [
                1.0,
                f32::from_bits(word(0x838)),
                f32::from_bits(word(0x83c)),
            ],
            ads: f32::from_bits(word(0xac0)),
        };
        ensure!(spread.valid(), "invalid weapon spread parameters");
        let sprint_duration_scale = f32::from_bits(word(0x7d0));
        ensure!(
            sprint_duration_scale.is_finite() && (0.0..=16.0).contains(&sprint_duration_scale),
            "invalid native weapon sprint duration scale"
        );
        tracing::info!(weapon = %w.name, ads_in_msec = word(0xad0), ads_out_msec = word(0xad4),
            hip_move_scale = scale, ads_move_scale = ads_scale,
            fire_msec = word(0xd10), rechamber_msec = rechamber.map_or(0, |t| t.duration_msec),
            hip_min = ?spread.minimum, hip_max = ?spread.maximum, "resolved weapon gameplay attributes");
        Ok(BootstrapWeapon {
            marketplace_item_id: None,
            offhand_switch: Some(crate::game::weapons::held::AlternateTiming {
                drop_msec: time(0x758)?,
                raise_msec: time(0x75c)?,
            }),
            fast_reload: w.definition[0xd70] != 0,
            name: w.name,
            index: w.index,
            ammo: Some(ammo),
            base_index: Some(w.base_index),
            composed_name: Some(w.composed_name),
            viewmodel_index: hands,
            attachment_source_sha256: if w.attachments.is_empty() {
                None
            } else {
                Some(source.into())
            },
            attachments: w.attachments,
            camo: Some(0),
            switch: Some(switch),
            move_scale: Some(scale),
            ads_move_scale: Some(ads_scale),
            sprint_duration_scale: Some(sprint_duration_scale),
            sprint_timing: Some(crate::game::weapons::held::SprintTiming {
                in_msec: time(0x76c)?,
                out_msec: time(0x774)?,
            }),
            fire_type: Some(fire_type),
            alternate_base,
            alternate: None,
            alternate_timing,
            rechamber,
            spread: Some(spread),
            melee,
            ballistics,
        })
    }
}
