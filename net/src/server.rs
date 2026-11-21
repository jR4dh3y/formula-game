use crate::{codec::Encoded, *};
use crate::{transport::ServerTransport, websocket::WsServer};
use racecraft_sim::{CarInput, Phase, Race, RaceEvent, PHYSICS_RATE};
use renet::RenetServer;
use renet_netcode::{NetcodeServerTransport, ServerAuthentication, ServerConfig};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    net::{SocketAddr, UdpSocket},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
struct Pending {
    name: String,
    expires: Duration,
}
#[derive(Default)]
struct Controls {
    samples: BTreeMap<u64, PackedInput>,
    commands: BTreeMap<u64, (u64, Command)>,
    received: u64,
    applied: u64,
    last_tick: u64,
    input: PackedInput,
    /// Newest snapshot tick the client reports holding; the delta baseline.
    ack: u64,
}
/// Sent snapshots kept as delta baselines: 64 × 33 ms ≈ two seconds.
const HISTORY: usize = 64;
/// How often every driver's round-trip time is shared with the room.
const PING_INTERVAL: Duration = Duration::from_secs(1);
/// Rejected joins get this long to receive the reason before disconnection.
const REJECT_GRACE: Duration = Duration::from_millis(500);
/// Main-thread independent server. `update` is also usable by deterministic test harnesses.
pub struct Server {
    net: RenetServer,
    transport: ServerTransport,
    root: PathBuf,
    pub lobby: Lobby,
    pub race: Option<Race>,
    content: [u8; 32],
    pending: HashMap<u64, Pending>,
    controls: [Controls; MAX_PLAYERS],
    loaded: Vec<u64>,
    time: Duration,
    loading_at: Duration,
    accumulator: f64,
    event_id: u64,
    budgets: HashMap<u64, f64>,
    /// Size of the most recent snapshot frame sent as a delta (or full, if none was).
    pub snapshot_bytes: usize,
    /// Size of the most recent full (non-delta) snapshot frame.
    pub full_snapshot_bytes: usize,
    pub overloads: u64,
    pub commands_applied: u64,
    admission_deadlines: HashMap<u64, Duration>,
    /// No hosting player owns this server; it outlives any one room owner.
    dedicated: bool,
    rejected: HashMap<u64, Duration>,
    /// Race events waiting for the next snapshot, sent together.
    events: Vec<Event>,
    history: VecDeque<(u64, Encoded)>,
    pings_sent: Duration,
}
impl Server {
    pub fn bind(
        root: &Path,
        address: SocketAddr,
        settings: RaceSettings,
        owner: Option<u64>,
    ) -> Result<Self, String> {
        let content = fingerprint(root, &settings)?;
        let socket = bind_socket(address).map_err(|e| format!("{address}: {e}"))?;
        let actual = socket.local_addr().map_err(|e| e.to_string())?;
        let transport = NetcodeServerTransport::new(
            ServerConfig {
                current_time: now(),
                max_clients: MAX_PLAYERS + 4,
                protocol_id: PROTOCOL_ID,
                public_addresses: vec![actual],
                authentication: ServerAuthentication::Unsecure,
            },
            socket,
        )
        .map_err(|e| e.to_string())?;
        Self::with_transport(
            root,
            settings,
            owner,
            content,
            ServerTransport::Udp(Box::new(transport)),
        )
    }
    /// Serve WebSockets and HTTP /health on one port, with TLS at the reverse proxy.
    pub fn bind_websocket(
        root: &Path,
        address: SocketAddr,
        settings: RaceSettings,
    ) -> Result<Self, String> {
        let content = fingerprint(root, &settings)?;
        let transport = ServerTransport::WebSocket(WsServer::bind(address)?);
        Self::with_transport(root, settings, None, content, transport)
    }
    fn with_transport(
        root: &Path,
        settings: RaceSettings,
        owner: Option<u64>,
        content: [u8; 32],
        transport: ServerTransport,
    ) -> Result<Self, String> {
        Ok(Self {
            net: RenetServer::new(connection_config()),
            transport,
            root: root.to_path_buf(),
            content,
            lobby: Lobby {
                settings,
                players: vec![],
                owner: owner.unwrap_or(0),
                epoch: 0,
                stage: Stage::Lobby,
            },
            race: None,
            pending: HashMap::new(),
            controls: std::array::from_fn(|_| Controls::default()),
            loaded: vec![],
            time: Duration::ZERO,
            loading_at: Duration::ZERO,
            accumulator: 0.,
            event_id: 0,
            budgets: HashMap::new(),
            snapshot_bytes: 0,
            full_snapshot_bytes: 0,
            overloads: 0,
            commands_applied: 0,
            admission_deadlines: HashMap::new(),
            dedicated: owner.is_none(),
            rejected: HashMap::new(),
            events: Vec::new(),
            history: VecDeque::new(),
            pings_sent: Duration::ZERO,
        })
    }
    pub fn address(&self) -> SocketAddr {
        self.transport.address()
    }
    fn send(&mut self, id: u64, channel: u8, msg: &ServerMessage) {
        if let Ok(bytes) = encode(msg) {
            if self.net.can_send_message(id, channel, bytes.len()) {
                self.net.send_message(id, channel, bytes);
            } else if channel == CONTROL {
                self.net.disconnect(id);
            }
        }
    }
    fn broadcast(&mut self, msg: ServerMessage) {
        for id in self.lobby.players.iter().map(|p| p.id).collect::<Vec<_>>() {
            self.send(id, CONTROL, &msg);
        }
    }
    /// Round-trip time to each connected player. A hosting player's own client
    /// shares the process, so its time is near zero.
    pub fn pings(&self) -> Vec<(u64, u16)> {
        self.lobby
            .players
            .iter()
            .filter(|p| p.connected && self.net.is_connected(p.id))
            .map(|p| {
                let ms = (self.net.rtt(p.id) * 1000.).round();
                (p.id, ms.clamp(0., u16::MAX as f64) as u16)
            })
            .collect()
    }
    fn announce(&mut self) {
        self.broadcast(ServerMessage::Lobby(self.lobby.clone()));
    }
    fn error(&mut self, id: u64, text: impl Into<String>) {
        self.send(id, CONTROL, &ServerMessage::Error(text.into()));
    }
    /// A hosting player's room ends with them. A dedicated server instead hands
    /// the room to the next connected driver, and resets once everyone is gone.
    fn leave(&mut self, id: u64) {
        self.pending.remove(&id);
        self.budgets.remove(&id);
        if let Some(slot) = self.lobby.players.iter().position(|p| p.id == id) {
            if id == self.lobby.owner && !self.dedicated {
                self.close("The host left. This race has ended.");
                return;
            }
            if self.lobby.stage == Stage::Lobby {
                self.lobby.players.remove(slot);
            } else if let Some(race) = &mut self.race {
                race.ai_takeover(slot);
                self.lobby.players[slot].ready = false;
                self.lobby.players[slot].connected = false;
                if self.lobby.stage == Stage::Loading {
                    self.loaded.push(id);
                }
            }
            if id == self.lobby.owner {
                self.lobby.owner = self
                    .lobby
                    .players
                    .iter()
                    .find(|p| p.connected && p.id != id)
                    .map_or(0, |p| p.id);
            }
            if self.dedicated && !self.lobby.players.iter().any(|p| p.connected) {
                self.reset_room();
            }
            self.announce();
        }
        self.net.disconnect(id);
    }
    /// Return an empty dedicated server to a fresh lobby with the same rules.
    fn reset_room(&mut self) {
        self.race = None;
        self.lobby.stage = Stage::Lobby;
        self.lobby.players.clear();
        self.lobby.owner = 0;
        self.loaded.clear();
        self.controls = std::array::from_fn(|_| Controls::default());
        self.events.clear();
        self.history.clear();
        self.accumulator = 0.;
    }
    /// Tell an unadmitted client why, then disconnect it shortly after, rather
    /// than leaving it waiting for the admission timeout.
    fn reject(&mut self, id: u64, reason: String) {
        self.pending.remove(&id);
        self.send(id, CONTROL, &ServerMessage::Closed(reason));
        self.rejected.insert(id, self.time + REJECT_GRACE);
    }
    /// How long a hosting loop can sleep before the next physics tick is due.
    pub fn idle_time(&self) -> Duration {
        if matches!(self.lobby.stage, Stage::Grid | Stage::Racing) {
            let left = 1. / PHYSICS_RATE - self.accumulator;
            Duration::from_secs_f64(left.clamp(0.000_2, 0.003))
        } else {
            Duration::from_millis(5)
        }
    }
    pub fn close(&mut self, reason: &str) {
        if self.lobby.stage == Stage::Closed {
            return;
        }
        self.lobby.stage = Stage::Closed;
        self.broadcast(ServerMessage::Closed(reason.into()));
    }
    fn handle(&mut self, id: u64, message: ClientMessage) -> Result<(), String> {
        let slot = self.lobby.players.iter().position(|p| p.id == id);
        match message {
            ClientMessage::Hello { name } => {
                if slot.is_some() {
                    self.announce();
                    return Ok(());
                }
                if self.lobby.stage != Stage::Lobby
                    || self.lobby.players.len() + self.lobby.settings.ai_cars >= MAX_PLAYERS
                {
                    return Err("Race has started or lobby is full".into());
                }
                let name = name.trim().to_owned();
                if name.is_empty() || name.len() > 32 || name.chars().any(char::is_control) {
                    return Err("Name must be 1–32 bytes without control characters".into());
                }
                let taken = |other: &str| other.eq_ignore_ascii_case(&name);
                if self.lobby.players.iter().any(|p| taken(&p.name))
                    || self
                        .pending
                        .iter()
                        .any(|(other, p)| *other != id && taken(&p.name))
                {
                    return Err("That driver name is already in use".into());
                }
                self.pending.entry(id).or_insert(Pending {
                    name,
                    expires: self.time + TIMEOUT,
                });
                self.send(
                    id,
                    CONTROL,
                    &ServerMessage::Offer {
                        settings: self.lobby.settings.clone(),
                        content: self.content,
                    },
                );
            }
            ClientMessage::Accept {
                content,
                assists,
                identity,
            } => {
                if slot.is_some() {
                    self.announce();
                    return Ok(());
                }
                if self.lobby.stage != Stage::Lobby
                    || content != self.content
                    || self.lobby.players.len() + self.lobby.settings.ai_cars >= MAX_PLAYERS
                {
                    return Err("Incompatible game/assets or lobby is full".into());
                }
                let mut settings = self.lobby.settings.clone();
                settings.assists = assists;
                settings.validate()?;
                let identity = select_identity(
                    &self.root,
                    &self.lobby.settings.car,
                    identity,
                    &self.lobby.players,
                )?;
                let pending = self.pending.remove(&id).ok_or("Join expired; reconnect")?;
                if self
                    .lobby
                    .players
                    .iter()
                    .any(|p| p.name.eq_ignore_ascii_case(&pending.name))
                {
                    return Err("That driver name is already in use".into());
                }
                if self.lobby.owner == 0 {
                    self.lobby.owner = id;
                }
                self.lobby.players.push(Player {
                    id,
                    name: pending.name,
                    ready: false,
                    connected: true,
                    identity,
                    assists,
                });
                self.announce();
            }
            _ if slot.is_none() => return Err("Join the lobby first".into()),
            ClientMessage::Ready(ready) if self.lobby.stage == Stage::Lobby => {
                self.lobby.players[slot.unwrap()].ready = ready;
                self.announce();
            }
            ClientMessage::Settings(settings)
                if id == self.lobby.owner && self.lobby.stage == Stage::Lobby =>
            {
                settings.validate()?;
                if self.lobby.players.len() + settings.ai_cars > MAX_PLAYERS {
                    return Err(
                        "The selected AI count leaves no room for every connected driver".into(),
                    );
                }
                // Different track/car means every member must validate a fresh offer.
                // The content fingerprint covers only those, so it cannot change here.
                if settings.car != self.lobby.settings.car
                    || settings.track != self.lobby.settings.track
                {
                    return Err("Leave and create a new room to change track or car".into());
                }
                self.lobby.settings = settings;
                for p in &mut self.lobby.players {
                    p.ready = false;
                }
                self.announce();
            }
            ClientMessage::Grid(ids)
                if id == self.lobby.owner && self.lobby.stage == Stage::Lobby =>
            {
                let mut proposed = ids.clone();
                proposed.sort_unstable();
                let mut actual: Vec<_> = self.lobby.players.iter().map(|p| p.id).collect();
                actual.sort_unstable();
                if proposed != actual {
                    return Err("Grid must contain every driver exactly once".into());
                }
                self.lobby.players = ids
                    .iter()
                    .map(|id| {
                        self.lobby
                            .players
                            .iter()
                            .find(|p| p.id == *id)
                            .unwrap()
                            .clone()
                    })
                    .collect();
                for player in &mut self.lobby.players {
                    player.ready = false;
                }
                self.announce();
            }
            ClientMessage::Start if id == self.lobby.owner && self.lobby.stage == Stage::Lobby => {
                if !self.lobby.players.iter().all(|p| p.ready) {
                    return Err("Every driver must be ready".into());
                }
                self.race = Some(load_race(
                    &self.root,
                    &self.lobby.settings,
                    &self.lobby.players,
                )?);
                self.lobby.epoch = self.lobby.epoch.wrapping_add(1);
                self.lobby.stage = Stage::Loading;
                self.controls = std::array::from_fn(|_| Controls::default());
                self.loaded.clear();
                self.event_id = 0;
                self.events.clear();
                self.history.clear();
                self.loading_at = self.time;
                self.accumulator = 0.;
                self.broadcast(ServerMessage::Load(self.lobby.clone()));
            }
            ClientMessage::Loaded(epoch)
                if epoch == self.lobby.epoch && self.lobby.stage == Stage::Loading =>
            {
                if !self.loaded.contains(&id) {
                    self.loaded.push(id);
                }
            }
            ClientMessage::Return
                if id == self.lobby.owner && matches!(self.lobby.stage, Stage::Results) =>
            {
                self.race = None;
                self.lobby.stage = Stage::Lobby;
                self.lobby.players.retain(|p| self.net.is_connected(p.id));
                for p in &mut self.lobby.players {
                    p.ready = false;
                }
                self.announce();
            }
            ClientMessage::Leave => self.leave(id),
            ClientMessage::Inputs {
                epoch,
                ack,
                newest,
                samples,
            } if epoch == self.lobby.epoch
                && matches!(self.lobby.stage, Stage::Grid | Stage::Racing) =>
            {
                if samples.len() > MAX_INPUT_SAMPLES {
                    return Err("Too many input samples".into());
                }
                let tick = self.race.as_ref().unwrap().tick;
                let control = &mut self.controls[slot.unwrap()];
                control.ack = control.ack.max(ack);
                for (back, input) in samples {
                    if !input.valid() {
                        return Err("Invalid input values".into());
                    }
                    let Some(at) = newest.checked_sub(back.into()) else {
                        return Err("Invalid input tick".into());
                    };
                    if at > tick && at <= tick + MAX_LEAD {
                        control.samples.entry(at).or_insert(input);
                    }
                }
            }
            ClientMessage::Command {
                epoch,
                sequence,
                tick,
                kind,
            } if epoch == self.lobby.epoch
                && matches!(self.lobby.stage, Stage::Grid | Stage::Racing) =>
            {
                let current = self.race.as_ref().unwrap().tick;
                let c = &mut self.controls[slot.unwrap()];
                if sequence <= c.received {
                    return Ok(());
                }
                if sequence != c.received + 1 || c.commands.len() >= 64 || tick > current + MAX_LEAD
                {
                    return Err("Invalid command sequence or age".into());
                }
                c.received = sequence;
                c.commands.insert(sequence, (tick.max(current + 1), kind));
            }
            _ => {
                return Err(
                    "Action is not available in this race phase, or requires the owner".into(),
                )
            }
        }
        Ok(())
    }
    pub fn update(&mut self, dt: Duration) -> Result<(), String> {
        self.time += dt;
        self.net.update(dt);
        self.transport.update(dt, &mut self.net)?;
        while self.net.get_event().is_some() {} // transport state below is the source of connection truth.
        self.budgets.retain(|id, _| self.net.is_connected(*id));
        self.admission_deadlines
            .retain(|id, _| self.net.is_connected(*id));
        self.rejected.retain(|id, _| self.net.is_connected(*id));
        for id in self.net.clients_id() {
            if let Some(deadline) = self.rejected.get(&id) {
                if self.time > *deadline {
                    self.net.disconnect(id);
                }
                continue;
            }
            if !self.lobby.players.iter().any(|p| p.id == id) {
                let deadline = self
                    .admission_deadlines
                    .entry(id)
                    .or_insert(self.time + TIMEOUT);
                if self.time > *deadline {
                    self.net.disconnect(id);
                    continue;
                }
            } else {
                self.admission_deadlines.remove(&id);
            }
            let budget = self.budgets.entry(id).or_insert(240.);
            *budget = (*budget + dt.as_secs_f64() * 240.).min(240.);
            for channel in [CONTROL, STATE] {
                for _ in 0..128 {
                    if self.rejected.contains_key(&id) {
                        break;
                    }
                    let Some(bytes) = self.net.receive_message(id, channel) else {
                        break;
                    };
                    let budget = self.budgets.get_mut(&id).unwrap();
                    if *budget < 1. {
                        self.net.disconnect(id);
                        break;
                    }
                    *budget -= 1.;
                    match decode::<ClientMessage>(&bytes) {
                        Ok(message)
                            if matches!(message, ClientMessage::Inputs { .. })
                                == (channel == STATE) =>
                        {
                            if let Err(e) = self.handle(id, message) {
                                if self.lobby.players.iter().any(|p| p.id == id) {
                                    self.error(id, e);
                                } else {
                                    self.reject(id, e);
                                    break;
                                }
                            }
                        }
                        _ => {
                            self.net.disconnect(id);
                            break;
                        }
                    }
                }
            }
        }
        let expired: Vec<_> = self
            .pending
            .iter()
            .filter(|(id, p)| p.expires < self.time || !self.net.is_connected(**id))
            .map(|(id, _)| *id)
            .collect();
        for id in expired {
            self.pending.remove(&id);
            self.net.disconnect(id);
            self.budgets.remove(&id);
        }
        let lost: Vec<_> = self
            .lobby
            .players
            .iter()
            .filter(|p| {
                !self.net.is_connected(p.id)
                    || self
                        .transport
                        .time_since_last_received_packet(p.id)
                        .is_some_and(|t| t > TIMEOUT)
            })
            .map(|p| p.id)
            .collect();
        for id in lost {
            // An abandoned car is only converted once; retain its roster slot until results.
            if self.lobby.stage == Stage::Lobby
                || self.race.as_ref().is_some_and(|r| {
                    matches!(
                        r.entrants[self.lobby.players.iter().position(|p| p.id == id).unwrap()]
                            .driver,
                        racecraft_sim::Driver::Human { .. }
                    )
                })
            {
                self.leave(id);
            }
        }
        if self.time >= self.pings_sent + PING_INTERVAL && !self.lobby.players.is_empty() {
            self.pings_sent = self.time;
            let pings = self.pings();
            self.broadcast(ServerMessage::Pings(pings));
        }
        if self.lobby.stage == Stage::Loading {
            let all = self
                .lobby
                .players
                .iter()
                .all(|p| self.loaded.contains(&p.id));
            if all {
                self.lobby.stage = Stage::Grid;
                self.accumulator = 0.;
                self.announce();
                self.snapshot()?;
            } else if self.time - self.loading_at > Duration::from_secs(30) {
                self.close("A driver did not finish loading in 30 seconds. Create a new room.");
            }
        }
        if matches!(self.lobby.stage, Stage::Grid | Stage::Racing) {
            self.accumulator += dt.as_secs_f64();
            // Bounded work per poll. Retain ordinary jitter debt; abort sustained overload.
            if self.accumulator > 0.5 {
                self.overloads += 1;
                self.close("Host cannot keep up with the race. Session ended.");
            }
            for _ in 0..36 {
                if self.accumulator < 1. / PHYSICS_RATE
                    || matches!(self.lobby.stage, Stage::Closed | Stage::Results)
                {
                    break;
                }
                self.accumulator -= 1. / PHYSICS_RATE;
                self.step()?;
            }
        }
        self.transport.send_packets(&mut self.net);
        Ok(())
    }
    fn step(&mut self) -> Result<(), String> {
        let race = self.race.as_mut().unwrap();
        let tick = race.tick + 1;
        let mut inputs = [CarInput::default(); MAX_PLAYERS];
        for (slot, c) in self.controls.iter_mut().enumerate() {
            // Samples are only ever for future ticks; drop the ones already simulated.
            c.samples = c.samples.split_off(&tick);
            if let Some(input) = c.samples.remove(&tick) {
                c.input = input;
                c.last_tick = tick;
            }
            if tick.saturating_sub(c.last_tick) > 90 {
                c.input = PackedInput::pack(&CarInput {
                    brake: 0.3,
                    ..CarInput::default()
                });
            }
            inputs[slot] = c.input.unpack();
            let next = c
                .commands
                .iter()
                .next()
                .filter(|(_, (scheduled, _))| *scheduled <= tick)
                .map(|(sequence, (_, kind))| (*sequence, *kind));
            // Preserve command order even if a peer supplies decreasing target ticks.
            // Consume only one edge per tick, with monotonic authoritative acknowledgements.
            if let Some((seq, kind)) = next {
                if slot < race.entrants.len() {
                    match kind {
                        Command::ShiftUp => inputs[slot].shift_up = true,
                        Command::ShiftDown => inputs[slot].shift_down = true,
                        Command::Harvest => inputs[slot].harvest = true,
                        Command::Recover => race.recover(slot),
                    }
                }
                self.commands_applied += 1;
                c.applied = seq;
                c.commands.remove(&seq);
            }
        }
        race.step(&inputs);
        for event in race.events.drain(..) {
            // Sustained contact reports every tick; keep one, the hardest, per pair per batch.
            if let RaceEvent::Contact { a, b, impulse } = event {
                let kept = self.events.iter_mut().find_map(|e| match &mut e.event {
                    RaceEvent::Contact {
                        a: ka,
                        b: kb,
                        impulse: ki,
                    } if *ka == a && *kb == b => Some(ki),
                    _ => None,
                });
                if let Some(kept) = kept {
                    *kept = kept.max(impulse);
                    continue;
                }
            }
            self.event_id += 1;
            self.events.push(Event {
                id: self.event_id,
                event,
            });
        }
        let phase = self.race.as_ref().unwrap().phase;
        let stage = if phase == Phase::Finished {
            Stage::Results
        } else if matches!(phase, Phase::Countdown { .. }) {
            Stage::Grid
        } else {
            Stage::Racing
        };
        if self.lobby.stage != stage {
            self.lobby.stage = stage;
            self.announce();
        }
        if tick.is_multiple_of(SNAPSHOT_INTERVAL) || stage == Stage::Results {
            self.snapshot()?;
        }
        Ok(())
    }
    fn snapshot(&mut self) -> Result<(), String> {
        if !self.events.is_empty() {
            let events = std::mem::take(&mut self.events);
            self.broadcast(ServerMessage::Events {
                epoch: self.lobby.epoch,
                events,
            });
        }
        let Some(race) = &mut self.race else {
            return Ok(());
        };
        let state = State {
            epoch: self.lobby.epoch,
            race: race.snapshot(),
            inputs: std::array::from_fn(|i| self.controls[i].input),
            commands: std::array::from_fn(|i| self.controls[i].applied),
        };
        let data = codec::encode(&state)?;
        // Continue from exactly the rounded state every client restores, so a
        // client that predicted the same inputs arrives at bit-identical state.
        let rounded: State = codec::decode(&data)?;
        race.restore(&rounded.race)?;
        let tick = rounded.race.tick;
        let reliable = self.lobby.stage == Stage::Results;
        let channel = if reliable { CONTROL } else { STATE };
        let mut frames: HashMap<Option<u64>, renet::Bytes> = HashMap::new();
        let frame = |base: Option<u64>, history: &VecDeque<(u64, Encoded)>| {
            let data = match base.and_then(|b| history.iter().find(|(t, _)| *t == b)) {
                Some((_, old)) => data.xor(old),
                None => data.clone(),
            };
            encode(&ServerMessage::State(Box::new(Frame {
                epoch: self.lobby.epoch,
                tick,
                base,
                data,
            })))
            .map(renet::Bytes::from)
        };
        let full = frame(None, &self.history)?;
        self.full_snapshot_bytes = full.len();
        self.snapshot_bytes = full.len();
        frames.insert(None, full);
        for (slot, player) in self.lobby.players.iter().enumerate() {
            if !player.connected {
                continue;
            }
            // The reliable final snapshot is always complete; otherwise use the
            // client's acknowledged snapshot when it is still in history.
            let ack = self.controls[slot].ack;
            let base = (!reliable && self.history.iter().any(|(t, _)| *t == ack)).then_some(ack);
            let payload = match frames.get(&base) {
                Some(payload) => payload.clone(),
                None => {
                    let payload = frame(base, &self.history)?;
                    self.snapshot_bytes = payload.len();
                    frames.insert(base, payload.clone());
                    payload
                }
            };
            if self.net.can_send_message(player.id, channel, payload.len()) {
                self.net.send_message(player.id, channel, payload);
            } else if channel == CONTROL {
                self.net.disconnect(player.id);
            }
        }
        if self.history.len() >= HISTORY {
            self.history.pop_front();
        }
        self.history.push_back((tick, data));
        Ok(())
    }
}
/// Bind UDP. An unspecified IPv6 address (`[::]`) also accepts IPv4 where the OS allows it.
fn bind_socket(address: SocketAddr) -> std::io::Result<UdpSocket> {
    if address.is_ipv6() && address.ip().is_unspecified() {
        use socket2::{Domain, Protocol, Socket, Type};
        let socket = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?;
        socket.set_only_v6(false)?;
        socket.bind(&address.into())?;
        return Ok(socket.into());
    }
    UdpSocket::bind(address)
}
/// Dropping a hosting game stops and joins its worker; no detached server survives leaving.
pub struct ServerHandle {
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    pub address: SocketAddr,
}
impl ServerHandle {
    pub fn spawn(mut server: Server) -> Self {
        let address = server.address();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let worker = thread::spawn(move || {
            let mut previous = Instant::now();
            while !flag.load(Ordering::Relaxed) && server.lobby.stage != Stage::Closed {
                let now = Instant::now();
                let dt = now.duration_since(previous);
                previous = now;
                if let Err(error) = server.update(dt) {
                    eprintln!("Race server: {error}");
                    break;
                }
                thread::sleep(server.idle_time());
            }
            server.close("The host closed the room.");
            server.transport.send_packets(&mut server.net);
            server.transport.disconnect_all(&mut server.net);
        });
        Self {
            stop,
            worker: Some(worker),
            address,
        }
    }
}
impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
