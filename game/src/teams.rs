//! Team data and race identities stay on the presentation side of the sim boundary.
use bevy::prelude::Resource;
use serde::Deserialize;
use std::{collections::HashSet, path::Path};

#[derive(Clone, Default, Deserialize, Resource)]
pub struct Teams {
    pub teams: Vec<Team>,
}
#[derive(Clone, Deserialize)]
pub struct Team {
    pub id: String,
    pub name: String,
    pub short: String,
    pub colors: Colors,
    pub drivers: Vec<Driver>,
}
#[derive(Clone, Deserialize)]
pub struct Colors {
    pub primary: String,
    pub secondary: String,
    pub accent: String,
    pub carbon: String,
}
#[derive(Clone, Deserialize)]
pub struct Driver {
    pub id: String,
    pub name: String,
    pub abbr: String,
    pub number: u32,
    #[serde(default)]
    pub helmets: Vec<serde_json::Value>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Identity {
    pub team: String,
    pub driver: String,
    pub name: String,
    pub abbr: String,
    pub team_short: String,
    pub primary: [u8; 3],
}

pub fn rgb(hex: &str) -> Option<[u8; 3]> {
    let hex = hex.strip_prefix('#')?;
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    Some([
        u8::from_str_radix(&hex[0..2], 16).ok()?,
        u8::from_str_radix(&hex[2..4], 16).ok()?,
        u8::from_str_radix(&hex[4..6], 16).ok()?,
    ])
}

impl Teams {
    pub fn load(root: &Path) -> Result<Self, String> {
        let path = root.join("cars/gen22/teams.json");
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&bytes).map_err(|e| format!("{}: {e}", path.display()))
    }
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let data: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if data.teams.is_empty() {
            return Err("teams must not be empty".into());
        }
        let mut teams = HashSet::new();
        let mut drivers = HashSet::new();
        let safe = |id: &str| {
            !id.is_empty()
                && id
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'-')
        };
        for team in &data.teams {
            if !safe(&team.id) || !teams.insert(&team.id) || team.drivers.is_empty() {
                return Err(format!(
                    "invalid/duplicate team or empty driver list: {}",
                    team.id
                ));
            }
            if team.short.trim().is_empty()
                || team.short.chars().count() > 12
                || team.short.chars().any(char::is_control)
            {
                return Err(format!(
                    "team short label must contain 1..=12 printable characters: {}",
                    team.id
                ));
            }
            for color in [
                &team.colors.primary,
                &team.colors.secondary,
                &team.colors.accent,
                &team.colors.carbon,
            ] {
                if rgb(color).is_none() {
                    return Err(format!("invalid team colour {color}"));
                }
            }
            for driver in &team.drivers {
                if !safe(&driver.id) || !drivers.insert(&driver.id) || driver.abbr.len() != 3 {
                    return Err(format!("invalid/duplicate driver: {}", driver.id));
                }
                // Retained for future helmet selection. Extra schema keys are ignored.
                let _ = &driver.helmets;
            }
        }
        Ok(data)
    }
    pub fn selection(&self, team: &str, driver: &str) -> Result<(&Team, &Driver), String> {
        let team = if team.is_empty() {
            self.teams.first()
        } else {
            self.teams.iter().find(|t| t.id == team)
        }
        .ok_or_else(|| format!("unknown gen22 team '{team}'"))?;
        let driver = if driver.is_empty() {
            team.drivers.first()
        } else {
            team.drivers.iter().find(|d| d.id == driver)
        }
        .ok_or_else(|| format!("unknown driver '{driver}' for team '{}'", team.id))?;
        Ok((team, driver))
    }
    pub fn identity(team: &Team, driver: &Driver) -> Identity {
        Identity {
            team: team.id.clone(),
            driver: driver.id.clone(),
            name: driver.name.clone(),
            abbr: driver.abbr.clone(),
            team_short: team.short.clone(),
            primary: rgb(&team.colors.primary).unwrap(),
        }
    }
    /// First drivers across teams, then second drivers: teammates are spread across the grid.
    /// Put the selected driver's teammate first in the candidate list so even a small field includes them.
    pub fn field(
        &self,
        options: &crate::options::Options,
    ) -> Result<Vec<Option<Identity>>, String> {
        if options.car == "rb6" {
            return Ok(vec![None; options.opponents + 1]);
        }
        let (team, driver) = self.selection(&options.team, &options.driver)?;
        let player = Self::identity(team, driver);
        let mut candidates: Vec<_> = team
            .drivers
            .iter()
            .filter(|d| d.id != driver.id)
            .map(|d| Self::identity(team, d))
            .collect();
        for index in 0..self
            .teams
            .iter()
            .map(|t| t.drivers.len())
            .max()
            .unwrap_or(0)
        {
            for other in &self.teams {
                if other.id == team.id {
                    continue;
                }
                if let Some(driver) = other.drivers.get(index) {
                    candidates.push(Self::identity(other, driver));
                }
            }
        }
        if candidates.len() < options.opponents {
            return Err("not enough real drivers for the requested field".into());
        }
        let mut field: Vec<_> = candidates
            .into_iter()
            .take(options.opponents)
            .map(Some)
            .collect();
        // Spread the teammate away from the player rather than always pairing adjacent grid slots.
        if field.len() > 2 {
            let last = field.len() - 1;
            field.swap(0, last);
        }
        field.insert(options.grid_slot - 1, Some(player));
        Ok(field)
    }
}

