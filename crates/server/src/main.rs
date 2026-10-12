use anyhow::{Context, Result, bail, ensure};
use clap::{Parser, Subcommand};
use codol_protocol::bootstrap::{DwauthConfig, Endpoint};
use codol_server::auth_server::{AuthOptions, AuthServer};
use codol_server::lobby_server::LobbyServer;
use codol_server::stun_server::StunServer;
use codol_server::{config::Config, observe::Observer};
use std::{
    io::{Read, Write},
    path::PathBuf,
};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    version,
    about = "CODOL research runtime: local AUTH, lobby 210, and bounded service recovery"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
// CLI arguments are parsed once at startup, outside gameplay frame processing.
#[allow(clippy::large_enum_variant)]
enum Command {
    /// Select captured TDM starts that fit the gameplay capsule, preserving rejected evidence.
    PrepareMapStarts {
        #[arg(long)]
        game_state: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        report: PathBuf,
    },
    /// Quickly validate each prepared map's server movement, combat and respawns.
    VerifyMaps {
        #[arg(long)]
        catalog: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, default_value_t = 60)]
        seconds: u32,
    },
    /// Prepare a real human training opponent from loaded models and animation evidence.
    PrepareTrainingEnemy {
        #[arg(long)]
        game_state: PathBuf,
        #[arg(long)]
        character_capture: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, default_value_t = 144.0)]
        distance: f32,
        #[arg(long, default_value = "mp_fullbody_juggernaut_explosive_so")]
        body_model: String,
    },
    /// Prepare a ground-placed visible training target using actual model bounds.
    PrepareTrainingTarget {
        #[arg(long)]
        game_state: PathBuf,
        #[arg(long)]
        model_capture: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, default_value_t = 160.0)]
        distance: f32,
        #[arg(long, default_value_t = 2.0)]
        scale: f32,
    },
    /// Offline only: back up an existing test account, set level 70 and local funds.
    BoostTestAccount {
        #[arg(long)]
        service_data_dir: PathBuf,
        #[arg(long, default_value_t = 1002)]
        title_id: u32,
        #[arg(long)]
        user_id: u64,
        #[arg(long, default_value_t = 100_000)]
        cp: u32,
        #[arg(long, default_value_t = 1_000_000)]
        gp: u32,
    },
    /// Create a local private profile from a CRC-valid client default; no overwrite.
    PreparePrivateProfile {
        #[arg(long)]
        assets: PathBuf,
        #[arg(long)]
        snapshot: PathBuf,
        #[arg(long)]
        service_data_dir: PathBuf,
        #[arg(long, default_value_t = 1002)]
        title_id: u32,
        /// Mark only the target's CQB training flag for lobby research.
        #[arg(long)]
        training_complete: bool,
    },
    /// Validate a captured decrypted 8/3 public-profile upload without importing it.
    InspectPublicProfile {
        #[arg(long)]
        input: PathBuf,
    },
    /// Generate the two minimal local publisher files; refuses existing files.
    PreparePublisher {
        #[arg(long)]
        output_dir: PathBuf,
    },
    /// Serve the observed legacy AUTH transport on loopback; default reply is 501.
    ServeAuth {
        #[arg(long, default_value = "127.0.0.1:39002")]
        bind: std::net::SocketAddr,
        #[arg(long)]
        cert: PathBuf,
        #[arg(long)]
        key: PathBuf,
        #[arg(long, default_value = "captures")]
        capture_dir: PathBuf,
        /// Explicit experimental JSON response (HTTP 200); not a login implementation.
        #[arg(long)]
        reply_file: Option<PathBuf>,
        /// Issue local test-account tickets for this title (not QQ login).
        #[arg(long, conflicts_with = "reply_file")]
        local_title_id: Option<u32>,
        /// Optional protocol-210 lobby handshake/observation endpoint.
        #[arg(long, requires = "local_title_id")]
        lobby_bind: Option<std::net::SocketAddr>,
        /// Enable local time/storage services and explicit failures for unknown tasks.
        #[arg(long, requires = "lobby_bind")]
        service_data_dir: Option<PathBuf>,
        /// Optional Demonware UDP discovery pair, on loopback only.
        #[arg(long, requires = "lobby_bind")]
        stun_bind: Option<std::net::SocketAddrV4>,
        #[arg(long, default_value = "127.0.0.1:3078")]
        stun_alt: std::net::SocketAddrV4,
        /// Experimental game UDP handshake endpoint; no world simulation yet.
        #[arg(long, requires_all = ["service_data_dir", "lobby_bind"])]
        game_bind: Option<std::net::SocketAddrV4>,
        /// Resource-derived gamestate for the controlled training playlist.
        #[arg(long, requires = "game_bind")]
        game_state: Option<PathBuf>,
        /// Prepared maps selected by the native room's map and mode fields.
        #[arg(long, requires = "game_bind")]
        game_map_catalog: Option<PathBuf>,
        /// Include the eleven server simulated TDM players in native custom rooms.
        #[arg(long, requires = "game_map_catalog")]
        native_bot_rooms: bool,
        /// Collect one native TDM map's loaded resources without sending a world.
        #[arg(long, requires = "native_bot_rooms")]
        resource_capture_map: Option<String>,
        /// Request a native compass candidate during metadata-only resource initialization.
        #[arg(long, requires = "resource_capture_map")]
        resource_compass_probe: Option<PathBuf>,
        /// Keep the authenticated local play session running until shutdown.
        #[arg(long, requires_all=["game_state","service_data_dir"], conflicts_with="snapshot_probe")]
        continuous_play: bool,
        /// Sample ordinary gameplay captures; errors and handshake stay captured.
        #[arg(long,default_value_t=1,value_parser=clap::value_parser!(u32).range(1..=1000))]
        game_capture_every: u32,
        /// Bootstrap frames at 20 Hz (default 60, maximum 18000); no simulation.
        #[arg(long, requires = "game_state", num_args = 0..=1, default_missing_value = "60", value_parser = clap::value_parser!(u32).range(1..=18000))]
        snapshot_probe: Option<u32>,
        /// Enemy body-pose capture; may be written after the server starts.
        #[arg(long, requires = "game_state")]
        hit_regions: Option<PathBuf>,
    },
    /// Parse an observed task-10 JSON body; identifiers are omitted from output.
    InspectAuth {
        #[arg(long)]
        input: PathBuf,
    },
    /// Inspect one reassembled TLS ClientHello (does not negotiate TLS).
    InspectTls {
        #[arg(long)]
        input: PathBuf,
    },
    /// Record incoming TCP streams without replying or guessing game packets.
    Observe {
        #[arg(long, default_value = "configs/local.toml")]
        config: PathBuf,
    },
    /// Validate listener configuration without opening sockets.
    CheckConfig {
        #[arg(long, default_value = "configs/local.toml")]
        config: PathBuf,
    },
    /// Produce a NEW laboratory dwauth.ini; endpoint order is static evidence.
    Bootstrap {
        #[arg(long, default_value = "configs/local.toml")]
        config: PathBuf,
        /// Address reachable from the client (VM loopback is not Mac loopback).
        #[arg(long)]
        host: String,
        /// Hex title_id. Field mapping is confirmed; valid game value is not.
        #[arg(long, alias = "context-hex")]
        title_id_hex: String,
        /// Must not exist. Use a research directory, then a disposable client copy.
        #[arg(long)]
        output: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();
    match Cli::parse().command {
        Command::PrepareMapStarts {
            game_state,
            output,
            report,
        } => {
            codol_server::game_map_starts::prepare(&game_state, &output, &report)?;
        }
        Command::VerifyMaps {
            catalog,
            output,
            seconds,
        } => {
            let started = std::time::Instant::now();
            let catalog = codol_server::game_map_catalog::GameMapCatalog::load(&catalog)?;
            let catalog_load_msec = started.elapsed().as_millis();
            let mut report = codol_server::game_map_verify::verify(
                &catalog,
                seconds
                    .checked_mul(1000)
                    .ok_or_else(|| anyhow::anyhow!("simulation duration overflow"))?,
            )?;
            report.catalog_load_msec = catalog_load_msec;
            report.elapsed_msec = started.elapsed().as_millis();
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&output)?;
            file.write_all(&serde_json::to_vec_pretty(&report)?)?;
            file.write_all(b"\n")?;
            println!(
                "{} maps, {}ms, passed={}",
                report.maps.len(),
                report.elapsed_msec,
                report.passed
            );
            ensure!(
                report.passed,
                "one or more map checks failed; see {}",
                output.display()
            );
        }
        Command::PrepareTrainingEnemy {
            game_state,
            character_capture,
            output,
            distance,
            body_model,
        } => {
            codol_server::game_state::PreparedGameState::prepare_training_enemy(
                &game_state,
                &character_capture,
                &output,
                distance,
                &body_model,
            )?;
            println!(
                "{}",
                serde_json::json!({"prepared":output,"scope":"human_training_enemy"})
            );
        }
        Command::PrepareTrainingTarget {
            game_state,
            model_capture,
            output,
            distance,
            scale,
        } => {
            codol_server::game_state::PreparedGameState::prepare_training_target(
                &game_state,
                &model_capture,
                &output,
                distance,
                scale,
            )?;
            println!(
                "{}",
                serde_json::json!({"prepared":output,"scope":"near_range_training_target"})
            );
        }
        Command::BoostTestAccount {
            service_data_dir,
            title_id,
            user_id,
            cp,
            gp,
        } => {
            let services = codol_server::services::LocalServices::open(service_data_dir).await?;
            let backup = services
                .boost_test_account(title_id, user_id, cp, gp)
                .await?;
            println!(
                "{}",
                serde_json::json!({"backup":backup,"level":70,"rank_version":4,"experience_within_rank":0,"cp":cp,"gp":gp,"scope":"local_test_account"})
            );
        }
        Command::PreparePrivateProfile {
            assets,
            snapshot,
            service_data_dir,
            title_id,
            training_complete,
        } => {
            use base64::Engine;
            let read_json = |path: &PathBuf, limit: u64| -> Result<serde_json::Value> {
                let mut bytes = Vec::new();
                std::fs::File::open(path)?
                    .take(limit + 1)
                    .read_to_end(&mut bytes)?;
                ensure!(
                    bytes.len() as u64 <= limit,
                    "input exceeds preparation limit"
                );
                Ok(serde_json::from_slice(&bytes)?)
            };
            let asset = read_json(&assets, 16 * 1024 * 1024)?;
            let captured = read_json(&snapshot, 128 * 1024)?;
            for value in [&asset, &captured] {
                ensure!(
                    value["client_sha256"] == codol_protocol::CLIENT_SHA256
                        && value["read_only"] == true,
                    "expected target-version read-only extraction"
                );
            }
            let definition = &asset["schemas"]["mp/playersecuredata.def"]["definitions"][0];
            ensure!(
                definition["version"] == 19
                    && definition["formatChecksum"] == 2318499177u64
                    && definition["leafCount"] == 9498
                    && definition["size"] == 13739,
                "private definition differs from recovered v19 layout"
            );
            let uid = captured["user_id"]
                .as_str()
                .context("snapshot identity missing")?
                .parse::<u64>()?;
            let raw = base64::engine::general_purpose::STANDARD.decode(
                captured["private_snapshot"]
                    .as_str()
                    .context("private snapshot missing")?,
            )?;
            let data = codol_protocol::profile::prepare_private(&raw, uid, training_complete)?;
            let directory = service_data_dir
                .join("profiles")
                .join(title_id.to_string())
                .join(uid.to_string());
            std::fs::create_dir_all(&directory)?;
            let output = directory.join("private.bin");
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options
                .open(&output)
                .context("create private profile; existing data is never overwritten")?;
            file.write_all(&data)?;
            file.sync_all()?;
            println!(
                "{}",
                serde_json::json!({"created":output,"schema_version":19,"rank":1,"cqb_training_complete":training_complete,"scope":"local_private_profile_seed"})
            );
        }
        Command::InspectPublicProfile { input } => {
            let mut bytes = Vec::new();
            std::fs::File::open(input)?
                .take((codol_protocol::task::MAX_FILE_BYTES + 4) as u64)
                .read_to_end(&mut bytes)?;
            ensure!(
                bytes.len() <= codol_protocol::task::MAX_FILE_BYTES,
                "profile request exceeds byte limit"
            );
            let request = codol_protocol::task::Request::parse(&bytes)?;
            ensure!(
                request.service == 8 && request.task == 3,
                "expected a public profile upload"
            );
            codol_protocol::profile::validate_public(request.arguments)?;
            println!(
                "{}",
                serde_json::json!({"validated":true,"snapshot_bytes":codol_protocol::profile::SNAPSHOT_BYTES,"scope":"public_profile_upload_shape_zlib_crc"})
            );
        }
        Command::PreparePublisher { output_dir } => {
            let config = include_bytes!("../../../configs/publisher/configs_s.ini");
            let playlist = codol_protocol::publisher::encode_playlist(include_str!(
                "../../../configs/publisher/mp_playlists_shipRetail_release.info"
            ))?;
            let files = [
                (
                    "configs_s.info",
                    codol_protocol::publisher::crypt_configs(config)?,
                ),
                ("mp_playlists_shipRetail_release.info", playlist),
            ];
            for (name, _) in &files {
                ensure!(
                    !output_dir.join(name).try_exists()?,
                    "publisher file already exists: {name}"
                );
            }
            std::fs::create_dir_all(&output_dir)?;
            for (name, bytes) in files {
                let path = output_dir.join(name);
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)?;
                file.write_all(&bytes)?;
                file.sync_all()?;
                println!(
                    "Created {} ({} bytes; local research data)",
                    path.display(),
                    bytes.len()
                );
            }
        }
        Command::ServeAuth {
            bind,
            cert,
            key,
            capture_dir,
            reply_file,
            local_title_id,
            lobby_bind,
            service_data_dir,
            stun_bind,
            stun_alt,
            game_bind,
            game_state,
            snapshot_probe,
            game_map_catalog,
            native_bot_rooms,
            resource_capture_map,
            resource_compass_probe,
            continuous_play,
            game_capture_every,
            hit_regions,
        } => {
            let server = AuthServer::bind(AuthOptions {
                bind,
                certificate: cert,
                key,
                capture_dir,
                reply_file,
                local_title_id,
            })
            .await?;
            if let Some(bind) = lobby_bind {
                let mut lobby = LobbyServer::bind_with_services(
                    bind,
                    server.registry(),
                    server.directory().join("lobby"),
                    service_data_dir,
                )
                .await?;
                if continuous_play {
                    lobby = lobby.with_continuous_play()?;
                }
                tracing::info!(address=%lobby.address()?,"Rust lobby listener ready");
                tracing::info!(address=%server.address()?,directory=%server.directory().display(),"Rust AUTH listener ready");
                let (stop, receiver) = tokio::sync::watch::channel(false);
                let mut tasks = tokio::task::JoinSet::new();
                if let Some(bind) = game_bind {
                    let mut game = codol_server::game_server::GameServer::bind(
                        bind,
                        server.directory().join("game"),
                    )
                    .await?;
                    game = game.with_capture_every(game_capture_every);
                    if continuous_play {
                        game = game.with_continuous_play();
                    }
                    if let Some(path) = game_state {
                        let mut state = codol_server::game_state::PreparedGameState::load(&path)?;
                        if let Some(frames) = snapshot_probe {
                            state = state.with_snapshot_frames(frames)?;
                        }
                        if continuous_play {
                            state = state.with_continuous_play();
                        }
                        game = game.with_game_state(state);
                    }
                    if let Some(path) = hit_regions {
                        game = game.with_hit_regions(path);
                    }
                    if let Some(path) = game_map_catalog {
                        let mut catalog =
                            codol_server::game_map_catalog::GameMapCatalog::load(&path)?;
                        if let Some(frames) = snapshot_probe {
                            catalog = catalog.with_snapshot_frames(frames)?;
                        }
                        if continuous_play {
                            catalog = catalog.with_continuous_play();
                        }
                        game = game.with_game_map_catalog(catalog);
                    }
                    if native_bot_rooms {
                        let services = lobby
                            .services()
                            .ok_or_else(|| anyhow::anyhow!("bot rooms require local services"))?;
                        game = game
                            .with_native_bot_rooms(services.publisher_playlist_version().await?)?;
                    }
                    if let Some(map) = resource_capture_map {
                        game = game.with_resource_capture_map(map)?;
                    }
                    if let Some(probe) = resource_compass_probe {
                        game = game.with_resource_compass_probe(&probe)?;
                    }
                    let endpoint = game.admissions();
                    tracing::info!(address=%endpoint.address(),"experimental Rust game UDP endpoint ready");
                    lobby = lobby.with_game_endpoint(endpoint).await?;
                    game = game.with_services(lobby.services());
                    tasks.spawn(game.run(wait_for_stop(receiver.clone())));
                }
                if let Some(bind) = stun_bind {
                    let stun =
                        StunServer::bind(bind, stun_alt, server.directory().join("stun")).await?;
                    tracing::info!(address=%stun.address()?,"Rust STUN listener ready");
                    tasks.spawn(stun.run(wait_for_stop(receiver.clone())));
                }
                tasks.spawn(server.run(wait_for_stop(receiver.clone())));
                tasks.spawn(lobby.run(wait_for_stop(receiver)));
                let outcome = tokio::select! {
                    result=tokio::signal::ctrl_c()=>result.context("shutdown signal"),
                    result=tasks.join_next()=>result.context("service task missing")?.context("service task failed")?,
                };
                stop.send_replace(true);
                while let Some(result) = tasks.join_next().await {
                    result.context("service shutdown")??;
                }
                outcome?;
            } else {
                tracing::info!(address=%server.address()?, directory=%server.directory().display(), "Rust AUTH listener ready");
                server.run(tokio::signal::ctrl_c()).await?;
            }
        }
        Command::InspectAuth { input } => {
            let mut body = Vec::new();
            std::fs::File::open(&input)
                .with_context(|| format!("open {}", input.display()))?
                .take((codol_protocol::auth::MAX_AUTH_BODY + 1) as u64)
                .read_to_end(&mut body)?;
            let request = codol_protocol::auth::parse_task10(&body)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "auth_task": request.wire.auth_task,
                    "title_id": request.title_id,
                    "identity_bytes": request.wire.identity.len(),
                    "extra_version": request.extra.version,
                    "additional_fields": request.wire.additional_fields.keys().collect::<Vec<_>>(),
                    "scope": "request_parsing_only_not_authentication",
                }))?
            );
        }
        Command::InspectTls { input } => {
            let mut bytes = Vec::new();
            std::fs::File::open(&input)
                .with_context(|| format!("open {}", input.display()))?
                .take((codol_protocol::tls::MAX_PLAINTEXT_RECORD + 6) as u64)
                .read_to_end(&mut bytes)?;
            let hello = codol_protocol::tls::parse_client_hello(&bytes)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "record_version": format!("0x{:04x}", hello.record_version),
                    "client_version": format!("0x{:04x}", hello.client_version),
                    "cipher_suites": hello.cipher_suites.iter().map(|suite| format!("0x{suite:04x}")).collect::<Vec<_>>(),
                    "session_id_bytes": hello.session_id.len(),
                    "compression_methods": hello.compression_methods,
                    "extension_types": hello.extensions.iter().map(|extension| extension.kind).collect::<Vec<_>>(),
                    "scope": "single_record_inspection_only",
                }))?
            );
        }
        Command::CheckConfig { config } => {
            let config = Config::load(&config)?;
            println!("{}", serde_json::to_string_pretty(&config)?);
        }
        Command::Observe { config } => {
            let observer = Observer::bind(Config::load(&config)?).await?;
            tracing::info!(directory = %observer.directory().display(), "observation run created; no game responses enabled");
            for (name, address) in observer.addresses()? {
                tracing::info!(listener = name, %address, "TCP listener ready");
            }
            observer.run(tokio::signal::ctrl_c()).await?;
            tracing::info!("all capture sessions closed");
        }
        Command::Bootstrap {
            config,
            host,
            title_id_hex,
            output,
        } => {
            let config = Config::load(&config)?;
            let endpoint = |name: &str| -> Result<Endpoint> {
                let listener = config
                    .listeners
                    .iter()
                    .find(|l| l.name == name)
                    .with_context(|| format!("missing listener {name}"))?;
                Ok(Endpoint {
                    host: host.clone(),
                    port: listener.bind.port(),
                })
            };
            let hex = title_id_hex
                .strip_prefix("0x")
                .or_else(|| title_id_hex.strip_prefix("0X"))
                .unwrap_or(&title_id_hex);
            ensure!(
                !hex.is_empty() && hex.len() <= 8 && hex.bytes().all(|b| b.is_ascii_hexdigit()),
                "title-id-hex must be a u32 hexadecimal integer"
            );
            let bootstrap = DwauthConfig {
                endpoint_a: endpoint("endpoint_a")?,
                endpoint_b: endpoint("endpoint_b")?,
                title_id: u32::from_str_radix(hex, 16)?,
            };
            let contents = bootstrap.render()?;
            if output.exists() {
                bail!("refusing to overwrite {}", output.display());
            }
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&output)
                .with_context(|| format!("create {} (parent must exist)", output.display()))?;
            file.write_all(contents.as_bytes())?;
            println!(
                "Created {}. Format and title_id mapping are confirmed; this does not establish successful authentication.",
                output.display()
            );
        }
    }
    Ok(())
}

async fn wait_for_stop(mut receiver: tokio::sync::watch::Receiver<bool>) -> std::io::Result<()> {
    if !*receiver.borrow() {
        let _ = receiver.changed().await;
    }
    Ok(())
}
