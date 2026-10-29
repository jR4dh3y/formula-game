//! Command-line options.

use racecraft_sim::{Compound, Setup};
use std::path::PathBuf;

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
    pub profile: bool,
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
  --profile           print CPU and GPU frame timings
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
            profile: false,
        };
        while let Some(flag) = args.next() {
            let mut value = || args.next().ok_or(format!("{flag} needs a value"));
            let number = |text: String| {
                text.parse::<f64>()
                    .map_err(|_| format!("bad number: {text}"))
            };
            match flag.as_str() {
                "--assets" => options.assets = PathBuf::from(value()?),
                "--track" => options.track = value()?,
                "--laps" => options.laps = number(value()?)?.max(1.0) as u32,
                "--opponents" => options.opponents = number(value()?)?.clamp(0.0, 15.0) as usize,
                "--grid" => options.grid_slot = number(value()?)?.max(1.0) as usize,
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
                    options.width = number(w.to_string())? as u32;
                    options.height = number(h.to_string())? as u32;
                }
                "--frames" => options.frames = Some(number(value()?)? as u64),
                "--capture" => options.capture = Some(PathBuf::from(value()?)),
                "--profile" => options.profile = true,
                "--help" | "-h" => return Err("help".to_string()),
                other => return Err(format!("unknown option {other}")),
            }
        }
        options.setup = options.setup.clamped();
        options.grid_slot = options.grid_slot.min(options.opponents + 1);
        Ok(options)
    }
}
