use super::*;

impl PreparedGameState {
    pub fn prepare_training_enemy(
        state_path: &Path,
        capture_path: &Path,
        output: &Path,
        distance: f32,
        model_name: &str,
    ) -> Result<()> {
        #[derive(Deserialize)]
        struct Model {
            model: u16,
            name: String,
            bounds_min: [f32; 3],
            bounds_max: [f32; 3],
            bone_names: Vec<String>,
        }
        #[derive(Deserialize)]
        struct Animation {
            index: u16,
            name: String,
            move_speed_bits: u32,
        }
        #[derive(Deserialize)]
        struct Capture {
            schema_version: u32,
            read_only: bool,
            client_sha256: String,
            ncs_crc: u32,
            models: Vec<Model>,
            animations: Vec<Animation>,
        }
        ensure!(
            distance.is_finite() && (64.0..=400.0).contains(&distance),
            "enemy distance must be64..400"
        );
        ensure!(
            std::fs::metadata(capture_path)?.len() <= 4 * 1024 * 1024,
            "character capture too large"
        );
        let raw = std::fs::read(capture_path)?;
        let capture: Capture = serde_json::from_slice(&raw)?;
        let mut state = Self::load(state_path)?;
        ensure!(
            capture.schema_version == 1
                && capture.read_only
                && capture.client_sha256 == codol_protocol::CLIENT_SHA256
                && capture.ncs_crc == state.ncs_crc,
            "character capture identity mismatch"
        );
        let models: Vec<_> = capture
            .models
            .iter()
            .filter(|m| m.name == model_name)
            .collect();
        ensure!(
            models.len() == 1,
            "enemy body must have one registered model identity"
        );
        let model = models[0];
        for bone in [
            "j_mainroot",
            "j_head",
            "j_spinelower",
            "j_wrist_le",
            "j_wrist_ri",
            "tag_weapon_right",
        ] {
            ensure!(
                model.bone_names.iter().any(|b| b == bone),
                "selected enemy model is not a complete human skeleton: missing {bone}"
            );
        }
        let animation = |name: &str| -> Result<&Animation> {
            let found: Vec<_> = capture
                .animations
                .iter()
                .filter(|a| a.name == name)
                .collect();
            ensure!(
                found.len() == 1 && (1..1024).contains(&found[0].index),
                "human animation {name} missing or ambiguous"
            );
            Ok(found[0])
        };
        let collision = state
            .collision
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("enemy requires map collision"))?;
        ensure!(
            collision.captured_contents_mask & crate::game::world::collision::SHOT_MASK
                == crate::game::world::collision::SHOT_MASK,
            "enemy requires full shot collision capture"
        );
        let spawn = state
            .spawn_origin
            .ok_or_else(|| anyhow::anyhow!("enemy requires player spawn"))?;
        let mut origin = [spawn[0] + distance, spawn[1], 0.0];
        let start = [origin[0], origin[1], spawn[2] + 512.0];
        let end = [origin[0], origin[1], spawn[2] - 2048.0];
        let floor = collision.raycast(start, end);
        ensure!(
            floor.contents != 0 && !floor.start_solid && floor.normal[2] >= 0.7,
            "no walkable ground under enemy candidate"
        );
        origin[2] = (start[2] + (end[2] - start[2]) * floor.fraction - model.bounds_min[2]).ceil();
        origin = crate::game::ai::controller::grounded_spawn(origin, collision)?;
        let eye = [spawn[0], spawn[1], spawn[2] + 60.0];
        let center =
            std::array::from_fn(|i| origin[i] + (model.bounds_min[i] + model.bounds_max[i]) * 0.5);
        let sight = collision.raycast(eye, center);
        ensure!(
            !sight.start_solid && (sight.contents == 0 || sight.fraction >= 1.0),
            "enemy candidate is behind world geometry"
        );
        let walk = animation("pb_stand_shoot_walk_forward")?;
        let weapon = state
            .bootstrap_weapon
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("enemy requires registered weapon resources"))?;
        let config = crate::game::ai::enemy::EnemyConfig {
            model: model.model,
            origin,
            bounds_min: model.bounds_min,
            bounds_max: model.bounds_max,
            idle_animation: animation("pb_stand_alert")?.index,
            walk_animation: walk.index,
            walk_left_animation: Some(animation("pb_stand_shoot_walk_left")?.index),
            walk_right_animation: Some(animation("pb_stand_shoot_walk_right")?.index),
            walk_back_animation: Some(animation("pb_stand_shoot_walk_back")?.index),
            locomotion: None,
            fire_animation: animation("pt_stand_shoot_auto")?.index,
            fire_animation_msec: None,
            reload_animation: None,
            death_animation: animation("pb_stand_death_chest_blowback")?.index,
            move_speed: f32::from_bits(walk.move_speed_bits),
            angles: [0, 32768, 0],
            weapon: weapon.base_index.unwrap_or(weapon.index),
            source_sha256: format!("{:x}", Sha256::digest(&raw)),
            bot: false,
        };
        config.validate()?;
        state.training_target = None;
        state.training_enemy = Some(config.clone());
        state.validate()?;
        let mut json: serde_json::Value = serde_json::from_slice(&std::fs::read(state_path)?)?;
        json.as_object_mut().unwrap().remove("training_target");
        json["training_enemy"] = serde_json::json!({"model":config.model,"origin":config.origin,"bounds_min":config.bounds_min,"bounds_max":config.bounds_max,"idle_animation":config.idle_animation,"walk_animation":config.walk_animation,"walk_left_animation":config.walk_left_animation,"walk_right_animation":config.walk_right_animation,"walk_back_animation":config.walk_back_animation,"fire_animation":config.fire_animation,"death_animation":config.death_animation,"move_speed":config.move_speed,"angles":config.angles,"weapon":config.weapon,"source_sha256":config.source_sha256});
        for name in ["arsenal_file", "collision_file"] {
            if let Some(path) = json[name].as_str() {
                json[name] = state_path
                    .parent()
                    .unwrap_or(Path::new("."))
                    .join(path)
                    .canonicalize()?
                    .to_string_lossy()
                    .into_owned()
                    .into();
            }
        }
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output)?;
        file.write_all(&serde_json::to_vec_pretty(&json)?)?;
        file.write_all(b"\n")?;
        Ok(())
    }
    pub fn prepare_training_target(
        state_path: &Path,
        model_path: &Path,
        output: &Path,
        distance: f32,
        scale: f32,
    ) -> Result<()> {
        #[derive(Deserialize)]
        struct ModelCapture {
            schema_version: u32,
            read_only: bool,
            client_sha256: String,
            ncs_crc: u32,
            model_name: String,
            model: u16,
            bounds_min: [f32; 3],
            bounds_max: [f32; 3],
        }
        ensure!(
            distance.is_finite() && (64.0..=400.0).contains(&distance),
            "training target distance must be 64..400 world units"
        );
        ensure!(
            scale.is_finite() && (0.25..=4.0).contains(&scale),
            "target scale outside supported range"
        );
        ensure!(
            std::fs::metadata(model_path)?.len() <= 65536,
            "target model capture too large"
        );
        let raw = std::fs::read(model_path)?;
        let model: ModelCapture = serde_json::from_slice(&raw)?;
        let mut state = Self::load(state_path)?;
        ensure!(
            model.schema_version == 1
                && model.read_only
                && model.client_sha256 == codol_protocol::CLIENT_SHA256
                && model.ncs_crc == state.ncs_crc
                && model.model_name == "com_barrel_russian_fuel_dirt",
            "training model identity mismatch"
        );
        let collision = state
            .collision
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("target needs map collision"))?;
        ensure!(
            collision.captured_contents_mask & crate::game::world::collision::SHOT_MASK
                == crate::game::world::collision::SHOT_MASK,
            "capture both movement and shooting contents before target preparation"
        );
        let spawn = state
            .spawn_origin
            .ok_or_else(|| anyhow::anyhow!("target needs player spawn"))?;
        let floor_at = |x: f32, y: f32| -> Result<f32> {
            let start = [x, y, spawn[2] + 512.0];
            let end = [x, y, spawn[2] - 2048.0];
            let hit = collision.raycast(start, end);
            ensure!(
                hit.contents != 0 && hit.fraction < 1.0 && !hit.start_solid && hit.normal[2] >= 0.7,
                "no walkable ground under target candidate"
            );
            Ok(start[2] + (end[2] - start[2]) * hit.fraction)
        };
        let mut config = crate::game::combat::TargetConfig {
            entity_number: 19,
            model: model.model,
            origin: [spawn[0] + distance, spawn[1], 0.0],
            scale,
            bounds_min: model.bounds_min,
            bounds_max: model.bounds_max,
            health: 100,
            respawn_msec: 3000,
            source_sha256: format!("{:x}", Sha256::digest(&raw)),
        };
        config.origin[2] =
            (floor_at(config.origin[0], config.origin[1])? - config.bounds_min[2] * scale).ceil();
        // Entity coordinates use integral world positions; use precisely those
        // positions for the server's hit volume as well as its visible model.
        config.origin = config.origin.map(f32::round_ties_even);
        config.validate()?;
        let eye = [spawn[0], spawn[1], floor_at(spawn[0], spawn[1])? + 60.0];
        let center = std::array::from_fn(|i| {
            config.origin[i] + (config.bounds_min[i] + config.bounds_max[i]) * 0.5 * scale
        });
        let sight = collision.raycast(eye, center);
        ensure!(
            sight.contents == 0 || sight.fraction >= 1.0,
            "training target candidate is behind map geometry"
        );
        state.training_target = Some(config.clone());
        state.validate()?;
        let mut json: serde_json::Value = serde_json::from_slice(&std::fs::read(state_path)?)?;
        json["training_target"] = serde_json::json!({"entity_number":config.entity_number,"model":config.model,"origin":config.origin,"scale":config.scale,"bounds_min":config.bounds_min,"bounds_max":config.bounds_max,"health":config.health,"respawn_msec":config.respawn_msec,"source_sha256":config.source_sha256});
        for name in ["arsenal_file", "collision_file"] {
            if let Some(file) = json[name].as_str() {
                json[name] = serde_json::Value::String(
                    state_path
                        .parent()
                        .unwrap_or(Path::new("."))
                        .join(file)
                        .canonicalize()?
                        .to_string_lossy()
                        .into(),
                );
            }
        }
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output)?;
        file.write_all(&serde_json::to_vec_pretty(&json)?)?;
        file.write_all(b"\n")?;
        Ok(())
    }
}
