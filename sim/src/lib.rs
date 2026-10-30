//! Deterministic racecraft simulation shared by the game client and, later,
//! a headless netcode server.
//!
//! Easy to drive, hard to race well: the car model is a stable simcade
//! approximation (shared grip budget, tyre temperature and wear, wing setup,
//! slipstream and dirty air) and the AI races with intent rather than
//! following a line.

pub mod aero;
pub mod ai;
pub mod car;
pub mod drs;
pub mod race;
pub mod rng;
pub mod track;

pub use ai::{AiDriver, DriverProfile, Intent};
pub use car::spec::{CarGeometry, CarSpec, Compound, Setup, WheelSpec};
pub use car::{AeroInfluence, Assists, Car, CarInput};
pub use race::{Driver, Entrant, EntrantConfig, Phase, Race, RaceEvent, Timing, PHYSICS_RATE};
pub use track::{Surface, Track, TrackPosition};