/// Rig metadata is deliberately game-side; CarGeometry ignores these extra JSON keys.
#[derive(Clone, Default, Deserialize)]
pub struct Rig {
    pub cockpit_eye: Option<[f64; 3]>,
    #[serde(default)]
    pub drs: Drs,
    /// Steering-wheel angles (radians about the `steering_wheel` node's +Z) at
    /// which the driver's arms and head were baked, one per morph target.
    #[serde(default)]
    pub driver_poses: Vec<f32>,
    /// Finger-pull morph targets ("up", "down"), after the steering poses.
    #[serde(default)]
    pub driver_shift_poses: Vec<String>,
    #[serde(default)]
    pub paddles: Paddles,
}
/// Paddle travel about each paddle node's local +Y, in radians.
#[derive(Clone, Default, Deserialize)]
pub struct Paddles {
    pub up_angle: f32,
    pub down_angle: f32,
}
#[derive(Clone, Default, Deserialize)]
pub struct Drs {
    pub open_angle: f32,
}
impl Rig {
    /// Morph weights that place the driver at `wheel` radians of steering-wheel
    /// rotation, interpolating linearly between neighbouring baked poses; the
    /// unmorphed mesh is the straight-ahead pose.
    pub fn driver_weights(&self, wheel: f32, weights: &mut [f32]) {
        weights.fill(0.);
        let mut knots: Vec<(f32, Option<usize>)> = self
            .driver_poses
            .iter()
            .enumerate()
            .map(|(index, &angle)| (angle, Some(index)))
            .chain([(0., None)])
            .collect();
        knots.sort_by(|a, b| a.0.total_cmp(&b.0));
        let wheel = wheel.clamp(knots[0].0, knots[knots.len() - 1].0);
        let Some(pair) = knots.windows(2).find(|pair| wheel <= pair[1].0) else {
            return;
        };
        let t = (wheel - pair[0].0) / (pair[1].0 - pair[0].0).max(f32::EPSILON);
        for ((_, index), weight) in pair.iter().zip([1. - t, t]) {
            if let Some(slot) = index.and_then(|index| weights.get_mut(index)) {
                *slot = weight;
            }
        }
    }
    /// Adds finger pulls (0..1 for the upshift and downshift hands) to the
    /// weights from [`Self::driver_weights`].
    pub fn shift_weights(&self, pulls: [f32; 2], weights: &mut [f32]) {
        for (offset, name) in self.driver_shift_poses.iter().enumerate() {
            let pull = match name.as_str() {
                "up" => pulls[0],
                "down" => pulls[1],
                _ => continue,
            };
            if let Some(weight) = weights.get_mut(self.driver_poses.len() + offset) {
                *weight = pull;
            }
        }
    }
    pub fn load(options: &crate::options::Options) -> Result<Self, String> {
        let path = options
            .assets
            .join(format!("cars/{}/car.json", options.car));
        serde_json::from_slice(&std::fs::read(&path).map_err(|e| e.to_string())?)
            .map_err(|e| format!("{}: {e}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Teams {
        let teams: Vec<_> = (0..10).map(|i| serde_json::json!({"id":format!("team{i}"),"name":"Team", "short":"TM", "colors":{"primary":"#AB0102","secondary":"#000000","accent":"#FFFFFF","carbon":"#111111"},"drivers":(0..2).map(|j|serde_json::json!({"id":format!("driver{i}_{j}"),"name":"Real Driver","abbr":"DRV","number":i*2+j,"helmets":[],"helmet":{"extra":true}})).collect::<Vec<_>>(),"livery":{"style":"extra"}})).collect();
        Teams::parse(
            &serde_json::to_vec(&serde_json::json!({"season":2022,"teams":teams})).unwrap(),
        )
        .unwrap()
    }
    #[test]
    fn driver_pose_weights_blend_neighbouring_targets() {
        let rig = Rig {
            driver_poses: vec![-2., -1., 1., 2.],
            ..Rig::default()
        };
        let mut weights = [0.; 4];
        let mut check = |wheel, expected: [f32; 4]| {
            rig.driver_weights(wheel, &mut weights);
            for (got, want) in weights.iter().zip(expected) {
                assert!((got - want).abs() < 1e-5, "{wheel}: {weights:?}");
            }
        };
        check(0., [0., 0., 0., 0.]);
        check(0.5, [0., 0., 0.5, 0.]);
        check(1.5, [0., 0., 0.5, 0.5]);
        check(-1.25, [0.25, 0.75, 0., 0.]);
        check(9., [0., 0., 0., 1.]);
        Rig::default().driver_weights(1., &mut []);
        let rig = Rig {
            driver_poses: vec![-1., 1.],
            driver_shift_poses: vec!["up".into(), "down".into()],
            ..Rig::default()
        };
        let mut weights = [0.; 4];
        rig.driver_weights(0.5, &mut weights);
        rig.shift_weights([0.25, 1.], &mut weights);
        assert_eq!(weights, [0., 0.5, 0.25, 1.]);
    }

    #[test]
    fn extra_fields_defaults_and_validation() {
        let data = fixture();
        assert_eq!(data.selection("", "").unwrap().1.id, "driver0_0");
        assert!(data.selection("team0", "driver1_0").is_err());
        assert!(data.selection("missing", "").is_err());
        assert!(Teams::parse(br#"{"teams":[]}"#).is_err());
        assert_eq!(rgb("#AB0102"), Some([171, 1, 2]));
        assert_eq!(rgb("#é0000"), None);
    }
    #[test]
    fn runtime_rosters_enforce_short_label_character_limit() {
        let mut value: serde_json::Value =
            serde_json::from_slice(include_bytes!("../../tools/car/teams.json")).unwrap();
        for label in [
            "",
            "             ",
            "1234567890123",
            "FE\nR",
            "FE\tR",
            "FE\0R",
        ] {
            value["teams"][0]["short"] = label.into();
            assert!(Teams::parse(&serde_json::to_vec(&value).unwrap()).is_err());
        }
        value["teams"][0]["short"] = "é".repeat(12).into();
        assert!(Teams::parse(&serde_json::to_vec(&value).unwrap()).is_ok());
    }

    #[test]
    fn field_has_unique_real_drivers_and_selected_player_at_every_grid_slot() {
        let teams = fixture();
        for slot in 1..=16 {
            let mut options = crate::options::Options::parse(std::iter::empty()).unwrap();
            options.opponents = 15;
            options.grid_slot = slot;
            options.team = "team4".into();
            options.driver = "driver4_1".into();
            let field = teams.field(&options).unwrap();
            assert_eq!(field[slot - 1].as_ref().unwrap().driver, "driver4_1");
            let unique: HashSet<_> = field.iter().map(|e| &e.as_ref().unwrap().driver).collect();
            assert_eq!(unique.len(), 16);
            assert!(unique.contains(&"driver4_0".to_owned()));
            options.car = "rb6".into();
            assert!(teams.field(&options).unwrap().iter().all(Option::is_none));
        }
    }
}
