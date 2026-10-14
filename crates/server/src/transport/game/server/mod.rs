//! Game UDP transport with matchmaking-bound captured map preparations.
#[cfg(test)]
use crate::backend::identity::session::Session;
use crate::backend::rooms::admission::Admission;
use crate::backend::rooms::admission::GameAdmissions;
use crate::backend::rooms::admission::story_capture;
use crate::infra::capture::unix_time_ms;
use crate::infra::storage::write_private;
use crate::transport::game::handshake::Peer;
use anyhow::{Result, ensure};
#[cfg(test)]
use base64::{Engine, engine::general_purpose::STANDARD};
#[cfg(test)]
use codol_protocol::lobby::task::Writer;
use std::{
    collections::HashMap,
    future::Future,
    net::{SocketAddr, SocketAddrV4},
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::net::UdpSocket;

pub struct GameServer {
    capture_every: u32,
    continuous_play: bool,
    socket: UdpSocket,
    admissions: Arc<GameAdmissions>,
    directory: PathBuf,
    game_state: Option<Arc<crate::game::session::prepared::PreparedGameState>>,
    story_resources: Option<Arc<crate::game::session::prepared::PreparedGameState>>,
    game_map_catalog: Option<crate::game::session::catalog::GameMapCatalog>,
    resource_capture_map: Option<String>,
    resource_capture_playlist: Option<(u32, u8)>,
    resource_compass_probe: Option<crate::content::maps::minimap::CompassProbe>,
    hit_regions: HitRegionSource,
    services: Option<Arc<crate::app::services::LocalServices>>,
}
/// Optional enemy pose capture. It may appear while the server runs, because
/// the capture is read from a client that has already entered training.
#[derive(Default)]
struct HitRegionSource {
    path: Option<PathBuf>,
    loaded: Option<Arc<crate::game::combat::hit_location::HitRegions>>,
    rejected: Option<std::time::SystemTime>,
}
impl HitRegionSource {
    fn poll(
        &mut self,
        ncs_crc: u32,
    ) -> Option<&Arc<crate::game::combat::hit_location::HitRegions>> {
        if self.loaded.is_none()
            && let Some(path) = &self.path
            && let Ok(modified) = std::fs::metadata(path).and_then(|m| m.modified())
            && self.rejected != Some(modified)
        {
            match crate::game::combat::hit_location::HitRegions::load(path, ncs_crc) {
                Ok(regions) => {
                    tracing::info!(path=%path.display(),source=regions.source_sha256(),"loaded enemy hit regions");
                    self.loaded = Some(Arc::new(regions));
                }
                Err(error) => {
                    tracing::warn!(path=%path.display(),error=format!("{error:#}"),"enemy hit regions rejected");
                    self.rejected = Some(modified);
                }
            }
        }
        self.loaded.as_ref()
    }
}
struct BoundPeer {
    transport: Peer,
    template: Option<Arc<crate::game::session::prepared::PreparedGameState>>,
    start_command: String,
}
type RegistryKey = (u32, u64, u32, String, String);
fn registry_key(
    admission: &Admission,
    state: &crate::game::session::prepared::PreparedGameState,
) -> RegistryKey {
    (
        admission.title,
        admission.owner,
        admission.seed,
        state.map().to_owned(),
        state.mode().to_owned(),
    )
}
impl BoundPeer {
    fn admitted(&self, admission: Option<&Admission>) -> bool {
        admission.is_some_and(|a| a.start_command.as_deref() == Some(&self.start_command))
    }
}
impl GameServer {
    pub async fn bind(address: SocketAddrV4, directory: PathBuf) -> Result<Self> {
        ensure!(
            address.ip().is_loopback(),
            "experimental game endpoint must bind loopback"
        );
        let socket = UdpSocket::bind(address).await?;
        let SocketAddr::V4(address) = socket.local_addr()? else {
            anyhow::bail!("IPv4 game endpoint required")
        };
        tokio::fs::create_dir_all(&directory).await?;
        write_private(&directory.join("run.json"),&serde_json::to_vec_pretty(&serde_json::json!({
            "mode":"experimental_game_udp_transport","bind":address,"simulation":false,"base_max_datagrams":2048,
            "client_sha256":codol_protocol::CLIENT_SHA256
        }))?).await?;
        Ok(Self {
            capture_every: 1,
            continuous_play: false,
            socket,
            directory,
            game_state: None,
            story_resources: None,
            game_map_catalog: None,
            resource_capture_map: None,
            resource_capture_playlist: None,
            resource_compass_probe: None,
            hit_regions: HitRegionSource::default(),
            services: None,
            admissions: Arc::new(GameAdmissions::new(address)),
        })
    }
    pub fn admissions(&self) -> Arc<GameAdmissions> {
        self.admissions.clone()
    }
    pub fn with_story_resource_state(mut self, path: &std::path::Path) -> Result<Self> {
        ensure!(
            self.admissions.story_capture_map.as_deref()
                == Some(codol_protocol::room::story::FIRST_MAP),
            "story metadata requires explicit native story capture"
        );
        self.story_resources = Some(Arc::new(
            crate::game::session::prepared::PreparedGameState::load_story_resources(path)?,
        ));
        Ok(self)
    }
    /// Collect the first story map's transport using its native UI entrance.
    /// This endpoint has no prepared gamestate or gameplay snapshots.
    pub fn with_story_capture_map(mut self, map: String) -> Result<Self> {
        ensure!(
            map == codol_protocol::room::story::FIRST_MAP,
            "only the first native story map is prepared for capture"
        );
        ensure!(
            self.game_state.is_none()
                && self.game_map_catalog.is_none()
                && self.resource_capture_map.is_none()
                && !self.continuous_play
                && self.admissions.bot_rooms.is_none(),
            "story capture requires a separate transport-only endpoint"
        );
        Arc::get_mut(&mut self.admissions)
            .ok_or_else(|| anyhow::anyhow!("story capture must precede admission sharing"))?
            .story_capture_map = Some(map);
        Ok(self)
    }
    pub fn with_continuous_play(mut self) -> Self {
        self.continuous_play = true;
        self
    }
    pub fn with_game_state(
        mut self,
        state: crate::game::session::prepared::PreparedGameState,
    ) -> Self {
        self.game_state = Some(Arc::new(state));
        self
    }
    pub fn with_capture_every(mut self, every: u32) -> Self {
        self.capture_every = every.max(1);
        self
    }
    pub fn with_services(
        mut self,
        services: Option<Arc<crate::app::services::LocalServices>>,
    ) -> Self {
        self.services = services;
        self
    }
    /// Captured enemy body pose (tools/client_state.py --hit-regions).
    pub fn with_hit_regions(mut self, path: PathBuf) -> Self {
        self.hit_regions.path = Some(path);
        self
    }
    pub fn with_game_map_catalog(
        mut self,
        catalog: crate::game::session::catalog::GameMapCatalog,
    ) -> Self {
        self.game_map_catalog = Some(catalog);
        self
    }
    pub fn with_native_bot_rooms(mut self, version: u32) -> Result<Self> {
        ensure!(version > 0, "native bot rooms require publisher version");
        let maps = self
            .game_map_catalog
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("native bot rooms require captured catalog"))?
            .bot_room_maps();
        ensure!(
            !maps.is_empty(),
            "native bot rooms require TDM preparations"
        );
        let loadouts = self
            .game_map_catalog
            .as_ref()
            .expect("validated catalog")
            .bot_room_loadouts();
        Arc::get_mut(&mut self.admissions)
            .ok_or_else(|| anyhow::anyhow!("bot room policy must precede admission sharing"))?
            .bot_rooms = Some(crate::backend::rooms::bots::Policy {
            version,
            maps,
            loadouts,
        });
        Ok(self)
    }
    /// Explicit single-map collection. Captured metadata may be initialized
    /// without gameplay snapshots or substitution of another map's CRC.
    pub fn with_resource_capture_map(mut self, map: String) -> Result<Self> {
        ensure!(
            crate::content::maps::registry::native_id(&map).is_some(),
            "resource capture requires a captured native TDM map identity"
        );
        let policy = Arc::get_mut(&mut self.admissions)
            .ok_or_else(|| anyhow::anyhow!("resource capture must precede admission sharing"))?
            .bot_rooms
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("resource capture requires native bot rooms"))?;
        policy.maps.insert(map.clone());
        self.resource_capture_map = Some(map);
        Ok(self)
    }
    pub fn with_resource_compass_probe(mut self, path: &std::path::Path) -> Result<Self> {
        let map = self
            .resource_capture_map
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("compass probe requires resource capture"))?;
        self.resource_compass_probe = Some(crate::content::maps::minimap::CompassProbe::load(
            path, map,
        )?);
        Ok(self)
    }
    /// A single-entry publisher playlist is the native route for maps which
    /// prohibit custom rooms. This opt-in still publishes no gameplay world.
    pub fn with_resource_capture_playlist(mut self, playlist: u8) -> Result<Self> {
        ensure!(
            playlist < 255 && self.resource_capture_map.is_some(),
            "resource playlist requires an explicit capture map and native playlist ID"
        );
        let version = self
            .admissions
            .bot_rooms
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("resource playlist requires local bot-room policy"))?
            .version;
        self.resource_capture_playlist = Some((version, playlist));
        Ok(self)
    }

    fn resource_template(
        &self,
        map: &str,
    ) -> Result<Option<Arc<crate::game::session::prepared::PreparedGameState>>> {
        self.game_map_catalog
            .as_ref()
            .and_then(|catalog| catalog.select(map, "war").ok())
            .map(|state| {
                let initialized = (*state).clone().resource_initialization()?;
                let initialized = if let Some(probe) = &self.resource_compass_probe {
                    initialized.with_resource_compass_probe(probe)?
                } else {
                    initialized
                };
                Ok(Arc::new(initialized))
            })
            .transpose()
    }
    fn template_for(
        &self,
        admission: &Admission,
    ) -> Result<Option<Arc<crate::game::session::prepared::PreparedGameState>>> {
        let start = admission
            .start_command
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("game start missing"))?;
        if let Some(map) = &self.admissions.story_capture_map {
            ensure!(
                admission.story_channel.is_some()
                    && admission.build == codol_protocol::room::story::CLIENT_BUILD
                    && self.game_state.is_none()
                    && self.game_map_catalog.is_none(),
                "story capture cannot use a client-key allocation or TDM preparation"
            );
            story_capture::selection(start, map)?;
            return Ok(self.story_resources.clone());
        }
        if let Some((version, playlist)) = self.resource_capture_playlist
            && let Some((selected, entry)) = crate::content::maps::selection::playlist_start(start)?
        {
            ensure!(
                admission.build == -688906341
                    && admission.playlist_version == Some(version)
                    && selected == u32::from(playlist)
                    && entry == 0,
                "resource playlist build/version/entry differs from the explicit capture binding"
            );
            return self.resource_template(self.resource_capture_map.as_deref().unwrap());
        }
        let parsed = crate::content::maps::selection::from_mmstart(start);
        if let Some(catalog) = self.game_map_catalog.as_ref().filter(|c| c.has_rotations())
            && let Some((playlist, entry)) = crate::content::maps::selection::playlist_start(start)?
        {
            ensure!(
                admission.build == -688906341,
                "prepared game state client build mismatch"
            );
            let version = admission.playlist_version.ok_or_else(|| {
                anyhow::anyhow!("playlist start requires booked publisher version")
            })?;
            return Ok(Some(catalog.select_rotation(version, playlist, entry)?));
        }
        // Transport-only recovery has no world to fall back to. Explicit
        // room maps always require a captured preparation, even in this mode.
        if self.game_state.is_none() && self.game_map_catalog.is_none() && parsed.is_err() {
            return Ok(None);
        }
        ensure!(
            admission.build == -688906341,
            "prepared game state client build mismatch"
        );
        match parsed? {
            Some(selection) => {
                if self.resource_capture_map.as_deref() == Some(&selection.map) {
                    ensure!(
                        selection.mode == "war",
                        "resource capture is limited to native TDM"
                    );
                    return self.resource_template(&selection.map);
                }
                let catalog = self.game_map_catalog.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("requested room map has no preparation catalog")
                })?;
                let state = catalog.select(&selection.map, &selection.mode)?;
                Ok(Some(if let Some(teams) = admission.room_teams {
                    Arc::new((*state).clone().with_room_teams(teams)?)
                } else {
                    state
                }))
            }
            None => {
                if let Some(state) = &self.game_state {
                    ensure!(
                        state.matches(admission.build, start),
                        "default preparation requires the captured training request"
                    );
                }
                ensure!(
                    self.game_state.is_some() || self.game_map_catalog.is_none(),
                    "training start has no default preparation"
                );
                Ok(self.game_state.clone())
            }
        }
    }
    pub async fn run<F: Future<Output = std::io::Result<()>>>(mut self, shutdown: F) -> Result<()> {
        if let Some(map) = &self.admissions.story_capture_map {
            ensure!(
                self.game_state.is_none()
                    && self.game_map_catalog.is_none()
                    && self.resource_capture_map.is_none()
                    && !self.continuous_play
                    && self.admissions.bot_rooms.is_none(),
                "story capture cannot publish a TDM world"
            );
            write_private(
                &self.directory.join("story-capture.json"),
                &serde_json::to_vec(
                    &serde_json::json!({"map":map,"mode":"coop","maximum_players":1,
                "server_issued_key":true,"world_simulation":false,"gameplay_snapshots":false,
                "resource_gamestate_available":self.story_resources.is_some(),
                "client_sha256":codol_protocol::CLIENT_SHA256}),
                )?,
            )
            .await?;
        }
        if let Some(map) = &self.resource_capture_map {
            let initializes = self
                .game_map_catalog
                .as_ref()
                .is_some_and(|catalog| catalog.select(map, "war").is_ok());
            write_private(
                &self.directory.join("resource-capture.json"),
                &serde_json::to_vec(
                    &serde_json::json!({"map":map,"mode":"war", "world_simulation":false,
                    "resource_gamestate_available":initializes,"gameplay_snapshots":false,
                    "compass_candidate_requested":self.resource_compass_probe.is_some(),
                    "client_sha256":codol_protocol::CLIENT_SHA256}),
                )?,
            )
            .await?;
        }
        if self.continuous_play {
            write_private(&self.directory.join("play-mode.json"),&serde_json::to_vec(&serde_json::json!({"snapshot_frame_limit":null,"capture_receive_byte_limit":32*1024*1024,"capture_every":self.capture_every}))?).await?;
        }
        tokio::pin!(shutdown);
        let mut buffer = [0; 4096];
        let mut count = 0u32;
        let mut total = 0usize;
        let mut peers = HashMap::<SocketAddrV4, BoundPeer>::new();
        let mut weapon_registries = HashMap::<
            RegistryKey,
            Arc<Mutex<crate::game::session::weapon_registry::WeaponRegistry>>,
        >::new();
        let datagram_limit = 2048
            + self
                .game_state
                .as_ref()
                .map_or(0, |s| s.snapshot_limit())
                .max(
                    self.game_map_catalog
                        .as_ref()
                        .map_or(0, |c| c.snapshot_limit()),
                )
                .min(18000)
                * 4;
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(50));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut snapshots = 0u32;
        let mut profile_check = std::time::Instant::now();
        loop {
            let (n, peer) = tokio::select! {result=&mut shutdown=>{result?;return Ok(())},
            _=tick.tick()=>{
                let now=(unix_time_ms()/1000) as u32;
                peers.retain(|_,p|p.admitted(self.admissions.get(&p.transport.id,now).as_ref()));
                if profile_check.elapsed()>=std::time::Duration::from_millis(500) {
                    profile_check=std::time::Instant::now();
                    for bound in peers.values_mut() {
                        if let Some(crc)=bound.template.as_ref().map(|t|t.ncs_crc())
                            && let Some(regions)=self.hit_regions.poll(crc) {
                            bound.transport.connection.set_hit_regions(regions);
                        }
                    }
                    if let Some(services)=&self.services {
                        for bound in peers.values_mut() {
                            let Some(template)=bound.template.as_ref().filter(|s|s.dynamic_loadout()) else {continue};
                            let p=&mut bound.transport;
                            if let Some(admission)=self.admissions.get(&p.id,now) {
                                match services.game_profile(admission.title,admission.owner).await {
                                    Ok(profile)=>if let Err(error)=p.connection.refresh_profile(template,&profile.public,&profile.owned,profile.rank){tracing::warn!(%error,"player backpack update rejected");},
                                    Err(error)=>tracing::warn!(%error,"saved backpack read failed"),
                                }
                            }
                        }
                    }
                }
                for (address,bound) in &mut peers {
                    let p=&mut bound.transport;
                    let Some(records)=p.records.as_mut() else { continue };
                    if let Some(notice)=p.connection.poll_profile_ready() {
                        let wire=records.seal(notice,p.remote_tag)?;
                        self.socket.send_to(&wire,*address).await?;
                        tracing::info!("native profile persistence ready notice sent");
                    }
                    if let Some(plain)=p.connection.poll_snapshot(std::time::Instant::now())? {
                        let wire=records.seal(&plain,p.remote_tag)?;
                        self.socket.send_to(&wire,*address).await?;
                        snapshots+=1;
                        if (!self.continuous_play || (count<=datagram_limit && total<=32*1024*1024)) && (snapshots.is_multiple_of(self.capture_every) || snapshots<10) {
                        write_private(&self.directory.join(format!("snapshot-{snapshots:04}-plaintext.bin")),&plain).await?;
                        write_private(&self.directory.join(format!("snapshot-{snapshots:04}-reply.bin")),&wire).await?;
                        write_private(&self.directory.join(format!("snapshot-{snapshots:04}.json")),&serde_json::to_vec(&serde_json::json!({
                            "peer":address,"native_sequence":u32::from_le_bytes(plain[..4].try_into()?),
                            "bytes":wire.len(),"unix_time_ms":unix_time_ms(),"mode":if self.continuous_play {"local_play"}else{"bounded_bootstrap_probe"},"motion":p.connection.motion_progress(),"prediction":p.connection.prediction_progress()
                        }))?).await?;
                        }
                    }
                }
                continue;
            },
            result=self.socket.recv_from(&mut buffer)=>result?};
            let SocketAddr::V4(peer) = peer else { continue };
            if !peer.ip().is_loopback()
                || (!self.continuous_play
                    && (count >= datagram_limit || total.saturating_add(n) > 32 * 1024 * 1024))
            {
                continue;
            }
            count = count.saturating_add(1);
            total = total.saturating_add(n);
            let wire = &buffer[..n];
            let sampled = count.is_multiple_of(self.capture_every) || wire.first() != Some(&6);
            let now = (unix_time_ms() / 1000) as u32;
            let live = self.admissions.has_live_entries(now)?;
            peers.retain(|_, p| p.admitted(self.admissions.get(&p.transport.id, now).as_ref()));
            let mut decoded = None;
            let inner_kind: Option<&str> = None;
            let mut inner_dropped = None;
            let mut client_server_id = None;
            let mut client_message_ack = None;
            let mut client_reliable_ack = None;
            let mut client_body_first = None;
            let mut client_opcode = None;
            let handled: Result<Option<Vec<u8>>> = async {
                if live && n == 29 && wire[..3] == [13, 2, 0] {
                    let mut reply = wire.to_vec();
                    reply[0] = 12;
                    return Ok(Some(reply));
                } else if n == 16 && wire[..2] == [1, 2] {
                    let id: [u8; 8] = wire[8..16].try_into()?;
                    let admission = self
                        .admissions
                        .get(&id, now)
                        .ok_or_else(|| anyhow::anyhow!("game INIT is not admitted/started"))?;
                    if !peers.get(&peer).is_some_and(|p| p.transport.is_retry(wire)) {
                        ensure!(
                            peers.len() < 32 || peers.contains_key(&peer),
                            "game peer capacity"
                        );
                        let template = self.template_for(&admission)?;
                        let mut connection = Peer::init(wire, peer)?;
                        if let Some(state) = &template {
                            let mut weapon_registry = None;
                            let state = if state.uses_player_loadout() {
                                let services = self.services.as_ref().ok_or_else(|| {
                                    anyhow::anyhow!("player loadout requires local services")
                                })?;
                                let profile = services
                                    .game_profile(admission.title, admission.owner)
                                    .await?;
                                let (public, owned) = (&profile.public, &profile.owned);
                                let selected = state.for_initial_backpack(public)?;
                                let state = &selected;
                                if state.dynamic_loadout() {
                                    let registry = weapon_registries
                                        .entry(registry_key(&admission, state))
                                        .or_insert_with(|| {
                                            Arc::new(Mutex::new(
                                                crate::game::session::weapon_registry::WeaponRegistry::new(
                                                    state.first_weapon_index(),
                                                ),
                                            ))
                                        })
                                        .clone();
                                    let mut prepared = state.for_profile_at(
                                        public,
                                        owned,
                                        state.first_weapon_index(),
                                    )?;
                                    prepared.set_card_rank(profile.rank);
                                    {
                                        let mut guard = registry.lock().map_err(|_| {
                                            anyhow::anyhow!("weapon registry poisoned")
                                        })?;
                                        let mut candidate = guard.clone();
                                        prepared.register_history(&mut candidate)?;
                                        codol_protocol::game::channel::wire::compact_server_packet(
                                            3,
                                            0,
                                            0,
                                            b"",
                                            &prepared.body(1)?,
                                        )?;
                                        *guard = candidate;
                                    }
                                    weapon_registry = Some(registry);
                                    Arc::new(prepared)
                                } else {
                                    let mut prepared = state.for_profile(public, owned)?;
                                    prepared.set_card_rank(profile.rank);
                                    Arc::new(prepared)
                                }
                            } else {
                                state.clone()
                            };
                            let resource_collection = self.resource_capture_map.as_deref()
                                == Some(state.map())
                                || admission.story_channel.is_some();
                            ensure!(
                                !state.has_nuke_diagnostics()
                                    || self
                                        .services
                                        .as_ref()
                                        .is_some_and(|s| s.permits_nuke_diagnostics()),
                                "nuclear diagnostics require marked isolated service data"
                            );
                            connection.connection =
                                crate::transport::game::connection::Connection::with_game_state(state);
                            if let Some(services) = &self.services {
                                let account = crate::contracts::account::AccountId { title_id: admission.title, user_id: admission.owner };
                                connection.connection.set_player_name(services.player_nickname(account).await?);
                            }
                            if resource_collection {
                                connection.connection =
                                    connection.connection.with_resource_collection();
                            }
                            if let Some(registry) = weapon_registry {
                                connection.connection =
                                    connection.connection.with_weapon_registry(registry);
                            }
                        }
                        peers.insert(
                            peer,
                            BoundPeer {
                                transport: connection,
                                template,
                                start_command: admission
                                    .start_command
                                    .clone()
                                    .expect("admitted start"),
                            },
                        );
                    }
                    return Ok(Some(peers[&peer].transport.init_reply()));
                } else if n == 189 && wire[..2] == [3, 2] {
                    let p = &mut peers
                        .get_mut(&peer)
                        .ok_or_else(|| anyhow::anyhow!("cookie without INIT"))?
                        .transport;
                    let admission = self
                        .admissions
                        .get(&p.id, now)
                        .ok_or_else(|| anyhow::anyhow!("expired cookie admission"))?;
                    return Ok(Some(p.cookie(wire, &admission.key)?));
                } else if n >= 16 && wire[..2] == [6, 2] {
                    let p = &mut peers
                        .get_mut(&peer)
                        .ok_or_else(|| anyhow::anyhow!("DATA without peer"))?
                        .transport;
                    let records = p
                        .records
                        .as_mut()
                        .ok_or_else(|| anyhow::anyhow!("DATA before cookie exchange"))?;
                    decoded = Some(records.open(wire, p.local_tag)?);
                    let admission = self
                        .admissions
                        .get(&p.id, now)
                        .ok_or_else(|| anyhow::anyhow!("DATA admission expired"))?;
                    let payload = decoded.as_ref().expect("decoded DATA");
                    let payload =
                        [payload.encrypted.as_slice(), payload.cleartext.as_slice()].concat();
                    if payload.len() >= 14 && payload[..4] != [255, 255, 255, 255] {
                        let header = codol_protocol::game::channel::wire::ClientHeader::parse(&payload)?;
                        let previous_sequence = p.inner_sequence;
                        p.inner_sequence = Some(header.sequence);
                        client_server_id = Some(header.server_id);
                        client_message_ack = Some(header.message_ack);
                        client_reliable_ack = Some(header.reliable_ack);
                        client_body_first = payload.get(13).copied();
                        let (_, clear) = p.connection.decode_native(&payload)?;
                        client_opcode = clear.first().map(|b| b & 7);
                        inner_dropped = previous_sequence
                            .map(|previous| header.sequence.saturating_sub(previous + 1));
                    }
                    if let Some(reply) =
                        p.connection
                            .handle(&payload, admission.owner, admission.build)?
                    {
                        return Ok(Some(records.seal(&reply, p.remote_tag)?));
                    }
                }
                Ok(None)
            }
            .await;
            let connection_stage = peers.get(&peer).map(|p| p.transport.connection.stage());
            let gamestate_progress = peers
                .get(&peer)
                .and_then(|p| p.transport.connection.gamestate_progress());
            let input_progress = peers
                .get(&peer)
                .map(|p| p.transport.connection.input_progress());
            let input_angles = peers
                .get(&peer)
                .map(|p| p.transport.connection.view_angles());
            let error = handled.as_ref().err().map(ToString::to_string);
            let response = handled.unwrap_or(None);
            if let Some(error) = &error {
                tracing::warn!(%peer,%error,"game input rejected");
            }
            let capture = (!self.continuous_play
                || (count <= datagram_limit && total <= 32 * 1024 * 1024))
                && (sampled || error.is_some() || response.is_some());
            if capture {
                write_private(
                    &self.directory.join(format!("{count:04}-request.bin")),
                    wire,
                )
                .await?;
            }
            if capture && let Some(plain) = &decoded {
                write_private(
                    &self.directory.join(format!("{count:04}-plaintext.bin")),
                    &[plain.encrypted.as_slice(), plain.cleartext.as_slice()].concat(),
                )
                .await?;
            }
            if let Some(reply) = &response {
                self.socket.send_to(reply, peer).await?;
                if capture {
                    write_private(&self.directory.join(format!("{count:04}-reply.bin")), reply)
                        .await?;
                }
            }
            // INIT metadata only; security material remains in memory/capture.
            let admitted = if n == 16 && wire[0] == 1 {
                let id: [u8; 8] = wire[8..16].try_into()?;
                self.admissions.get(&id, now).is_some()
            } else {
                false
            };
            if capture {
                write_private(&self.directory.join(format!("{count:04}-event.json")),&serde_json::to_vec(&serde_json::json!({
                "peer":peer,"bytes":n,"type":wire.first(),"version_byte":wire.get(1),"admitted_init":admitted,
                "reply_bytes":response.as_ref().map_or(0,Vec::len),"unix_time_ms":unix_time_ms(),"gamestate_progress":gamestate_progress,"input_progress":input_progress,"input_angles":input_angles
                ,"connection_stage":connection_stage,"inner_kind":inner_kind,"inner_dropped":inner_dropped,"client_server_id":client_server_id,"client_message_ack":client_message_ack,"client_reliable_ack":client_reliable_ack,"client_body_first":client_body_first,"client_opcode":client_opcode,"mac_verified":decoded.is_some(),"plaintext_bytes":decoded.as_ref().map(|p| p.encrypted.len() + p.cleartext.len()),"encrypted_payload_bytes":decoded.as_ref().map(|p| p.encrypted.len()),"error":error
            }))?).await?;
                tracing::info!(%peer,bytes=n,kind=?wire.first(),admitted_init=admitted,mac_verified=decoded.is_some(),"game UDP datagram captured");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        Session {
            title_id: 1002,
            user_id: u64::MAX - 1,
            iv_seed: 41,
            expires_at: 100,
            key: [0; 24],
        }
    }
    fn command(text: &str) -> Vec<u8> {
        let mut w = Writer::default();
        w.u64(1_000_001);
        let mut data = (text.len() as u32).to_le_bytes().to_vec();
        data.extend_from_slice(text.as_bytes());
        w.blob(&data).unwrap();
        w.0.push(0);
        w.0
    }

    #[tokio::test]
    async fn explicit_room_maps_bind_preparations_without_default_fallback() {
        let temp = tempfile::tempdir().unwrap();
        crate::game::session::catalog::write_synthetic(
            &temp.path().join("afghan.json"),
            "mp_afghan",
        );
        crate::game::session::catalog::write_synthetic(
            &temp.path().join("killhouse.json"),
            "mp_killhouse_sh",
        );
        let manifest = temp.path().join("maps.json");
        std::fs::write(
            &manifest,
            serde_json::to_vec(&serde_json::json!({
                "schema_version":1,"entries":[
                    {"map":"mp_afghan","mode":"war","file":"afghan.json"},
                    {"map":"mp_killhouse_sh","mode":"war","file":"killhouse.json"}
                ], "rotations":[{"version":6,"playlist":0,"maps":[
                    {"map":"mp_afghan","mode":"war"},
                    {"map":"mp_killhouse_sh","mode":"war"}
                ]}]
            }))
            .unwrap(),
        )
        .unwrap();
        let server = GameServer::bind("127.0.0.1:0".parse().unwrap(), temp.path().join("game"))
            .await
            .unwrap()
            .with_game_state(crate::game::session::prepared::synthetic())
            .with_game_map_catalog(
                crate::game::session::catalog::GameMapCatalog::load(&manifest).unwrap(),
            );
        let mut admission = Admission {
            title: 1002,
            owner: 9,
            seed: 41,
            build: -688906341,
            expires: 100,
            key: [7; 16],
            booked: true,
            playlist_version: None,
            room_teams: None,
            story_channel: None,
            start_command: Some(
                "mmstart sv_resetServer; map mp_killhouse_sh; g_gametype war".into(),
            ),
        };
        let selected = server.template_for(&admission).unwrap().unwrap();
        assert_eq!(selected.map(), "mp_killhouse_sh");
        let mut wire = vec![1, 2, 0, 0, 0, 0, 42, 0];
        wire.extend_from_slice(&[3; 8]);
        let bound = BoundPeer {
            transport: Peer::init(&wire, "127.0.0.1:39001".parse().unwrap()).unwrap(),
            template: Some(selected.clone()),
            start_command: admission.start_command.clone().unwrap(),
        };
        assert!(bound.admitted(Some(&admission)));
        assert!(!bound.admitted(None));
        admission.start_command =
            Some("mmstart sv_resetServer; map mp_afghan; g_gametype war".into());
        assert!(!bound.admitted(Some(&admission))); // Old DATA cannot hot-switch maps.
        let fresh = server.template_for(&admission).unwrap().unwrap();
        assert_eq!(fresh.map(), "mp_afghan");
        assert_ne!(
            registry_key(&admission, &selected),
            registry_key(&admission, &fresh)
        );
        let mut other_session = admission.clone();
        other_session.seed += 1;
        assert_ne!(
            registry_key(&admission, &selected),
            registry_key(&other_session, &selected)
        );
        assert_eq!(bound.template.as_ref().unwrap().map(), "mp_killhouse_sh");
        assert!(!Arc::ptr_eq(&selected, &fresh));
        for request in [
            "mmstart sv_resetServer; map mp_unknown; g_gametype war",
            "mmstart sv_resetServer; map mp_killhouse; g_gametype war",
            "mmstart sv_resetServer; map mp_killhouse_sh; g_gametype sd",
            "mmstart sv_resetServer; map mp_afghan; g_gametype war; quit",
        ] {
            admission.start_command = Some(request.into());
            assert!(server.template_for(&admission).is_err(), "{request}");
        }
        admission.start_command = Some(
            "mmstart sv_resetServer; 2486 1; 3265 12; 2489 1; playlist 0; start_map_rotate_once 0"
                .into(),
        );
        assert!(server.template_for(&admission).is_err());
        admission.playlist_version = Some(5);
        assert!(server.template_for(&admission).is_err());
        admission.playlist_version = Some(6);
        assert_eq!(
            server.template_for(&admission).unwrap().unwrap().map(),
            "mp_afghan"
        );
        admission.start_command = Some(
            admission
                .start_command
                .unwrap()
                .replace("rotate_once 0", "rotate_once 1"),
        );
        assert_eq!(
            server.template_for(&admission).unwrap().unwrap().map(),
            "mp_killhouse_sh"
        );
        admission.start_command = Some(
            admission
                .start_command
                .unwrap()
                .replace("rotate_once 1", "rotate_once 2"),
        );
        assert!(server.template_for(&admission).is_err());
        admission.build = 0;
        assert!(server.template_for(&admission).is_err());
    }

    #[tokio::test]
    async fn resource_collection_is_explicit_and_cannot_fall_back_to_another_world() {
        let temp = tempfile::tempdir().unwrap();
        let mut server = GameServer::bind("127.0.0.1:0".parse().unwrap(), temp.path().join("game"))
            .await
            .unwrap()
            .with_game_state(crate::game::session::prepared::synthetic());
        let mut admission = Admission {
            title: 1002,
            owner: 9,
            seed: 41,
            build: -688906341,
            expires: 100,
            key: [7; 16],
            booked: true,
            playlist_version: None,
            room_teams: None,
            story_channel: None,
            start_command: Some("mmstart sv_resetServer; map mp_bog_sh; g_gametype war".into()),
        };
        assert!(server.template_for(&admission).is_err());
        Arc::get_mut(&mut server.admissions).unwrap().bot_rooms =
            Some(crate::backend::rooms::bots::Policy {
                version: 7,
                maps: ["mp_afghan".into()].into(),
                loadouts: Default::default(),
            });
        let server = server
            .with_resource_capture_map("mp_bog_sh".into())
            .unwrap();
        assert!(server.template_for(&admission).unwrap().is_none());
        assert!(
            server
                .admissions
                .bot_rooms
                .as_ref()
                .unwrap()
                .maps
                .contains("mp_bog_sh")
        );
        for start in [
            "mmstart sv_resetServer; map mp_nuked; g_gametype war",
            "mmstart sv_resetServer; map mp_bog_sh; g_gametype sd",
            "mmstart sv_resetServer; map mp_bog_sh; g_gametype war; quit",
        ] {
            admission.start_command = Some(start.into());
            assert!(server.template_for(&admission).is_err());
        }
        admission.start_command =
            Some("mmstart sv_resetServer; map mp_bog_sh; g_gametype war".into());
        admission.build = 0;
        assert!(server.template_for(&admission).is_err());
        admission.build = -688906341;
        let metadata = temp.path().join("bog.json");
        crate::game::session::catalog::write_synthetic(&metadata, "mp_bog_sh");
        let mut data: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&metadata).unwrap()).unwrap();
        data["ncs_crc"] = serde_json::json!(0x87654321u32);
        std::fs::write(&metadata, serde_json::to_vec(&data).unwrap()).unwrap();
        let manifest = temp.path().join("capture-maps.json");
        std::fs::write(
            &manifest,
            serde_json::to_vec(&serde_json::json!({"schema_version":1,
            "entries":[{"map":"mp_bog_sh","mode":"war","file":"bog.json"}]}))
            .unwrap(),
        )
        .unwrap();
        let server = server.with_game_map_catalog(
            crate::game::session::catalog::GameMapCatalog::load(&manifest)
                .unwrap()
                .with_continuous_play(),
        );
        let initialized = server.template_for(&admission).unwrap().unwrap();
        assert_eq!(initialized.map(), "mp_bog_sh");
        assert_eq!(initialized.ncs_crc(), 0x87654321);
        assert!(!initialized.snapshot_probe());
        assert_eq!(initialized.snapshot_limit(), 0);
        let server = server.with_resource_capture_playlist(0).unwrap();
        admission.start_command = Some(
            "mmstart sv_resetServer; 2486 1; 3265 12; 2489 1; playlist 0; start_map_rotate_once 0"
                .into(),
        );
        admission.playlist_version = Some(7);
        let playlist_metadata = server.template_for(&admission).unwrap().unwrap();
        assert_eq!(playlist_metadata.map(), "mp_bog_sh");
        assert_eq!(playlist_metadata.ncs_crc(), 0x87654321);
        assert!(!playlist_metadata.snapshot_probe());
        for (version, playlist, entry) in [
            (None, 0, 0),
            (Some(6), 0, 0),
            (Some(7), 1, 0),
            (Some(7), 0, 1),
        ] {
            admission.playlist_version = version;
            admission.start_command = Some(format!(
                "mmstart sv_resetServer; 2486 1; 3265 12; 2489 1; playlist {playlist}; start_map_rotate_once {entry}"
            ));
            assert!(server.template_for(&admission).is_err());
        }
        assert!(
            server
                .with_resource_capture_map("mp_unknown".into())
                .is_err()
        );
    }
    #[tokio::test]
    async fn an_unprepared_room_map_is_rejected_even_when_a_default_is_available() {
        let temp = tempfile::tempdir().unwrap();
        let server = GameServer::bind("127.0.0.1:0".parse().unwrap(), temp.path().join("game"))
            .await
            .unwrap()
            .with_game_state(crate::game::session::prepared::synthetic());
        let admission = Admission {
            title: 1002,
            owner: 9,
            seed: 41,
            build: -688906341,
            expires: 100,
            key: [7; 16],
            booked: true,
            playlist_version: None,
            room_teams: None,
            story_channel: None,
            start_command: Some(
                "mmstart sv_resetServer; map mp_killhouse_sh; g_gametype war".into(),
            ),
        };
        assert!(server.template_for(&admission).is_err());
        let transport_only = GameServer::bind(
            "127.0.0.1:0".parse().unwrap(),
            temp.path().join("transport"),
        )
        .await
        .unwrap();
        assert!(transport_only.template_for(&admission).is_err());
    }

    #[tokio::test]
    async fn udp_admission_handshake_and_authenticated_challenge_roundtrip() {
        use std::time::Duration;
        let temp = tempfile::tempdir().unwrap();
        let server = GameServer::bind("127.0.0.1:0".parse().unwrap(), temp.path().join("game"))
            .await
            .unwrap();
        let a = server.admissions();
        let now = (unix_time_ms() / 1000) as u32;
        let mut s = session();
        s.expires_at = now + 30;
        let (id, _) = a.allocate(&s, [7; 16], -688906341, now).unwrap();
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(server.run(async {
            let _ = stopped.await;
            Ok(())
        }));
        let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut init = vec![1, 2, 0, 0, 0, 0, 42, 0];
        init.extend_from_slice(&id);
        let mut buffer = [0; 256];
        client.send_to(&init, a.address()).await.unwrap();
        // This waits for an on-disk observation, not a network reply. Cold
        // filesystem access during a full test suite can exceed two seconds;
        // the actual handshake response deadlines below remain two seconds.
        tokio::time::timeout(Duration::from_secs(10), async {
            while !temp.path().join("game/0001-event.json").exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|error| {
            let files = std::fs::read_dir(temp.path().join("game"))
                .unwrap()
                .filter_map(Result::ok)
                .map(|entry| entry.file_name())
                .collect::<Vec<_>>();
            panic!(
                "rejected INIT observation timed out: {error}; server_finished={}; files={files:?}",
                task.is_finished()
            );
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(100), client.recv_from(&mut buffer))
                .await
                .is_err()
        );
        a.control(
            &command(&format!("mmbook 4 {} 0", STANDARD.encode(id))),
            &s,
            now,
        )
        .unwrap();
        a.control(&command("mmstart playlist 0"), &s, now).unwrap();
        client.send_to(&init, a.address()).await.unwrap();
        let (n, source) =
            tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut buffer))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(source, SocketAddr::V4(a.address()));
        assert_eq!(n, 38);
        assert_eq!(&buffer[..4], &[2, 2, 42, 0]);
        let first = buffer[..n].to_vec();
        client.send_to(&init, a.address()).await.unwrap();
        let (n, _) = tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut buffer))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&buffer[..n], first);
        let local_tag = u16::from_le_bytes([first[14], first[15]]);
        let group = openssl::ec::EcGroup::from_curve_name(openssl::nid::Nid::SECP224R1).unwrap();
        let ec = openssl::ec::EcKey::generate(&group).unwrap();
        let public = crate::transport::game::handshake::encode_public(&ec).unwrap();
        let key = openssl::pkey::PKey::from_ec_key(ec).unwrap();
        let mut cookie = vec![3, 2];
        cookie.extend_from_slice(&local_tag.to_le_bytes());
        cookie.extend_from_slice(&[0, 0]);
        cookie.extend_from_slice(&first);
        cookie.extend_from_slice(&[0; 37]);
        cookie.extend_from_slice(&id);
        cookie.extend_from_slice(&public);
        client.send_to(&cookie, a.address()).await.unwrap();
        let (n, _) = tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut buffer))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(n, 114);
        assert_eq!(&buffer[..6], &[4, 2, 42, 0, 0, 0]);
        let shared =
            crate::transport::game::handshake::shared_secret(&key, &buffer[6..106]).unwrap();
        let mut records = codol_protocol::game::channel::records::Records::new(
            codol_protocol::game::channel::records::derive_key(&shared, &[7; 16]),
        );
        let mut challenge = vec![255; 4];
        challenge.extend_from_slice(format!("getchallenge 0 \"{:016x}\"\0", s.user_id).as_bytes());
        let mut challenge_number = String::new();
        for _ in 0..2 {
            client
                .send_to(&records.seal(&challenge, local_tag).unwrap(), a.address())
                .await
                .unwrap();
            let (n, _) =
                tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut buffer))
                    .await
                    .unwrap()
                    .unwrap();
            let reply = records.open(&buffer[..n], 42).unwrap();
            assert!(
                reply
                    .encrypted
                    .starts_with(b"\xff\xff\xff\xffchallengeResponse ")
            );
            assert!(reply.cleartext.is_empty());
            challenge_number = std::str::from_utf8(&reply.encrypted[22..reply.encrypted.len() - 1])
                .unwrap()
                .to_owned();
        }
        let mut connect = vec![255; 4];
        connect.extend_from_slice(format!("connect {0:016x} \"\\xuid\\{0:016x}\\challenge\\{challenge_number}\\2490\\-688906341\"", s.user_id).as_bytes());
        client
            .send_to(&records.seal(&connect, local_tag).unwrap(), a.address())
            .await
            .unwrap();
        let (n, _) = tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut buffer))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            records.open(&buffer[..n], 42).unwrap().encrypted,
            b"\xff\xff\xff\xffconnectResponse\0"
        );
        a.revoke(&s);
        client.send_to(&init, a.address()).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), client.recv_from(&mut buffer))
                .await
                .is_err()
        );
        stop.send(()).unwrap();
        task.await.unwrap().unwrap();
    }
    const START: &str =
        "mmstart sv_resetServer; 2486 1; 3265 1; g_gametype coop; map coop_training_mp_cargoship";
    #[tokio::test]
    async fn story_endpoint_never_falls_back_to_a_tdm_world() {
        let temp = tempfile::tempdir().unwrap();
        let server = GameServer::bind("127.0.0.1:0".parse().unwrap(), temp.path().join("game"))
            .await
            .unwrap()
            .with_story_capture_map(codol_protocol::room::story::FIRST_MAP.into())
            .unwrap();
        let a = server.admissions();
        let s = session();
        let (id, _) = a
            .allocate_story(&s, 7, codol_protocol::room::story::CLIENT_BUILD, 1)
            .unwrap();
        a.control(
            &command(&format!("mmbook 7 {} 0", STANDARD.encode(id))),
            &s,
            2,
        )
        .unwrap();
        a.control(&command(START), &s, 2).unwrap();
        let admission = a.get(&id, 2).unwrap();
        assert!(server.template_for(&admission).unwrap().is_none());
        let mixed = server.with_game_state(crate::game::session::prepared::synthetic());
        assert!(mixed.template_for(&admission).is_err());
        let plain = GameServer::bind("127.0.0.1:0".parse().unwrap(), temp.path().join("plain"))
            .await
            .unwrap();
        assert!(
            plain
                .admissions()
                .allocate_story(&s, 7, codol_protocol::room::story::CLIENT_BUILD, 1)
                .is_err()
        );
        assert!(
            plain
                .with_game_state(crate::game::session::prepared::synthetic())
                .with_story_capture_map(codol_protocol::room::story::FIRST_MAP.into())
                .is_err()
        );
    }
}
