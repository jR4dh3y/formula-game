use crate::link::LinkHandle;
use crate::transport::ClientTransport;
use crate::{codec::Encoded, *};
use racecraft_sim::{Assists, CarInput, Race, PHYSICS_RATE};
use renet::RenetClient;
use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    time::Duration,
};

/// Optional instrumentation for rendered multiplayer test sessions.
#[derive(Clone, Debug, serde::Serialize)]
pub struct AuthorityObservation {
    pub tick: u64,
    pub slot: usize,
    pub phase: String,
    pub hash: String,
    pub positions: Vec<[f64; 3]>,
    pub speeds: Vec<f64>,
    pub distances: Vec<f64>,
}
pub struct Client {
    /// Socket and reliability layer, serviced on the network thread.
    link: LinkHandle,
    root: PathBuf,
    name: String,
    assists: Assists,
    pub id: u64,
    pub identity: Option<DriverIdentity>,
    pub lobby: Option<Lobby>,
    pub error: Option<String>,
    pub closed: bool,
    pub load: Option<Lobby>,
    pub events: VecDeque<Event>,
    /// Each connected driver's round-trip time to the server in ms, by player id.
    pub pings: HashMap<u64, u16>,
    hello: bool,
    elapsed: Duration,
    sent: Duration,
    pending: VecDeque<InputSample>,
    commands: VecDeque<(u64, u64, Command)>,
    sequence: u64,
    /// Remote controls from the newest authoritative state, used for prediction.
    inputs: [PackedInput; MAX_PLAYERS],
    latest: Option<Box<State>>,
    /// Decoded snapshots kept as delta baselines, oldest first.
    received: VecDeque<(u64, Encoded)>,
    /// Newest snapshot tick held; reported to the server as the delta baseline.
    ack: u64,
    /// Encoded predicted race state per tick, to detect an exact prediction.
    history: VecDeque<(u64, Encoded)>,
    /// Ticks the prediction is ahead of where it needs to be; bled off gradually.
    excess: u64,
    steps: u64,
    /// Snapshots that needed a rollback and replay, and those that matched the prediction.
    pub resims: u64,
    pub resims_skipped: u64,
    pub authoritative_tick: Option<u64>,
    harvest_pending: bool,
    epoch: u64,
    pub last_correction: f64,
    pub observe: bool,
    pub observation: Option<AuthorityObservation>,
}
impl Client {
    /// Connect without blocking the render thread, including DNS and TLS for WSS.
    pub fn connect(
        root: &Path,
        endpoint: Endpoint,
        name: String,
        assists: Assists,
        id: Option<u64>,
    ) -> Result<Self, String> {
        let id = id.unwrap_or_else(random_id);
        let transport = ClientTransport::connect(endpoint, id)?;
        Ok(Self {
            link: LinkHandle::spawn(RenetClient::new(connection_config()), transport)?,
            root: root.to_path_buf(),
            name,
            assists,
            id,
            identity: None,
            lobby: None,
            error: None,
            closed: false,
            load: None,
            events: VecDeque::new(),
            pings: HashMap::new(),
            hello: false,
            elapsed: Duration::ZERO,
            sent: Duration::ZERO,
            pending: VecDeque::new(),
            commands: VecDeque::new(),
            sequence: 0,
            inputs: [PackedInput::default(); MAX_PLAYERS],
            latest: None,
            received: VecDeque::new(),
            ack: 0,
            history: VecDeque::new(),
            excess: 0,
            steps: 0,
            resims: 0,
            resims_skipped: 0,
            authoritative_tick: None,
            harvest_pending: false,
            epoch: 0,
            last_correction: 0.,
            observe: false,
            observation: None,
        })
    }
    pub fn send(&mut self, message: ClientMessage) -> Result<(), String> {
        let channel = if matches!(message, ClientMessage::Inputs { .. }) {
            STATE
        } else {
            CONTROL
        };
        let bytes = encode(&message)?;
        let mut link = self.link.lock();
        if !link.net.can_send_message(channel, bytes.len()) {
            return Err("Network send queue is full".into());
        }
        link.net.send_message(channel, bytes);
        Ok(())
    }
    pub fn slot(&self) -> Option<usize> {
        self.lobby
            .as_ref()?
            .players
            .iter()
            .position(|p| p.id == self.id)
    }
    pub fn stage(&self) -> Stage {
        self.lobby.as_ref().map_or(Stage::Lobby, |l| l.stage)
    }
    pub fn owner(&self) -> bool {
        self.lobby.as_ref().is_some_and(|l| l.owner == self.id)
    }
    pub fn active(&self) -> bool {
        !self.closed && matches!(self.stage(), Stage::Grid | Stage::Racing | Stage::Results)
    }
    pub fn loaded(&mut self) -> Result<(), String> {
        self.send(ClientMessage::Loaded(self.epoch))
    }
    pub fn poll(&mut self, dt: Duration) -> Result<(), String> {
        if self.closed {
            return Ok(());
        }
        self.elapsed += dt;
        // The network thread keeps the connection serviced between frames;
        // catch it up now too, then take what has arrived.
        let (connected, inbox) = {
            let mut link = self.link.lock();
            // Read only here; `poll` ends with a flush.
            link.service();
            if let Some(e) = link.error.clone() {
                drop(link);
                self.closed = true;
                return Err(format!("Connection ended: {e}"));
            }
            let mut inbox = Vec::new();
            for channel in [CONTROL, STATE] {
                for _ in 0..256 {
                    let Some(bytes) = link.net.receive_message(channel) else {
                        break;
                    };
                    inbox.push(bytes);
                }
            }
            (link.net.is_connected(), inbox)
        };
        if connected && !self.hello {
            self.send(ClientMessage::Hello {
                name: self.name.clone(),
            })?;
            self.hello = true;
        }
        {
            for bytes in inbox {
                match decode::<ServerMessage>(&bytes)? {
                    ServerMessage::Offer { settings, content } => {
                        if fingerprint(&self.root, &settings)? != content {
                            self.closed = true;
                            self.link.lock().transport.disconnect();
                            return Err("Game version or converted track/car assets differ from the host. Install the same build and assets.".into());
                        }
                        self.send(ClientMessage::Accept {
                            content,
                            assists: self.assists,
                            identity: self.identity.clone(),
                        })?;
                    }
                    ServerMessage::Lobby(lobby) => {
                        if lobby.players.len() > MAX_PLAYERS {
                            return Err("Invalid lobby size".into());
                        }
                        self.lobby = Some(lobby);
                    }
                    ServerMessage::Load(lobby) => {
                        if lobby.epoch <= self.epoch {
                            continue;
                        }
                        self.epoch = lobby.epoch;
                        self.authoritative_tick = None;
                        self.harvest_pending = false;
                        self.latest = None;
                        self.pending.clear();
                        self.commands.clear();
                        self.events.clear();
                        self.sequence = 0;
                        self.inputs = [PackedInput::default(); MAX_PLAYERS];
                        self.received.clear();
                        self.history.clear();
                        self.ack = 0;
                        self.excess = 0;
                        self.load = Some(lobby.clone());
                        self.lobby = Some(lobby);
                    }
                    ServerMessage::State(frame) => {
                        // Newer snapshots supersede older ones; a delta whose
                        // baseline we no longer hold is dropped, and our older
                        // ack makes the server fall back to a full snapshot.
                        if frame.epoch != self.epoch
                            || self.received.back().is_some_and(|(t, _)| frame.tick <= *t)
                        {
                            continue;
                        }
                        let data = match frame.base {
                            None => frame.data,
                            Some(base) => match self.received.iter().find(|(t, _)| *t == base) {
                                Some((_, old)) => frame.data.xor(old),
                                None => continue,
                            },
                        };
                        let state: State = codec::decode(&data)?;
                        if state.epoch != frame.epoch || state.race.tick != frame.tick {
                            return Err("Inconsistent snapshot from host".into());
                        }
                        if self.received.len() >= 64 {
                            self.received.pop_front();
                        }
                        self.received.push_back((frame.tick, data));
                        self.ack = frame.tick;
                        self.latest = Some(Box::new(state));
                    }
                    ServerMessage::Events { epoch, events } if epoch == self.epoch => {
                        for event in events {
                            if self.events.len() >= 1024 {
                                self.events.pop_front();
                            }
                            self.events.push_back(event);
                        }
                    }
                    ServerMessage::Pings(pings) => self.pings = pings.into_iter().collect(),
                    ServerMessage::Error(error) => self.error = Some(error),
                    ServerMessage::Closed(reason) => {
                        self.error = Some(reason);
                        self.closed = true;
                    }
                    _ => {}
                }
            }
        }
        if self.lobby.is_none() && self.elapsed > TIMEOUT {
            self.closed = true;
            return Err(
                "Could not reach the room. Check the address and that the host is running.".into(),
            );
        }
        if self.link.lock().transport.time_since_last_received_packet() > TIMEOUT {
            self.closed = true;
            return Err("Host is not responding. The race has ended.".into());
        }
        if matches!(self.stage(), Stage::Grid | Stage::Racing)
            && self.elapsed - self.sent >= Duration::from_secs_f64(1. / INPUT_RATE as f64)
        {
            self.sent = self.elapsed;
            let newest = self.pending.back().map_or(0, |s| s.tick);
            let samples = self
                .pending
                .iter()
                .rev()
                .take(MAX_INPUT_SAMPLES)
                .filter_map(|s| Some((u16::try_from(newest - s.tick).ok()?, s.input)))
                .collect();
            self.send(ClientMessage::Inputs {
                epoch: self.epoch,
                ack: self.ack,
                newest,
                samples,
            })?;
        }
        self.flush()
    }
    /// Apply the newest authoritative state.
    ///
    /// When the prediction already reached exactly that state with the same
    /// remote controls, nothing changes and this returns `Ok(false)`. Otherwise
    /// the race rolls back to the snapshot, replays pending inputs up to the
    /// prediction lead and returns `Ok(true)` so the renderer can blend.
    pub fn reconcile(&mut self, race: &mut Race) -> Result<bool, String> {
        let Some(state) = self.latest.take() else {
            return Ok(false);
        };
        let slot = self.slot().ok_or("Host did not assign a driver slot")?;
        let tick = state.race.tick;
        let natural = tick + prediction_lead(self.link.lock().net.rtt());
        self.authoritative_tick = Some(tick);
        self.pending.retain(|s| s.tick > tick);
        self.commands
            .retain(|(seq, _, _)| *seq > state.commands[slot]);
        let authoritative = codec::encode(&state.race)?;
        // Exact when the remote cars were predicted with these controls, every
        // own tick since was a real sample, and the stored state matches.
        let remote_unchanged =
            (0..MAX_PLAYERS).all(|i| i == slot || self.inputs[i] == state.inputs[i]);
        let own_covered = race.tick >= tick
            && self.pending.iter().filter(|s| s.tick <= race.tick).count() as u64
                == race.tick - tick;
        let predicted = self
            .history
            .iter()
            .find(|(t, _)| *t == tick)
            .is_some_and(|(_, predicted)| *predicted == authoritative);
        self.inputs = state.inputs;
        if !self.observe && remote_unchanged && own_covered && predicted {
            self.history.retain(|(t, _)| *t >= tick);
            self.excess = lead_plan(race.tick, tick, natural).1;
            self.last_correction = 0.;
            self.resims_skipped += 1;
            return Ok(false);
        }
        let (target, excess) = lead_plan(race.tick, tick, natural);
        let previous = race.entrants[slot].car.position;
        race.restore(&state.race)?;
        if self.observe && tick.is_multiple_of(120) {
            use sha2::{Digest, Sha256};
            let hash = Sha256::digest(encode(&race.snapshot())?)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            self.observation = Some(AuthorityObservation {
                tick,
                slot,
                phase: format!("{:?}", race.phase),
                hash,
                positions: race
                    .entrants
                    .iter()
                    .map(|e| e.car.position.to_array())
                    .collect(),
                speeds: race.entrants.iter().map(|e| e.car.speed()).collect(),
                distances: race.entrants.iter().map(|e| e.timing.distance).collect(),
            });
        }
        self.history.clear();
        self.history.push_back((tick, authoritative));
        let remote = state.inputs.map(|p| p.unpack());
        let predicted_to = self.pending.back().map_or(tick, |s| s.tick);
        let latest = self.pending.back().map_or(state.inputs[slot], |s| s.input);
        let events = std::mem::take(&mut race.events);
        while race.tick < target && self.stage() != Stage::Results {
            let next = race.tick + 1;
            let mut inputs = remote;
            if let Ok(i) = self.pending.binary_search_by_key(&next, |s| s.tick) {
                inputs[slot] = self.pending[i].input.unpack();
            } else if next > predicted_to {
                // Catching up past anything predicted before: hold the newest
                // own input and send it, so the server simulates the same.
                inputs[slot] = latest.unpack();
                self.pending.push_back(InputSample {
                    tick: next,
                    input: latest,
                });
            }
            for (_, t, cmd) in &self.commands {
                if *t == next {
                    apply_command(race, slot, &mut inputs[slot], *cmd);
                }
            }
            race.step(&inputs);
            race.events.clear();
            self.record(race)?;
        }
        race.events = events;
        self.excess = excess;
        self.last_correction = previous.distance(race.entrants[slot].car.position);
        self.resims += 1;
        Ok(true)
    }
    /// On snapshot ticks, round like the server does and keep the result for
    /// comparing with the authoritative snapshot when it arrives.
    fn record(&mut self, race: &mut Race) -> Result<(), String> {
        if !race.tick.is_multiple_of(SNAPSHOT_INTERVAL) {
            return Ok(());
        }
        if self.history.len() >= HISTORY {
            self.history.pop_front();
        }
        let encoded = quantize(race)?;
        self.history.push_back((race.tick, encoded));
        Ok(())
    }
    /// Physics input targets one explicit server tick. Commands travel reliably; held controls use redundant samples.
    pub fn predict(&mut self, race: &mut Race, mut input: CarInput) -> Result<(), String> {
        let Some(authoritative) = self.authoritative_tick else {
            return Ok(());
        };
        if !self.active() || self.stage() == Stage::Results {
            return Ok(());
        }
        self.harvest_pending |= input.harvest;
        if race.tick >= authoritative + MAX_LEAD {
            return Ok(());
        }
        let slot = self.slot().ok_or("Missing player slot")?;
        let tick = race.tick + 1;
        if input.shift_up {
            self.command_at(tick, Command::ShiftUp)?;
        }
        if input.shift_down {
            self.command_at(tick, Command::ShiftDown)?;
        }
        if self.harvest_pending {
            self.command_at(tick, Command::Harvest)?;
            self.harvest_pending = false;
        }
        // Hold one tick now and then (about 6%) while ahead of the needed lead.
        // Any command just queued for this tick is applied on the next call.
        if self.excess > 0 {
            self.steps += 1;
            if self.steps.is_multiple_of(THROTTLE_EVERY) {
                self.excess -= 1;
                return Ok(());
            }
        }
        // Predict with exactly the resolution the server will simulate.
        let packed = PackedInput::pack(&input);
        input = packed.unpack();
        let mut inputs = self.inputs.map(|p| p.unpack());
        inputs[slot] = input;
        for (_, t, cmd) in &self.commands {
            if *t == tick {
                apply_command(race, slot, &mut inputs[slot], *cmd);
            }
        }
        self.pending.push_back(InputSample {
            tick,
            input: packed,
        });
        while self.pending.len() > MAX_LEAD as usize {
            self.pending.pop_front();
        }
        let events = std::mem::take(&mut race.events);
        race.step(&inputs);
        race.events = events;
        self.record(race)
    }
    fn command_at(&mut self, tick: u64, kind: Command) -> Result<(), String> {
        if self.commands.len() >= 64 {
            return Err("Too many pending commands".into());
        }
        self.sequence += 1;
        self.commands.push_back((self.sequence, tick, kind));
        self.send(ClientMessage::Command {
            epoch: self.epoch,
            sequence: self.sequence,
            tick,
            kind,
        })
    }
    pub fn recover(&mut self, tick: u64) -> Result<(), String> {
        if !matches!(self.stage(), Stage::Grid | Stage::Racing) {
            return Ok(());
        }
        self.command_at(tick, Command::Recover)
    }
    /// Send queued messages now rather than on the network thread's next pass.
    pub fn flush(&mut self) -> Result<(), String> {
        self.link.lock().send()
    }
    pub fn rtt_ms(&self) -> f64 {
        self.link.lock().net.rtt() * 1000.
    }
    /// Latest round-trip time for a connected driver, as the server measures it,
    /// so every driver sees the same figure. Before the first room broadcast this
    /// client falls back to its own measurement.
    pub fn ping_ms(&self, player: u64) -> Option<u16> {
        self.pings.get(&player).copied().or_else(|| {
            (player == self.id && self.lobby.is_some())
                .then(|| self.rtt_ms().round().clamp(0., u16::MAX as f64) as u16)
        })
    }
}
/// Predicted snapshot-tick states kept for exact-match checks; covers twice the maximum lead.
const HISTORY: usize = (2 * MAX_LEAD / SNAPSHOT_INTERVAL) as usize + 1;
/// Lead above the latency-based target tolerated before bleeding it off,
/// so ordinary round-trip jitter does not cause skipped ticks.
const LEAD_SLACK: u64 = 2;
/// While ahead, skip one predicted tick in this many (about 22 ticks per second).
const THROTTLE_EVERY: u64 = 16;
/// Where to replay a rollback to, and how many surplus ticks to bleed off.
///
/// The local timeline never steps backwards (that would re-send inputs for
/// ticks the server already holds). A lead above the latency-based target is
/// instead removed gradually by `predict`, so a brief latency spike cannot
/// leave the prediction permanently further ahead than it needs to be.
fn lead_plan(local: u64, authoritative: u64, natural: u64) -> (u64, u64) {
    let target = local.max(natural).min(authoritative + MAX_LEAD);
    (target, target.saturating_sub(natural + LEAD_SLACK))
}
/// Keep enough predicted time for one-way transport delay and the next 60 Hz input send.
/// The fixed 33 ms baseline remains useful on a local connection.
fn prediction_lead(rtt_seconds: f64) -> u64 {
    let network_ticks = (rtt_seconds.max(0.) * PHYSICS_RATE / 2.).ceil() as u64;
    (INPUT_LEAD + network_ticks).min(MAX_LEAD)
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.send(ClientMessage::Leave);
        let _ = self.flush();
        self.link.lock().transport.disconnect();
    }
}

