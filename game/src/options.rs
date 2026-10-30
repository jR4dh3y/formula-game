//! Command-line options.

use crate::camera::Mode;
use racecraft_sim::{Assists, Compound, Setup};
use std::path::PathBuf;

#[derive(Clone, bevy::prelude::Resource)]
pub struct Options {
    pub assets: PathBuf,
    pub track: String,
    pub car: String,
    pub laps: u32,
    pub opponents: usize,
    pub grid_slot: usize,
    pub setup: Setup,
    pub automatic_gears: bool,
    pub assist_level: f64,
    pub width: u32,
    pub height: u32,
    pub frames: Option<u64>,
    pub capture: Option<PathBuf>,
    pub benchmark: Option<PathBuf>,
    pub benchmark_seed: u64,
    pub benchmark_start: f64,
    pub mute: bool,
    pub quick_race: bool,
    pub camera: Mode,
    pub driver_aids: Option<Assists>,
    pub corner_braking: f64,
    pub aids_explicit: bool,
    pub camera_explicit: bool,
    pub graphics: crate::graphics::Quality,
    pub graphics_explicit: bool,
    pub vsync: bool,
}

impl Options {
    pub const USAGE: &'static str = "usage: f1-the-game [options]
  --assets DIR        asset directory (default: ./assets)
  --track NAME        circuit (default: spa; other circuits require an exp build)
  --car rb6|rb22      car and matching AI field (default: rb6, V8 era)
  --laps N            race length (default: 3)
  --opponents N       AI cars (default: 7)
  --grid N            starting slot, 1 = pole (default: 6)
  --front-wing N      0-10 (default: 5)
  --rear-wing N       0-10 (default: 5)
  --tyres soft|medium|hard
  --auto-gears        let the car shift for you
  --manual-gears      disable automatic shifting
  --mute              start with sound muted (M toggles sound)
  --quick-race        skip the race selection menu
  --camera cockpit|tv|chase   starting view (default: chase)
  --assists 0-1       driving aid strength (default: 1.0, automatic gears)
  --graphics low|medium|high|ultra   graphics preset (default: high)
  --no-vsync          render as fast as the GPU allows
  --size WxH          window size (default: 1600x900)
  --frames N --capture FILE   render N frames, save the last one, exit
  --benchmark DIR     20-second moving-start, two-AI benchmark + sampled PNGs
  --benchmark-seed N  deterministic drive and screenshot seed (default: 24301)
  --benchmark-start M start distance along circuit in metres (default: 0)";

