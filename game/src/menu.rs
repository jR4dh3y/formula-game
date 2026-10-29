//! Asset discovery and pre-race setup. The race starts in a new process because
//! winit permits only one event loop per process, and track materials are static.

use crate::options::Options;
use dhool_pro::input::{GamepadButton, Key};
use dhool_pro::math::{Color, Vec3};
use dhool_pro::{
    run_live_game, Align, AppExit, FontId, GameEngine, Hud, HudFont, InputState, LiveOptions,
    Material, Mesh, ProjectSettings, SceneBuilder, SystemContext,
};
use racecraft_sim::{Compound, Setup};
use serde::Deserialize;
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Deserialize)]
struct Metadata {
    name: String,
    length: f64,
    corners: Vec<serde_json::Value>,
}

struct TrackEntry {
    directory: String,
    metadata: Metadata,
}

fn discover(root: &Path) -> Result<Vec<TrackEntry>, String> {
    let directory = root.join("tracks");
    let mut tracks = Vec::new();
    for entry in std::fs::read_dir(&directory).map_err(|error| {
        format!(
            "{}: {error}; run tools/build_assets.sh",
            directory.display()
        )
    })? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path().join("track.json");
        if !path.is_file() {
            continue;
        }
        let metadata: Metadata = serde_json::from_slice(
            &std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?,
        )
        .map_err(|error| format!("{}: {error}", path.display()))?;
        tracks.push(TrackEntry {
            directory: entry
                .file_name()
                .into_string()
                .map_err(|_| "non-UTF8 track name")?,
            metadata,
        });
    }
    tracks.sort_by(|a, b| a.metadata.name.cmp(&b.metadata.name));
    if tracks.is_empty() {
        return Err(format!(
            "no tracks in {}; run tools/build_assets.sh",
            directory.display()
        ));
    }
    Ok(tracks)
}

#[derive(Clone)]
struct Selection {
    track: String,
    laps: u32,
    opponents: usize,
    setup: Setup,
}

struct Menu {
    tracks: Vec<TrackEntry>,
    track: usize,
    row: usize,
    selection: Selection,
    font: FontId,
    result: Arc<Mutex<Option<Selection>>>,
}

fn pressed(input: InputState, key: Key, button: GamepadButton) -> bool {
    input.keys_pressed.contains(key) || input.gamepad.pressed.contains(button)
}

impl Menu {
    fn update(&mut self, input: InputState) -> bool {
        if pressed(input, Key::ArrowDown, GamepadButton::DPadDown) {
            self.row = (self.row + 1) % 7;
        }
        if pressed(input, Key::ArrowUp, GamepadButton::DPadUp) {
            self.row = (self.row + 6) % 7;
        }
        let change = i32::from(pressed(input, Key::ArrowRight, GamepadButton::DPadRight))
            - i32::from(pressed(input, Key::ArrowLeft, GamepadButton::DPadLeft));
        if change != 0 {
            match self.row {
                0 => {
                    self.track =
                        (self.track as i32 + change).rem_euclid(self.tracks.len() as i32) as usize
                }
                1 => {
                    self.selection.laps =
                        (self.selection.laps as i64 + i64::from(change)).clamp(1, 99) as u32
                }
                2 => {
                    self.selection.opponents =
                        (self.selection.opponents as i32 + change).clamp(0, 15) as usize
                }
                3 => {
                    let current = match self.selection.setup.compound {
                        Compound::Soft => 0,
                        Compound::Medium => 1,
                        Compound::Hard => 2,
                    };
                    self.selection.setup.compound = match (current + change).rem_euclid(3) {
                        0 => Compound::Soft,
                        1 => Compound::Medium,
                        _ => Compound::Hard,
                    };
                }
                4 => {
                    self.selection.setup.front_wing =
                        (self.selection.setup.front_wing + f64::from(change)).clamp(0.0, 10.0)
                }
                5 => {
                    self.selection.setup.rear_wing =
                        (self.selection.setup.rear_wing + f64::from(change)).clamp(0.0, 10.0)
                }
                _ => {}
            }
        }
        if pressed(input, Key::Enter, GamepadButton::South) {
            if self.row == 6 {
                self.selection
                    .track
                    .clone_from(&self.tracks[self.track].directory);
                *self.result.lock().expect("menu result") = Some(self.selection.clone());
                return true;
            }
            self.row += 1;
        }
        pressed(input, Key::Escape, GamepadButton::East)
    }

