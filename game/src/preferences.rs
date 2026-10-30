//! User preferences. Automated runs never touch disk.
use crate::{
    camera::{Mode, Tuning},
    options::Options,
};
use racecraft_sim::Assists;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Preferences {
    #[serde(default)]
    pub team: String,
    #[serde(default)]
    pub driver: String,
    pub aids: Assists,
    pub corner_braking: f64,
    pub racing_line: bool,
    pub camera: Mode,
    pub views: BTreeMap<String, Tuning>,
    #[serde(default)]
    pub graphics: crate::graphics::Quality,
    #[serde(default = "vsync_default")]
    pub vsync: bool,
    #[serde(default)]
    pub display: crate::hud::Settings,
    #[serde(default)]
    pub audio: crate::audio::Settings,
}

fn vsync_default() -> bool {
    true
}

impl Preferences {
    pub fn defaults(options: &Options) -> Self {
        Self {
            team: options.team.clone(),
            driver: options.driver.clone(),
            aids: options.player_assists(),
            corner_braking: options.corner_braking,
            racing_line: options.assist_level > 0.,
            camera: options.camera,
            views: ["rb6", "gen22"]
                .map(|id| (id.into(), Tuning::for_car(id)))
                .into(),
            graphics: options.graphics,
            vsync: options.vsync,
            display: crate::hud::Settings::default(),
            audio: crate::audio::Settings::default(),
        }
    }
    pub fn sanitize(&mut self) {
        if let Some(old) = self.views.remove("rb22") {
            self.views.entry("gen22".into()).or_insert(old);
        }
        for id in ["rb6", "gen22"] {
            self.views
                .entry(id.into())
                .or_insert_with(|| Tuning::for_car(id));
        }
        for value in [
            &mut self.aids.steering,
            &mut self.aids.braking,
            &mut self.aids.traction,
            &mut self.aids.stability,
            &mut self.corner_braking,
        ] {
            *value = if value.is_finite() {
                value.clamp(0., 1.)
            } else {
                1.
            };
        }
        for tuning in self.views.values_mut() {
            tuning.sanitize();
        }
        self.audio.sanitize();
    }
}

pub struct Store {
    path: Option<PathBuf>,
    saved: Preferences,
}
impl Store {
    pub fn load(options: &Options) -> (Self, Preferences) {
        let path = if cfg!(test) || options.frames.is_some() || options.quick_race {
            None
        } else {
            std::env::var_os("F1_SETTINGS")
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var_os("XDG_CONFIG_HOME")
                        .map(PathBuf::from)
                        .or_else(|| {
                            std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config"))
                        })
                        .map(|p| p.join("f1-racecraft/preferences.json"))
                })
        };
        let mut prefs = path
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|bytes| serde_json::from_slice::<Preferences>(&bytes).ok())
            .unwrap_or_else(|| Preferences::defaults(options));
        prefs.sanitize();
        if options.camera_explicit {
            prefs.camera = options.camera;
        }
        if options.graphics_explicit {
            prefs.graphics = options.graphics;
        }
        if !options.vsync {
            prefs.vsync = false;
        }
        if options.aids_explicit {
            prefs.aids = options.player_assists();
            prefs.corner_braking = options.corner_braking;
        }
        (
            Self {
                path,
                saved: prefs.clone(),
            },
            prefs,
        )
    }
    pub fn save(&mut self, prefs: Preferences) -> Result<(), String> {
        if prefs == self.saved {
            return Ok(());
        }
        if let Some(path) = &self.path {
            let write = || -> Result<(), Box<dyn std::error::Error>> {
                if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                    std::fs::create_dir_all(parent)?;
                }
                let temporary = path.with_extension("json.tmp");
                std::fs::write(&temporary, serde_json::to_vec_pretty(&prefs)?)?;
                std::fs::rename(temporary, path)?;
                Ok(())
            };
            write().map_err(|e| format!("Could not save preferences: {e}"))?;
        }
        self.saved = prefs;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preferences_round_trip_aids_and_per_car_cameras() {
        let options = Options::parse(std::iter::empty()).unwrap();
        let mut prefs = Preferences::defaults(&options);
        prefs.team = "ferrari".into();
        prefs.driver = "leclerc".into();
        prefs.corner_braking = 0.35;
        prefs.aids.automatic_gears = false;
        prefs.views.get_mut("gen22").unwrap().cockpit.height = 1.12;
        prefs.audio.engine = crate::audio::EngineChoice::V12;
        prefs.audio.opponents = 0.25;
        prefs.display = crate::hud::Settings {
            layout: crate::hud::Layout::Gauges,
            map: crate::hud::MapMode::Nearby,
        };
        let path = std::env::temp_dir().join(format!("f1-prefs-{}.json", std::process::id()));
        let mut store = Store {
            path: Some(path.clone()),
            saved: Preferences::defaults(&options),
        };
        store.save(prefs.clone()).unwrap();
        let restored: Preferences = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(restored == prefs);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn legacy_modern_camera_key_migrates_without_overwriting_new_views() {
        let mut prefs = Preferences::defaults(&Options::parse(std::iter::empty()).unwrap());
        let mut old = crate::camera::Tuning::for_car("gen22");
        old.cockpit.height = 1.12;
        prefs.views.remove("gen22");
        prefs.views.insert("rb22".into(), old.clone());
        prefs.sanitize();
        assert_eq!(prefs.views["gen22"], old);
        assert!(!prefs.views.contains_key("rb22"));
        prefs
            .views
            .insert("rb22".into(), crate::camera::Tuning::for_car("rb6"));
        prefs.sanitize();
        assert_eq!(prefs.views["gen22"], old);
    }

    #[test]
    fn older_preferences_default_to_segment_display_and_full_circuit() {
        let prefs = Preferences::defaults(&Options::parse(std::iter::empty()).unwrap());
        let mut json = serde_json::to_value(&prefs).unwrap();
        json.as_object_mut().unwrap().remove("display");
        let restored: Preferences = serde_json::from_value(json).unwrap();
        assert_eq!(restored.display.layout, crate::hud::Layout::Segments);
        assert_eq!(restored.display.map, crate::hud::MapMode::FullCircuit);
    }
}
