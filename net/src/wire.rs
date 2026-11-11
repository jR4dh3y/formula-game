use crate::{RaceSettings, MAX_PLAYERS};
use bincode::Options;
use racecraft_sim::{race::Snapshot, Assists, CarInput, RaceEvent};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::time::Duration;
pub const CONTROL: u8 = 0;
pub const STATE: u8 = 1;
pub const MAX_MESSAGE: usize = 64 * 1024;
pub const TIMEOUT: Duration = Duration::from_secs(8);
pub const INPUT_LEAD: u64 = 12; // 33 ms, six physics steps per input-send interval.
pub const MAX_LEAD: u64 = 90;
/// Held-input samples per packet: 100 ms of redundancy at 360 Hz.
pub const MAX_INPUT_SAMPLES: usize = 36;
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DriverIdentity {
    pub team: String,
    pub driver: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Player {
    pub id: u64,
    pub name: String,
    pub ready: bool,
    pub connected: bool,
    pub identity: Option<DriverIdentity>,
    pub assists: Assists,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stage {
    Lobby,
    Loading,
    Grid,
    Racing,
    Results,
    Closed,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Lobby {
    pub settings: RaceSettings,
    pub players: Vec<Player>,
    pub owner: u64,
    pub epoch: u64,
    pub stage: Stage,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ClientMessage {
    Hello {
        name: String,
    },
    Accept {
        content: [u8; 32],
        assists: Assists,
        identity: Option<DriverIdentity>,
    },
    Ready(bool),
    Settings(RaceSettings),
    Grid(Vec<u64>),
    Start,
    Loaded(u64),
    Return,
    Leave,
    Command {
        epoch: u64,
        sequence: u64,
        tick: u64,
        kind: Command,
    },
    Inputs {
        epoch: u64,
        /// Newest snapshot tick this client holds; the server may delta against it.
        ack: u64,
        /// Tick of the newest sample; each sample is stored as `newest - back`.
        newest: u64,
        samples: Vec<(u16, PackedInput)>,
    },
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum Command {
    ShiftUp,
    ShiftDown,
    Recover,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct InputSample {
    pub tick: u64,
    pub input: PackedInput,
}
/// Held controls in the exact resolution the server simulates.
///
/// Clients predict with `pack(input).unpack()`, so prediction and authority use
/// bit-identical values. Shifts never travel here; they are reliable commands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackedInput {
    pub steer: i16,
    pub throttle: u16,
    pub brake: u16,
    pub flags: u8,
}
const DRS_FLAG: u8 = 1;
const HARVEST_FLAG: u8 = 2;
const HANDBRAKE_FLAG: u8 = 4;
impl PackedInput {
    pub fn pack(input: &CarInput) -> Self {
        let unit = |x: f64| (x.clamp(0., 1.) * u16::MAX as f64).round() as u16;
        Self {
            steer: (input.steer.clamp(-1., 1.) * i16::MAX as f64).round() as i16,
            throttle: unit(input.throttle),
            brake: unit(input.brake),
            flags: if input.drs { DRS_FLAG } else { 0 }
                | if input.harvest { HARVEST_FLAG } else { 0 }
                | if input.handbrake { HANDBRAKE_FLAG } else { 0 },
        }
    }
    pub fn unpack(&self) -> CarInput {
        CarInput {
            steer: self.steer as f64 / i16::MAX as f64,
            throttle: self.throttle as f64 / u16::MAX as f64,
            brake: self.brake as f64 / u16::MAX as f64,
            drs: self.flags & DRS_FLAG != 0,
            harvest: self.flags & HARVEST_FLAG != 0,
            handbrake: self.flags & HANDBRAKE_FLAG != 0,
            ..CarInput::default()
        }
    }
    pub fn valid(&self) -> bool {
        self.steer != i16::MIN && self.flags & !(DRS_FLAG | HARVEST_FLAG | HANDBRAKE_FLAG) == 0
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ServerMessage {
    Offer {
        settings: RaceSettings,
        content: [u8; 32],
    },
    Lobby(Lobby),
    Load(Lobby),
    State(Box<Frame>),
    Events {
        epoch: u64,
        events: Vec<Event>,
    },
    Error(String),
    Closed(String),
    /// Round-trip time per connected player id, in milliseconds, as the server measures it.
    Pings(Vec<(u64, u16)>),
}
/// Authoritative state, carried in the compact [`crate::codec`] format.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct State {
    pub epoch: u64,
    pub race: Snapshot,
    pub inputs: [PackedInput; MAX_PLAYERS],
    pub commands: [u64; MAX_PLAYERS],
}
/// One snapshot on the wire: the full encoding, or an XOR delta against a
/// snapshot the client acknowledged (`base`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Frame {
    pub epoch: u64,
    pub tick: u64,
    pub base: Option<u64>,
    pub data: crate::codec::Encoded,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub id: u64,
    pub event: RaceEvent,
}
pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    let bytes = bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit((MAX_MESSAGE - 1) as u64)
        .serialize(value)
        .map_err(|e| e.to_string())?;
    if bytes.len() >= MAX_MESSAGE {
        return Err("network message exceeds limit".into());
    }
    if bytes.len() > 1000 {
        let compressed = miniz_oxide::deflate::compress_to_vec_zlib(&bytes, 6);
        if compressed.len() + 5 < bytes.len() + 1 {
            let mut frame = vec![1];
            frame.extend((compressed.len() as u32).to_le_bytes());
            frame.extend(compressed);
            return Ok(frame);
        }
    }
    let mut frame = vec![0];
    frame.extend(bytes);
    Ok(frame)
}
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, String> {
    if bytes.is_empty() || bytes.len() > MAX_MESSAGE {
        return Err("invalid network message size".into());
    }
    let plain;
    let payload = match bytes[0] {
        0 => &bytes[1..],
        1 if bytes.len() >= 5 => {
            let len = u32::from_le_bytes(bytes[1..5].try_into().unwrap()) as usize;
            if len != bytes.len() - 5 {
                return Err("invalid compressed frame size".into());
            }
            plain = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(
                &bytes[5..],
                MAX_MESSAGE - 1,
            )
            .map_err(|_| "invalid compressed frame")?;
            &plain
        }
        _ => return Err("invalid network frame".into()),
    };
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(MAX_MESSAGE as u64)
        .reject_trailing_bytes()
        .deserialize(payload)
        .map_err(|e| e.to_string())
}
pub fn connection_config() -> renet::ConnectionConfig {
    use renet::{ChannelConfig, SendType};
    let channels = vec![
        ChannelConfig {
            channel_id: CONTROL,
            max_memory_usage_bytes: 256 * 1024,
            send_type: SendType::ReliableOrdered {
                resend_time: Duration::from_millis(100),
            },
        },
        ChannelConfig {
            channel_id: STATE,
            // Renet retains incomplete slices for three seconds. Budget for 90
            // worst-size snapshots so lost fragments cannot starve fresh ones.
            max_memory_usage_bytes: 8 * 1024 * 1024,
            send_type: SendType::Unreliable,
        },
    ];
    renet::ConnectionConfig {
        available_bytes_per_tick: 128 * 1024,
        server_channels_config: channels.clone(),
        client_channels_config: channels,
    }
}
pub fn now() -> Duration {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
}
pub fn random_id() -> u64 {
    let bytes = renet_netcode::generate_random_bytes::<8>();
    u64::from_le_bytes(bytes)
}
pub fn valid_input(input: &CarInput) -> bool {
    input.steer.is_finite()
        && (-1. ..=1.).contains(&input.steer)
        && [input.throttle, input.brake]
            .iter()
            .all(|x| x.is_finite() && (0. ..=1.).contains(x))
        && !input.shift_up
        && !input.shift_down
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_buttons_round_trip_and_unknown_flags_are_invalid() {
        for (drs, harvest) in [(false, false), (true, false), (false, true), (true, true)] {
            let input = CarInput {
                drs,
                harvest,
                ..CarInput::default()
            };
            let packed = PackedInput::pack(&input);
            assert!(packed.valid());
            let back = packed.unpack();
            assert_eq!((back.drs, back.harvest), (drs, harvest));
        }
        let unknown = PackedInput {
            flags: 8,
            ..PackedInput::default()
        };
        assert!(!unknown.valid());
    }
}