    pub fn parse(mut args: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut options = Self {
            assets: PathBuf::from("assets"),
            track: "spa".to_string(),
            car: "rb6".to_string(),
            laps: 3,
            opponents: 7,
            grid_slot: 6,
            setup: Setup::default(),
            automatic_gears: true,
            assist_level: 1.0,
            width: 1600,
            height: 900,
            frames: None,
            capture: None,
            benchmark: None,
            benchmark_seed: 0x5EED,
            benchmark_start: 0.,
            mute: false,
            quick_race: false,
            camera: Mode::Chase,
            driver_aids: None,
            corner_braking: 1.,
            aids_explicit: false,
            camera_explicit: false,
            graphics: crate::graphics::Quality::High,
            graphics_explicit: false,
            vsync: true,
        };
        while let Some(flag) = args.next() {
            let mut value = || args.next().ok_or(format!("{flag} needs a value"));
            let number = |text: String| {
                text.parse::<f64>()
                    .ok()
                    .filter(|n| n.is_finite())
                    .ok_or_else(|| format!("bad number: {text}"))
            };
            match flag.as_str() {
                "--assets" => options.assets = PathBuf::from(value()?),
                "--track" => options.track = value()?,
                "--car" => {
                    options.car = value()?;
                    if !matches!(options.car.as_str(), "rb6" | "rb22") {
                        return Err("car must be rb6 or rb22".into());
                    }
                }
                "--laps" => {
                    options.laps = value()?
                        .parse::<u32>()
                        .map_err(|_| "laps must be a positive integer")?
                        .max(1)
                }
                "--opponents" => {
                    options.opponents = value()?
                        .parse::<usize>()
                        .map_err(|_| "opponents must be an integer")?
                        .min(15)
                }
                "--grid" => {
                    options.grid_slot = value()?
                        .parse::<usize>()
                        .map_err(|_| "grid must be an integer")?
                        .max(1)
                }
                "--front-wing" => options.setup.front_wing = number(value()?)?,
                "--rear-wing" => options.setup.rear_wing = number(value()?)?,
                "--tyres" => {
                    options.setup.compound = match value()?.as_str() {
                        "soft" => Compound::Soft,
                        "medium" => Compound::Medium,
                        "hard" => Compound::Hard,
                        other => return Err(format!("unknown tyre compound {other}")),
                    }
                }
                "--auto-gears" => {
                    options.automatic_gears = true;
                    options.aids_explicit = true;
                }
                "--manual-gears" => {
                    options.automatic_gears = false;
                    options.aids_explicit = true;
                }
                "--mute" => options.mute = true,
                "--no-vsync" => options.vsync = false,
                "--quick-race" => options.quick_race = true,
                "--camera" => {
                    options.camera_explicit = true;
                    options.camera = match value()?.as_str() {
                        "cockpit" => Mode::Cockpit,
                        "tv" => Mode::Tv,
                        "chase" => Mode::Chase,
                        _ => return Err("camera must be cockpit, tv or chase".into()),
                    }
                }
                "--graphics" => {
                    use crate::graphics::Quality;
                    options.graphics_explicit = true;
                    options.graphics = match value()?.as_str() {
                        "low" => Quality::Low,
                        "medium" => Quality::Medium,
                        "high" => Quality::High,
                        "ultra" => Quality::Ultra,
                        _ => return Err("graphics must be low, medium, high or ultra".into()),
                    }
                }
                "--assists" => {
                    options.assist_level = number(value()?)?.clamp(0.0, 1.0);
                    options.corner_braking = options.assist_level;
                    options.automatic_gears = options.assist_level > 0.;
                    options.aids_explicit = true;
                }
                "--size" => {
                    let text = value()?;
                    let (w, h) = text.split_once('x').ok_or("size must be WxH")?;
                    options.width = w.parse::<u32>().map_err(|_| "width must be an integer")?;
                    options.height = h.parse::<u32>().map_err(|_| "height must be an integer")?;
                    if options.width == 0 || options.height == 0 {
                        return Err("window size must be nonzero".into());
                    }
                }
                "--frames" => {
                    let frames = value()?
                        .parse::<u64>()
                        .map_err(|_| "frames must be a positive integer")?;
                    if frames == 0 {
                        return Err("frames must be nonzero".into());
                    }
                    options.frames = Some(frames);
                }
                "--capture" => options.capture = Some(PathBuf::from(value()?)),
                "--benchmark" => options.benchmark = Some(PathBuf::from(value()?)),
                "--benchmark-seed" => {
                    options.benchmark_seed = value()?
                        .parse()
                        .map_err(|_| "benchmark seed must be an unsigned integer")?
                }
                "--benchmark-start" => {
                    options.benchmark_start = number(value()?)?;
                    if options.benchmark_start < 0. {
                        return Err("benchmark start must be nonnegative".into());
                    }
                }
                "--help" | "-h" => return Err("help".to_string()),
                other => return Err(format!("unknown option {other}")),
            }
        }
        if options.benchmark.is_some() {
            if options.frames.is_some() || options.capture.is_some() {
                return Err("--benchmark cannot be combined with --frames or --capture".into());
            }
            options.opponents = 1;
            options.grid_slot = 1;
            options.quick_race = true;
            options.vsync = false;
        }
        options.setup = options.setup.clamped();
        options.grid_slot = options.grid_slot.min(options.opponents + 1);
        if options.capture.is_some() && options.frames.is_none() {
            return Err("--capture requires --frames".into());
        }
        options.validate_track()?;
        Ok(options)
    }

    pub fn track_enabled(track: &str) -> bool {
        track == "spa" || cfg!(feature = "exp")
    }

    pub fn validate_track(&self) -> Result<(), String> {
        if Self::track_enabled(&self.track) {
            Ok(())
        } else {
            Err(format!(
                "Only Spa is supported in this build. Track '{}' requires the experimental build (cargo build --release -p f1-the-game --features exp).",
                self.track
            ))
        }
    }

