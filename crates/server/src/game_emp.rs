//! Native otherFlags EMP bit with explicitly sourced reference player timing.
use serde::Deserialize;
use std::sync::LazyLock;

#[derive(Deserialize)]
struct Catalog {
    schema_version: u32,
    client_sha256: String,
    native: Native,
    server_rules: Rules,
}
#[derive(Deserialize)]
struct Native {
    hud_operator: u16,
    getter_va: u32,
    getter_sha256: String,
    player_state_offset: u32,
    field: u8,
    bits: u8,
    mask: u32,
}
#[derive(Deserialize)]
struct Rules {
    source: String,
    duration_msec: u32,
    flash_msec: u16,
    immunity_ability: String,
}
static CATALOG: LazyLock<Catalog> = LazyLock::new(|| {
    let c: Catalog = serde_json::from_str(include_str!(
        "../../../fixtures/protocol/emp-status-3.3.3.3.json"
    ))
    .expect("verified current EMP consumer and reference rules");
    assert_eq!(c.schema_version, 1);
    assert_eq!(c.client_sha256, codol_protocol::CLIENT_SHA256);
    assert_eq!(c.native.hud_operator, 154);
    assert_eq!(c.native.getter_va, 0x574130);
    assert_eq!(c.native.getter_sha256.len(), 64);
    assert_eq!(
        (
            c.native.player_state_offset,
            c.native.field,
            c.native.bits,
            c.native.mask
        ),
        (0x10, 54, 28, 0x400)
    );
    assert!(c.server_rules.source.contains("reference player EMP port"));
    assert_eq!(c.server_rules.duration_msec, 10000);
    assert_eq!(c.server_rules.flash_msec, 1000);
    assert_eq!(c.server_rules.immunity_ability, "modEmpProtectionScale");
    c
});
#[derive(Clone, Copy)]
pub(crate) struct Target {
    pub origin: [f32; 3],
    pub height: f32,
    pub protection: f32,
}
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct State {
    interval: Option<(u32, u32)>,
}
impl State {
    pub fn clear(&mut self) {
        self.interval = None;
    }
    pub fn active(&self, now: u32) -> bool {
        self.interval
            .is_some_and(|(begin, end)| now >= begin && now < end)
    }
    pub fn apply(
        &mut self,
        now: u32,
        explosion: &crate::game_grenade::Explosion,
        target: Target,
        collision: &crate::game_motion::Collision,
        shock: &mut crate::game_tactical::State,
    ) -> bool {
        let Target {
            origin,
            height,
            protection,
        } = target;
        if !explosion.definition.emp
            || !protection.is_finite()
            || protection <= 0.0
            || crate::game_grenade::radius_damage(explosion, origin, height, collision) == 0
        {
            return false;
        }
        let duration = (CATALOG.server_rules.duration_msec as f32 * protection)
            .round()
            .clamp(0.0, 65535.0) as u32;
        if duration == 0 {
            return false;
        }
        self.interval = Some((now, now.saturating_add(duration)));
        shock.apply(
            now,
            crate::game_tactical::Hit {
                kind: crate::game_tactical::Kind::Flash,
                duration_msec: CATALOG.server_rules.flash_msec,
            },
        );
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn emp_refreshes_its_deadline_without_inventing_an_emp_shock_or_treating_default_zero_as_immunity()
     {
        let export = serde_json::json!({"schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,
            "map":"maps/mp/test.d3dbsp","ncs_crc":0,"brushes":[{"center":[0,0,-10],"half":[2000,2000,10],"contents":1,"planes":[]}],"triangles":[]});
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), serde_json::to_vec(&export).unwrap()).unwrap();
        let collision = crate::game_motion::Collision::load(file.path()).unwrap();
        let explosion = crate::game_grenade::Explosion {
            time: 2000,
            origin: [0.0, 0.0, 60.0],
            normal: [0.0, 0.0, 1.0],
            definition: crate::game_grenade::captured("emp_grenade_mp"),
        };
        let mut state = State::default();
        let mut shock = crate::game_tactical::State::default();
        let mut player = crate::game_player::tests::scavenger_player(90, 180);
        assert_eq!(player.emp_protection(), 1.0);
        assert!(state.apply(
            2000,
            &explosion,
            Target {
                origin: [0.0; 3],
                height: 70.0,
                protection: player.emp_protection()
            },
            &collision,
            &mut shock
        ));
        assert!(state.active(11999));
        assert!(!state.active(12000));
        assert_eq!(shock.snapshot(2000).unwrap().index, 5);
        assert_eq!(shock.snapshot(3000).unwrap().duration_msec, 0);
        assert!(state.apply(
            9000,
            &explosion,
            Target {
                origin: [0.0; 3],
                height: 70.0,
                protection: 1.0
            },
            &collision,
            &mut shock
        ));
        assert!(state.active(18999));
        assert!(!state.active(19000));
        state.clear();
        assert!(!state.active(9100));
        player.set_perks(crate::game_perks::Perks::selected([0, 60213, 0], &[60213]).unwrap());
        assert_eq!(player.emp_protection(), 0.0);
        shock.clear();
        assert!(!state.apply(
            2000,
            &explosion,
            Target {
                origin: [0.0; 3],
                height: 70.0,
                protection: player.emp_protection()
            },
            &collision,
            &mut shock
        ));
        assert!(!state.active(2000));
        assert_eq!(shock.snapshot(2000).unwrap().duration_msec, 0);
        assert!(!state.apply(
            2000,
            &explosion,
            Target {
                origin: [600.0, 0.0, 0.0],
                height: 70.0,
                protection: 1.0
            },
            &collision,
            &mut shock
        ));
    }
}