fn apply_command(race: &mut Race, slot: usize, input: &mut CarInput, command: Command) {
    match command {
        Command::Recover => race.recover(slot),
        Command::ShiftUp => input.shift_up = true,
        Command::ShiftDown => input.shift_down = true,
        Command::Harvest => input.harvest = true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surplus_lead_is_bled_off_instead_of_kept() {
        // A 150 ms spike left the prediction 39 ticks ahead; latency is now ~13 ticks.
        let natural = 1_000 + prediction_lead(0.005);
        let (target, excess) = lead_plan(1_039, 1_000, natural);
        assert_eq!(target, 1_039, "the local timeline never steps backwards");
        assert_eq!(excess, 1_039 - natural - LEAD_SLACK);
        // Behind the needed lead: replay forward, nothing to bleed.
        assert_eq!(lead_plan(1_005, 1_000, natural), (natural, 0));
        // Within jitter slack: leave it alone.
        assert_eq!(lead_plan(natural + LEAD_SLACK, 1_000, natural).1, 0);
        // Never further ahead than the hard prediction limit.
        assert_eq!(lead_plan(5_000, 1_000, natural).0, 1_000 + MAX_LEAD);
    }

    #[test]
    fn prediction_lead_grows_with_latency_but_stays_bounded() {
        assert_eq!(prediction_lead(0.), INPUT_LEAD);
        assert!(prediction_lead(0.100) > INPUT_LEAD);
        assert_eq!(prediction_lead(10.), MAX_LEAD);
    }
}
