//! Command-line options.

use racecraft_sim::{Compound, Setup};
use std::path::PathBuf;

#[derive(bevy::prelude::Resource)]
pub struct Options {
    pub assets: PathBuf,
    pub track: String,
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
}

impl Options {
    pub const USAGE: &'static str = "usage: f1-the-game [options]
  --assets DIR        asset directory (default: ./assets)
  --track NAME        track directory under assets/tracks (default: spa)
  --laps N            race length (default: 3)
  --opponents N       AI cars (default: 7)
  --grid N            starting slot, 1 = pole (default: 6)
  --front-wing N      0-10 (default: 5)
  --rear-wing N       0-10 (default: 5)
  --tyres soft|medium|hard
  --auto-gears        let the car shift for you
  --assists 0-1       driving aid strength (default: 0.6)
  --size WxH          window size (default: 1600x900)
  --frames N --capture FILE   render N frames, save the last one, exit";

    pub fn parse(mut args: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut options = Self {
            assets: PathBuf::from("assets"),
            track: "spa".to_string(),
            laps: 3,
            opponents: 7,
            grid_slot: 6,
            setup: Setup::default(),
            automatic_gears: false,
            assist_level: 0.6,
            width: 1600,
            height: 900,
            frames: None,
            capture: None,
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
                "--auto-gears" => options.automatic_gears = true,
                "--assists" => options.assist_level = number(value()?)?.clamp(0.0, 1.0),
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
                "--help" | "-h" => return Err("help".to_string()),
                other => return Err(format!("unknown option {other}")),
            }
        }
        options.setup = options.setup.clamped();
        options.grid_slot = options.grid_slot.min(options.opponents + 1);
        if options.capture.is_some() && options.frames.is_none() {
            return Err("--capture requires --frames".into());
        }
        Ok(options)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Options, String> {
        Options::parse(args.iter().map(|arg| (*arg).to_owned()))
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
        ] {
            assert!(parse(&args).is_err(), "accepted {args:?}");
        }
    }
}