    pub fn player_assists(&self) -> Assists {
        self.driver_aids.unwrap_or(Assists {
            steering: self.assist_level,
            braking: self.assist_level,
            traction: self.assist_level,
            stability: self.assist_level,
            automatic_gears: self.automatic_gears,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Options, String> {
        Options::parse(args.iter().map(|arg| (*arg).to_owned()))
    }

    #[test]
    fn benchmark_has_two_computer_slots_and_rejects_conflicting_capture_modes() {
        for args in [
            vec!["--benchmark", "out", "--opponents", "12", "--grid", "8"],
            vec!["--opponents", "12", "--grid", "8", "--benchmark", "out"],
        ] {
            let options = parse(&args).unwrap();
            assert_eq!(options.opponents, 1);
            assert_eq!(options.grid_slot, 1);
            assert!(options.quick_race && !options.vsync);
        }
        for args in [
            vec!["--benchmark", "out", "--frames", "100"],
            vec!["--benchmark", "out", "--capture", "out.png"],
            vec!["--benchmark", "out", "--benchmark-start", "-1"],
            vec!["--benchmark", "out", "--benchmark-seed", "-1"],
        ] {
            assert!(parse(&args).is_err());
        }
    }

    #[test]
    fn setup_and_grid_are_applied_to_the_selected_field() {
        let options = parse(&[
            "--opponents",
            "2",
            "--grid",
            "6",
            "--laps",
            "5",
            "--front-wing",
            "9",
            "--rear-wing",
            "2",
            "--tyres",
            "hard",
            "--auto-gears",
            "--assists",
            "0.4",
        ])
        .unwrap();
        assert_eq!(options.grid_slot, 3);
        assert_eq!(options.opponents, 2);
        assert_eq!(options.laps, 5);
        assert_eq!(options.setup.front_wing, 9.0);
        assert_eq!(options.setup.rear_wing, 2.0);
        assert_eq!(options.setup.compound, Compound::Hard);
        assert!(options.automatic_gears);
        assert_eq!(options.assist_level, 0.4);
    }

    #[test]
    fn rejects_unusable_window_capture_and_nonfinite_setup() {
        for args in [
            vec!["--size", "0x900"],
            vec!["--size", "800xNaN"],
            vec!["--frames", "0"],
            vec!["--capture", "out.png"],
            vec!["--front-wing", "NaN"],
            vec!["--assists", "inf"],
            vec!["--opponents", "1.5"],
            vec!["--car", "rb9"],
        ] {
            assert!(parse(&args).is_err(), "accepted {args:?}");
        }
    }

    #[test]
    fn audio_starts_enabled_unless_muted() {
        assert!(!parse(&[]).unwrap().mute);
        assert!(parse(&["--mute"]).unwrap().mute);
    }

    #[test]
    fn unfinished_tracks_require_the_exp_build() {
        assert_eq!(parse(&[]).unwrap().track, "spa");
        assert!(parse(&["--track", "spa"]).is_ok());
        for track in ["silverstone", "bahrain", "cota", "other", "../tracks/spa"] {
            let result = parse(&["--track", track]);
            assert_eq!(result.is_ok(), cfg!(feature = "exp"), "{track}");
            if let Err(error) = result {
                assert!(error.contains("--features exp"));
            }
        }
    }

    #[test]
    #[cfg(not(feature = "exp"))]
    fn race_loading_rejects_experimental_tracks_even_without_cli_parsing() {
        let mut options = parse(&[]).unwrap();
        options.track = "silverstone".into();
        let error = crate::assets::load_race(&options).err().unwrap();
        assert!(error.contains("Only Spa is supported"), "{error}");
    }

    #[test]
    fn reference_era_v8_is_default_and_modern_car_remains_selectable() {
        assert_eq!(parse(&[]).unwrap().car, "rb6");
        assert_eq!(parse(&["--car", "rb22"]).unwrap().car, "rb22");
    }

    #[test]
    fn individual_aids_override_global_level_and_can_be_completely_disabled() {
        let mut options = parse(&["--assists", "0", "--auto-gears"]).unwrap();
        assert_eq!(options.player_assists().steering, 0.);
        let selected = Assists {
            steering: 0.2,
            braking: 0.,
            traction: 0.8,
            stability: 0.,
            automatic_gears: false,
        };
        options.driver_aids = Some(selected);
        assert_eq!(options.player_assists(), selected);
    }
}