    fn draw(&self, hud: &mut Hud) {
        let white = Color::rgb(0.94, 0.95, 0.98);
        let dim = Color::rgb(0.6, 0.65, 0.73);
        let accent = Color::rgb(0.93, 0.13, 0.17);
        hud.clear();
        let w = hud.width() as f32;
        let h = hud.height() as f32;
        hud.fill_rect(0.0, 0.0, w, h, Color::rgb(0.018, 0.024, 0.035));
        let x = (w * 0.08).max(20.0);
        let top = (h * 0.12).max(20.0);
        let step = ((h - top - 140.0) / 9.0).clamp(24.0, 62.0);
        hud.text(self.font, 36.0, (x, top), Align::Left, white, "RACECRAFT");
        hud.text(
            self.font,
            18.0,
            (x, top + 45.0),
            Align::Left,
            accent,
            "PRE-RACE SETUP",
        );
        let entry = &self.tracks[self.track];
        let compound = match self.selection.setup.compound {
            Compound::Soft => "Soft",
            Compound::Medium => "Medium",
            Compound::Hard => "Hard",
        };
        let rows = [
            format!("Track   < {} >", entry.metadata.name),
            format!("Laps   < {} >", self.selection.laps),
            format!("Opponents   < {} >", self.selection.opponents),
            format!("Tyres   < {compound} >"),
            format!("Front wing   < {:.0} >", self.selection.setup.front_wing),
            format!("Rear wing   < {:.0} >", self.selection.setup.rear_wing),
            "START RACE".to_string(),
        ];
        for (i, label) in rows.iter().enumerate() {
            let y = top + 95.0 + step * i as f32;
            if i == self.row {
                hud.fill_rect(
                    x - 10.0,
                    y - 5.0,
                    w - 2.0 * x,
                    step - 2.0,
                    Color::rgb(0.11, 0.14, 0.19),
                );
                hud.fill_rect(x - 10.0, y - 5.0, 4.0, step - 2.0, accent);
            }
            hud.text(self.font, 22.0, (x + 8.0, y), Align::Left, white, label);
        }
        if w >= 1200.0 {
            let column = w * 0.58;
            hud.text(
                self.font,
                18.0,
                (column, top + 95.0 + step),
                Align::Left,
                accent,
                "AVAILABLE TRACKS",
            );
            for (i, track) in self.tracks.iter().enumerate() {
                let y = top + 130.0 + step + i as f32 * 52.0;
                let color = if i == self.track { white } else { dim };
                hud.text(
                    self.font,
                    20.0,
                    (column, y),
                    Align::Left,
                    color,
                    &track.metadata.name,
                );
                hud.text(
                    self.font,
                    15.0,
                    (column, y + 24.0),
                    Align::Left,
                    dim,
                    &format!(
                        "{:.3} km   /   {} corners",
                        track.metadata.length / 1000.0,
                        track.metadata.corners.len()
                    ),
                );
            }
        }
        hud.text(
            self.font,
            18.0,
            (x, h - 90.0),
            Align::Left,
            dim,
            &format!(
                "{:.3} km   /   {} corners   /   track {} of {}",
                entry.metadata.length / 1000.0,
                entry.metadata.corners.len(),
                self.track + 1,
                self.tracks.len()
            ),
        );
        hud.text(
            self.font,
            17.0,
            (x, h - 55.0),
            Align::Left,
            dim,
            "Up/Down: select   Left/Right: change   Enter / A: confirm   Esc / B: quit",
        );
    }
}

/// Returns true when discovery or the menu handled this launch completely.
pub fn handle_launch(options: &Options) -> Result<bool, String> {
    if options.track.is_some() && !options.list_tracks {
        return Ok(false);
    }
    let tracks = discover(&options.assets)?;
    if options.list_tracks {
        for entry in tracks {
            println!(
                "{}\t{}\t{:.3} km\t{} corners",
                entry.directory,
                entry.metadata.name,
                entry.metadata.length / 1000.0,
                entry.metadata.corners.len()
            );
        }
        return Ok(true);
    }
    let mut hud = Hud::new();
    let font = hud.add_font(HudFont::from_bytes(include_bytes!(
        "../fonts/TitilliumWeb-SemiBold.ttf"
    ))?);
    let result = Arc::new(Mutex::new(None));
    let menu = Menu {
        tracks,
        track: 0,
        row: 0,
        selection: Selection {
            track: String::new(),
            laps: options.laps,
            opponents: options.opponents,
            setup: options.setup,
        },
        font,
        result: result.clone(),
    };
    // The renderer requires nonempty geometry buffers. This backdrop keeps
    // the menu independent of car/track assets; the HUD covers the scene.
    let mut builder = SceneBuilder::new();
    let backdrop = builder.add_material(Material::standard(
        "menu-backdrop",
        Color::rgb(0.02, 0.025, 0.035),
    ));
    builder.add_mesh(Mesh::cuboid("menu-backdrop", Vec3::ONE), backdrop);
    let scene = builder.build().map_err(|e| e.to_string())?;
    let game = GameEngine::builder(ProjectSettings::default())
        .scene(scene)
        .resource(hud)
        .resource(menu)
        .update_system("pre-race-menu", |context: &mut SystemContext<'_>| {
            let mut hud = context
                .resources
                .remove::<Hud>()
                .ok_or("missing menu HUD")?;
            let menu = context.resources.get_mut::<Menu>().ok_or("missing menu")?;
            if menu.update(context.input) {
                context.events.send(AppExit);
            }
            menu.draw(&mut hud);
            context.resources.insert(hud);
            Ok(())
        })
        .build()
        .map_err(|e| e.to_string())?;
    run_live_game(
        game,
        LiveOptions {
            title: "Racecraft • Pre-race setup".to_string(),
            width: options.width,
            height: options.height,
            max_frames: options.frames,
            capture_path: options.capture.clone(),
            capture_ui: true,
            ..LiveOptions::default()
        },
    )
    .map_err(|e| e.to_string())?;
    let selection = result.lock().map_err(|e| e.to_string())?.take();
    if let Some(selection) = selection {
        let compound = match selection.setup.compound {
            Compound::Soft => "soft",
            Compound::Medium => "medium",
            Compound::Hard => "hard",
        };
        let status =
            std::process::Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
                .args(std::env::args_os().skip(1))
                .args([
                    "--track",
                    &selection.track,
                    "--laps",
                    &selection.laps.to_string(),
                    "--opponents",
                    &selection.opponents.to_string(),
                    "--tyres",
                    compound,
                    "--front-wing",
                    &selection.setup.front_wing.to_string(),
                    "--rear-wing",
                    &selection.setup.rear_wing.to_string(),
                ])
                .status()
                .map_err(|e| format!("start race: {e}"))?;
        if !status.success() {
            return Err(format!("race exited with {status}"));
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn menu() -> Menu {
        let mut hud = Hud::new();
        Menu {
            tracks: ["spa", "silverstone"]
                .map(|name| TrackEntry {
                    directory: name.to_string(),
                    metadata: Metadata {
                        name: name.to_string(),
                        length: 5891.0,
                        corners: vec![],
                    },
                })
                .into(),
            track: 0,
            row: 0,
            selection: Selection {
                track: String::new(),
                laps: 3,
                opponents: 7,
                setup: Setup::default(),
            },
            font: hud.add_font(
                HudFont::from_bytes(include_bytes!("../fonts/TitilliumWeb-SemiBold.ttf")).unwrap(),
            ),
            result: Arc::new(Mutex::new(None)),
        }
    }

    fn key(key: Key) -> InputState {
        let mut input = InputState::default();
        input.keys_pressed.insert(key);
        input
    }

    fn button(button: GamepadButton) -> InputState {
        let mut input = InputState::default();
        input.gamepad.pressed.insert(button);
        input
    }

    #[test]
    fn keyboard_and_gamepad_select_and_launch_the_same_setup() {
        let mut menu = menu();
        menu.update(key(Key::ArrowRight));
        menu.update(button(GamepadButton::South));
        menu.update(button(GamepadButton::DPadRight));
        menu.update(key(Key::Enter));
        menu.update(key(Key::ArrowLeft));
        menu.update(key(Key::Enter));
        menu.update(button(GamepadButton::DPadRight));
        menu.update(key(Key::Enter));
        menu.update(key(Key::ArrowRight));
        menu.update(key(Key::Enter));
        menu.update(button(GamepadButton::DPadLeft));
        menu.update(key(Key::Enter));
        assert!(menu.update(button(GamepadButton::South)));
        let result = menu.result.lock().unwrap();
        let selected = result.as_ref().unwrap();
        assert_eq!(selected.track, "silverstone");
        assert_eq!(selected.laps, 4);
        assert_eq!(selected.opponents, 6);
        assert_eq!(selected.setup.compound, Compound::Hard);
        assert_eq!(selected.setup.front_wing, 6.0);
        assert_eq!(selected.setup.rear_wing, 4.0);
    }

    #[test]
    fn cancel_does_not_launch_and_values_stay_in_bounds() {
        let mut menu = menu();
        menu.update(key(Key::ArrowUp));
        assert_eq!(menu.row, 6);
        menu.row = 4;
        for _ in 0..20 {
            menu.update(key(Key::ArrowRight));
        }
        assert_eq!(menu.selection.setup.front_wing, 10.0);
        menu.row = 2;
        for _ in 0..20 {
            menu.update(button(GamepadButton::DPadLeft));
        }
        assert_eq!(menu.selection.opponents, 0);
        assert!(menu.update(key(Key::Escape)));
        assert!(menu.result.lock().unwrap().is_none());
    }
}
