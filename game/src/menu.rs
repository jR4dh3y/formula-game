//! Race setup and pause menus. Inherits the HUD's Titillium type, dark surfaces
//! and red race-control accent. Setup stays editable; only Start replaces a race.

use crate::{
    assets,
    camera::{ChaseCamera, Mode},
    controls::Controls,
    hud,
    options::Options,
    Session,
};
use bevy::prelude::*;
use bevy_egui::{
    egui::{self, Color32, RichText},
    EguiContexts,
};
use racecraft_sim::{Assists, Compound};
use serde::Deserialize;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Main,
    Loading,
    Racing,
    Paused,
    CameraEditor,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tab {
    Race,
    Aids,
    Controls,
    Graphics,
}

pub struct Circuit {
    pub id: String,
    pub name: String,
    pub length: f64,
    pub outline: Vec<[f64; 3]>,
    pub unavailable: Option<String>,
}

pub struct CarChoice {
    pub id: String,
    pub name: &'static str,
    pub description: &'static str,
}

#[derive(Default)]
pub struct Catalog {
    pub circuits: Vec<Circuit>,
    pub cars: Vec<CarChoice>,
}

#[derive(Deserialize)]
struct TrackSummary {
    name: String,
    centerline: Vec<[f64; 3]>,
}

impl Catalog {
    pub fn discover(root: &Path) -> Self {
        let mut catalog = Self::default();
        for (directory, deferred) in [("tracks", false), ("deferred", true)] {
            let Ok(entries) = std::fs::read_dir(root.join(directory)) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let summary = std::fs::read(path.join("track.json"))
                    .ok()
                    .and_then(|data| serde_json::from_slice::<TrackSummary>(&data).ok());
                let Some(summary) = summary else { continue };
                let unavailable = if deferred {
                    Some("Conversion deferred: the drivable surface still needs repair.".into())
                } else if !path.join("visual.glb").is_file()
                    || !path.join("collision.bin").is_file()
                    || summary.centerline.len() < 8
                {
                    Some("Missing converted scene, collision geometry or centerline.".into())
                } else {
                    None
                };
                let length = summary
                    .centerline
                    .iter()
                    .zip(summary.centerline.iter().cycle().skip(1))
                    .take(summary.centerline.len())
                    .map(|(a, b)| {
                        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2))
                            .sqrt()
                    })
                    .sum();
                catalog.circuits.push(Circuit {
                    id: entry.file_name().to_string_lossy().into_owned(),
                    name: summary.name,
                    length,
                    outline: summary.centerline,
                    unavailable,
                });
            }
        }
        catalog.circuits.sort_by(|a, b| {
            a.unavailable
                .is_some()
                .cmp(&b.unavailable.is_some())
                .then(a.name.cmp(&b.name))
        });
        for (id, name, description) in [
            ("rb6", "Red Bull RB6", "2010 / V8 / 7-speed / 18,000 RPM"),
            ("rb22", "Red Bull RB22", "Modern era / 8-speed / 15,000 RPM"),
        ] {
            let car = root.join("cars").join(id);
            if car.join("car.glb").is_file() && car.join("car.json").is_file() {
                catalog.cars.push(CarChoice {
                    id: id.into(),
                    name,
                    description,
                });
            }
        }
        catalog
    }
}

#[derive(Resource)]
pub struct Frontend {
    pub catalog: Catalog,
    pub draft: Options,
    pub aids: Assists,
    pub corner_braking: f64,
    pub show_racing_line: bool,
    pub graphics: crate::graphics::Quality,
    pub views: std::collections::BTreeMap<String, crate::camera::Tuning>,
    store: crate::preferences::Store,
    pub pending: bool,
    pub error: Option<String>,
    pub race_started: bool,
    pub(crate) tab: Tab,
    #[cfg(test)]
    pub widgets: std::collections::HashMap<&'static str, egui::Rect>,
}

impl Frontend {
    pub fn new(options: &Options) -> Self {
        let (store, prefs) = crate::preferences::Store::load(options);
        let mut draft = options.clone();
        draft.camera = prefs.camera;
        Self {
            catalog: Catalog::discover(&options.assets),
            draft,
            aids: prefs.aids,
            corner_braking: prefs.corner_braking,
            show_racing_line: prefs.racing_line,
            graphics: prefs.graphics,
            views: prefs.views,
            store,
            pending: false,
            error: None,
            race_started: options.quick_race || options.frames.is_some(),
            tab: Tab::Race,
            #[cfg(test)]
            widgets: default(),
        }
    }
}

pub fn resume(session: &mut Session) {
    session.screen = Screen::Racing;
    session.paused = false;
    session.controls = Controls::default();
}

/// Commit the pending selection transactionally: CPU-load failures leave the old race intact.
#[allow(clippy::too_many_arguments)]
pub fn apply(
    mut commands: Commands,
    server: Res<AssetServer>,
    mut frontend: ResMut<Frontend>,
    mut options: ResMut<Options>,
    mut session: ResMut<Session>,
    roots: Query<Entity, With<assets::RaceVisualRoot>>,
    mut lights: Query<&mut Transform, With<DirectionalLight>>,
    mut windows: Query<&mut Window>,
) {
    // Persist the actual active camera, not the next race's draft selection.
    if frontend.race_started {
        frontend
            .views
            .insert(options.car.clone(), session.camera.tuning.clone());
        if matches!(session.screen, Screen::Racing | Screen::CameraEditor) {
            frontend.draft.camera = session.camera.mode;
        }
    }
    let prefs = crate::preferences::Preferences {
        aids: frontend.aids,
        corner_braking: frontend.corner_braking,
        racing_line: frontend.show_racing_line,
        camera: frontend.draft.camera,
        views: frontend.views.clone(),
        graphics: frontend.graphics,
    };
    if let Err(error) = frontend.store.save(prefs) {
        frontend.error = Some(error);
    }
    if !frontend.pending {
        return;
    }
    frontend.pending = false;
    let available = frontend
        .catalog
        .circuits
        .iter()
        .any(|track| track.id == frontend.draft.track && track.unavailable.is_none());
    if !available {
        frontend.error = Some("This circuit is not ready. Select an available circuit.".into());
        return;
    }
    if !frontend
        .catalog
        .cars
        .iter()
        .any(|car| car.id == frontend.draft.car)
    {
        frontend.error = Some("This car is not available in the asset directory.".into());
        return;
    }
    let mut selected = frontend.draft.clone();
    selected.driver_aids = Some(frontend.aids);
    selected.automatic_gears = frontend.aids.automatic_gears;
    selected.corner_braking = frontend.corner_braking;
    selected.grid_slot = selected.grid_slot.clamp(1, selected.opponents + 1);
    let race = match assets::load_race(&selected) {
        Ok(race) => race,
        Err(error) => {
            frontend.error = Some(error);
            return;
        }
    };
    for root in &roots {
        commands.entity(root).despawn();
    }
    let player = selected.grid_slot - 1;
    let mut camera = ChaseCamera::new(&race, player);
    camera.select(selected.camera);
    camera.configure(
        frontend
            .views
            .get(&selected.car)
            .cloned()
            .unwrap_or_else(|| crate::camera::Tuning::for_car(&selected.car)),
    );
    let sun = race.track.sun_direction;
    for mut light in &mut lights {
        *light = Transform::IDENTITY
            .looking_to(Vec3::new(sun.x as f32, sun.y as f32, sun.z as f32), Vec3::Y);
    }
    for mut window in &mut windows {
        window.title = format!("F1 Racecraft - {}", race.track.name);
    }
    *session = Session {
        guidance: crate::driving::Guidance::new(&race, player),
        corner_braking: frontend.corner_braking,
        show_racing_line: frontend.show_racing_line,
        show_performance: session.show_performance,
        minimap: hud::Minimap::new(&race.track),
        poses: crate::pose::Poses::new(&race),
        race,
        player,
        camera,
        controls: default(),
        paused: true,
        messages: default(),
        delta: default(),
        ready: false,
        screen: Screen::Loading,
    };
    *options = selected;
    frontend.error = None;
    frontend.race_started = false;
    assets::spawn_visuals(&mut commands, &server, &options, &session);
}

const RED: Color32 = Color32::from_rgb(230, 35, 48);
const DIM: Color32 = Color32::from_rgb(175, 185, 199);

pub fn ui(
    mut contexts: EguiContexts,
    mut frontend: ResMut<Frontend>,
    mut session: ResMut<Session>,
    mut options: ResMut<Options>,
    mut exit: MessageWriter<AppExit>,
    mut styled: Local<bool>,
    mut warmed: Local<u8>,
) -> Result {
    let context = contexts.ctx_mut()?;
    if !*styled {
        let mut style = (*context.style_of(egui::Theme::Dark)).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.selection.bg_fill = RED;
        style.visuals.widgets.active.bg_fill = RED;
        style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(65, 72, 84);
        style.spacing.item_spacing = egui::vec2(14., 7.);
        style.spacing.button_padding = egui::vec2(14., 5.);
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(17.));
        style
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(17.));
        context.set_theme(egui::Theme::Dark);
        context.set_style_of(egui::Theme::Dark, style);
        *styled = true;
    }
    // Warm both font sets: the HUD installs its definitions on the first pass.
    // Keeping the atlas stable avoids missing glyphs on a panel's first frame.
    if *warmed == 1 {
        warm_fonts(context);
    }
    if *warmed < 2 {
        *warmed += 1;
    }
    let mut viewport = egui::Ui::new(
        context.clone(),
        "menu-viewport".into(),
        egui::UiBuilder::new()
            .layer_id(egui::LayerId::background())
            .max_rect(context.viewport_rect()),
    );
    if session.screen == Screen::Racing {
        frontend.draft.camera = session.camera.mode;
        let scale = context.viewport_rect().height() / 1080.;
        // Match the timing tower's header, entrant rows and three timing rows.
        let below_timing =
            (24. + 45. + session.race.entrants.len() as f32 * 30. + 10. + 90. + 10.) * scale;
        egui::Area::new(egui::Id::new("race-controls"))
            .anchor(
                egui::Align2::LEFT_TOP,
                egui::vec2(24. * scale, below_timing),
            )
            .show(&viewport, |ui| {
                egui::Frame::new()
                    .fill(Color32::from_black_alpha(200))
                    .inner_margin(6)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if ui
                                .button(format!("{}  C", session.camera.mode.label()))
                                .clicked()
                            {
                                session.camera.cycle();
                            }
                            if ui.button("Adjust  F4").clicked() {
                                session.screen = Screen::CameraEditor;
                                session.paused = true;
                            }
                            if ui.button("Menu  Esc").clicked() {
                                session.screen = Screen::Paused;
                                session.paused = true;
                            }
                        });
                    });
            });
        return Ok(());
    }
    if session.screen == Screen::CameraEditor {
        camera_editor(&viewport, &mut session, &options.car);
        return Ok(());
    }
    if session.screen == Screen::Loading {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(Color32::from_black_alpha(220)))
            .show(&mut viewport, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(ui.available_height() * 0.4);
                    ui.spinner();
                    ui.heading("Loading circuit and cars");
                    ui.label("Preparing the scene. Your race starts when every car is ready.");
                });
            });
        return Ok(());
    }
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(Color32::from_rgb(9, 13, 20))
                .inner_margin(28),
        )
        .show(&mut viewport, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(if session.screen == Screen::Paused {
                        "Race paused"
                    } else {
                        "F1 Racecraft"
                    })
                    .size(36.)
                    .strong(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if frontend.race_started && session.ready && ui.button("Resume race").clicked()
                    {
                        resume(&mut session);
                    }
                    if ui.button("Quit game").clicked() {
                        exit.write(AppExit::Success);
                    }
                });
            });
            ui.label(RichText::new("Choose your circuit, car and driving aids.").color(DIM));
            ui.add_space(8.);
            ui.horizontal(|ui| {
                for (tab, label) in [
                    (Tab::Race, "Race setup"),
                    (Tab::Aids, "Driving aids"),
                    (Tab::Controls, "Cameras & controls"),
                    (Tab::Graphics, "Graphics"),
                ] {
                    let _response = ui.selectable_value(&mut frontend.tab, tab, label);
                    #[cfg(test)]
                    frontend.widgets.insert(label, _response.rect);
                }
            });
            ui.separator();
            if let Some(error) = &frontend.error {
                ui.colored_label(Color32::from_rgb(255, 150, 150), error);
            }
            let footer_height = 70.;
            egui::ScrollArea::vertical()
                .max_height((ui.available_height() - footer_height).max(100.))
                .show(ui, |ui| match frontend.tab {
                    Tab::Race => race_setup(ui, &mut frontend),
                    Tab::Aids => {
                        let _presets = aids_panel(ui, &mut frontend);
                        #[cfg(test)]
                        for (name, rect) in ["beginner", "balanced", "unassisted"]
                            .into_iter()
                            .zip(_presets)
                        {
                            frontend.widgets.insert(name, rect);
                        }
                    }
                    Tab::Controls => controls_panel(ui, &mut frontend.draft.camera, &mut session),
                    Tab::Graphics => graphics_panel(ui, &mut frontend.graphics, &mut session),
                });
            ui.add_space(12.);
            ui.separator();
            ui.horizontal(|ui| {
                if session.screen == Screen::Paused && ui.add(
                    egui::Button::new("Resume race").fill(RED).min_size(egui::vec2(180., 44.))
                ).clicked() { resume(&mut session); }
                let available =
                    frontend.catalog.circuits.iter().any(|track| {
                        track.id == frontend.draft.track && track.unavailable.is_none()
                    }) && frontend
                        .catalog
                        .cars
                        .iter()
                        .any(|car| car.id == frontend.draft.car);
                let start = ui.add_enabled(
                    available && !frontend.pending,
                    egui::Button::new(
                        RichText::new(if frontend.race_started {
                            "Start new race"
                        } else {
                            "Start race"
                        })
                        .strong(),
                    )
                    .fill(if frontend.race_started { Color32::from_rgb(45, 52, 64) } else { RED })
                    .min_size(egui::vec2(220., 44.)),
                );
                #[cfg(test)]
                frontend.widgets.insert("start", start.rect);
                if start.clicked() {
                    frontend.pending = true;
                }
                ui.label(
                    RichText::new(if session.screen == Screen::Paused {
                        "A new race replaces this session. Driving aids apply immediately."
                    } else {
                        "Beginner aids include corner braking, steering guidance and automatic gears."
                    })
                    .color(DIM),
                );
            });
        });
    if frontend.race_started {
        let player = session.player;
        session.race.entrants[player].car.assists = frontend.aids;
        options.driver_aids = Some(frontend.aids);
        options.automatic_gears = frontend.aids.automatic_gears;
        options.corner_braking = frontend.corner_braking;
        session.corner_braking = frontend.corner_braking;
        session.show_racing_line = frontend.show_racing_line;
    }
    Ok(())
}

fn race_setup(ui: &mut egui::Ui, frontend: &mut Frontend) {
    if ui.available_width() > 850. {
        ui.columns(2, |columns| {
            selection(&mut columns[0], &frontend.catalog, &mut frontend.draft);
            preview(&mut columns[1], &frontend.catalog, &frontend.draft);
        });
    } else {
        selection(ui, &frontend.catalog, &mut frontend.draft);
        ui.separator();
        preview(ui, &frontend.catalog, &frontend.draft);
    }
}

fn selection(ui: &mut egui::Ui, catalog: &Catalog, draft: &mut Options) {
    ui.heading("Circuit");
    let selected = catalog
        .circuits
        .iter()
        .find(|track| track.id == draft.track)
        .map_or("Select a circuit", |track| track.name.as_str());
    egui::ComboBox::from_id_salt("circuit")
        .width(ui.available_width())
        .selected_text(selected)
        .show_ui(ui, |ui| {
            for track in &catalog.circuits {
                ui.add_enabled_ui(track.unavailable.is_none(), |ui| {
                    ui.selectable_value(&mut draft.track, track.id.clone(), &track.name);
                });
                if let Some(reason) = &track.unavailable {
                    ui.small(reason);
                }
            }
        });
    ui.add_space(4.);
    ui.heading("Car");
    for car in &catalog.cars {
        ui.radio_value(&mut draft.car, car.id.clone(), car.name);
    }
    if let Some(car) = catalog.cars.iter().find(|car| car.id == draft.car) {
        ui.label(RichText::new(car.description).color(DIM));
    }
    ui.separator();
    ui.add(egui::Slider::new(&mut draft.laps, 1..=30).text("Laps"));
    ui.add(egui::Slider::new(&mut draft.opponents, 0..=15).text("AI opponents"));
    draft.grid_slot = draft.grid_slot.clamp(1, draft.opponents + 1);
    ui.add(egui::Slider::new(&mut draft.grid_slot, 1..=draft.opponents + 1).text("Grid position"));
    ui.horizontal(|ui| {
        ui.label("Tyres");
        for (compound, label) in [
            (Compound::Soft, "Soft"),
            (Compound::Medium, "Medium"),
            (Compound::Hard, "Hard"),
        ] {
            ui.selectable_value(&mut draft.setup.compound, compound, label);
        }
    });
    ui.add(
        egui::Slider::new(&mut draft.setup.front_wing, 0.0..=10.0)
            .step_by(1.)
            .text("Front wing"),
    );
    ui.add(
        egui::Slider::new(&mut draft.setup.rear_wing, 0.0..=10.0)
            .step_by(1.)
            .text("Rear wing"),
    );
}

fn preview(ui: &mut egui::Ui, catalog: &Catalog, draft: &Options) {
    let Some(track) = catalog
        .circuits
        .iter()
        .find(|track| track.id == draft.track)
    else {
        ui.label("No converted circuits found in the asset directory.");
        return;
    };
    ui.heading(&track.name);
    ui.label(
        RichText::new(format!(
            "{:.2} km / {} laps / {} cars",
            track.length / 1000.,
            draft.laps,
            draft.opponents + 1
        ))
        .color(DIM),
    );
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 240.), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, 8., Color32::from_rgb(17, 23, 32));
    if !track.outline.is_empty() {
        let (mut min, mut max) = (
            egui::pos2(f32::MAX, f32::MAX),
            egui::pos2(f32::MIN, f32::MIN),
        );
        for point in &track.outline {
            min.x = min.x.min(point[0] as f32);
            min.y = min.y.min(point[2] as f32);
            max.x = max.x.max(point[0] as f32);
            max.y = max.y.max(point[2] as f32);
        }
        let bounds = max - min;
        let scale =
            ((rect.width() - 40.) / bounds.x.max(1.)).min((rect.height() - 40.) / bounds.y.max(1.));
        let points: Vec<_> = track
            .outline
            .iter()
            .step_by(3)
            .map(|p| {
                rect.center()
                    + egui::vec2(
                        (p[0] as f32 - (min.x + max.x) * 0.5) * scale,
                        (p[2] as f32 - (min.y + max.y) * 0.5) * scale,
                    )
            })
            .collect();
        if let Some(start) = points.first() {
            ui.painter().circle_filled(*start, 5., RED);
        }
        ui.painter().add(egui::Shape::closed_line(
            points,
            egui::Stroke::new(3., Color32::from_rgb(217, 225, 238)),
        ));
    }
    ui.add_space(12.);
    ui.heading("Drive your way");
    ui.label("Choose individual driving aids or a preset in Driving aids. You can adjust them from the pause menu without restarting.");
    ui.label(
        RichText::new("C cycles cameras / 1 cockpit / 2 TV / 3 third person / Esc menu").color(DIM),
    );
}

fn aids_panel(ui: &mut egui::Ui, frontend: &mut Frontend) -> [egui::Rect; 3] {
    ui.heading("Driving aids");
    ui.label("Hold the accelerator; guidance helps steer and brake for corners. Manual steering overrides it.");
    let aids = &mut frontend.aids;
    let presets = ui
        .horizontal(|ui| {
            let beginner = ui.button("Beginner");
            if beginner.clicked() {
                frontend.corner_braking = 1.;
                frontend.show_racing_line = true;
                *aids = Assists {
                    steering: 1.,
                    braking: 1.,
                    traction: 1.,
                    stability: 1.,
                    automatic_gears: true,
                };
            }
            let balanced = ui.button("Balanced");
            if balanced.clicked() {
                frontend.corner_braking = 0.6;
                frontend.show_racing_line = true;
                *aids = Assists {
                    steering: 0.6,
                    braking: 0.6,
                    traction: 0.6,
                    stability: 0.5,
                    automatic_gears: true,
                };
            }
            let unassisted = ui.button("Unassisted");
            if unassisted.clicked() {
                frontend.corner_braking = 0.;
                frontend.show_racing_line = false;
                *aids = Assists {
                    steering: 0.,
                    braking: 0.,
                    traction: 0.,
                    stability: 0.,
                    automatic_gears: false,
                };
            }
            [beginner.rect, balanced.rect, unassisted.rect]
        })
        .inner;
    ui.separator();
    ui.horizontal(|ui| {
        ui.checkbox(&mut aids.automatic_gears, "Automatic gears")
            .on_hover_text("Manual shifts: E / Shift up, Q / Ctrl down.");
        ui.checkbox(&mut frontend.show_racing_line, "Racing line");
    });
    ui.add_space(8.);
    ui.columns(2, |columns| {
        columns[0].heading("Guidance");
        columns[0].add(
            egui::Slider::new(&mut frontend.corner_braking, 0.0..=1.0)
                .text("Corner braking")
                .custom_formatter(|value, _| format!("{:.0}%", value * 100.)),
        );
        columns[0].label(
            RichText::new("Brakes for bends and reduces a held accelerator.")
                .size(15.)
                .color(DIM),
        );
        columns[1].heading("Car control");
        for (value, label, help) in [
            (
                &mut aids.steering,
                "Steering assistance",
                "Guides towards the racing line. Strong manual steering takes priority.",
            ),
            (
                &mut aids.braking,
                "Anti-lock braking",
                "Helps prevent wheel lock-ups while braking.",
            ),
            (
                &mut aids.traction,
                "Traction control",
                "Reduces wheelspin when applying power.",
            ),
            (
                &mut aids.stability,
                "Stability assistance",
                "Helps recover a developing slide.",
            ),
        ] {
            let ui = if label == "Steering assistance" {
                &mut columns[0]
            } else {
                &mut columns[1]
            };
            ui.add(
                egui::Slider::new(value, 0.0..=1.0)
                    .step_by(0.05)
                    .text(label)
                    .custom_formatter(|value, _| format!("{:.0}%", value * 100.)),
            );
            ui.label(RichText::new(help).size(15.).color(DIM));
        }
    });
    presets
}

fn camera_editor(viewport: &egui::Ui, session: &mut Session, car: &str) {
    egui::Area::new("camera-editor".into())
        .anchor(egui::Align2::RIGHT_CENTER, egui::vec2(-18., 0.))
        .show(viewport, |ui| {
            egui::Frame::new()
                .fill(Color32::from_black_alpha(235))
                .inner_margin(18)
                .show(ui, |ui| {
                    ui.set_width(330.);
                    ui.heading("Camera position");
                    ui.label("Live preview / saved for this car");
                    ui.horizontal(|ui| {
                        for mode in Mode::ALL {
                            if ui
                                .selectable_label(session.camera.mode == mode, mode.label())
                                .clicked()
                            {
                                session.camera.select(mode);
                            }
                        }
                    });
                    let mut tuning = session.camera.tuning.clone();
                    let view = tuning.view_mut(session.camera.mode);
                    ui.add(egui::Slider::new(&mut view.height, 0.65..=4.).text("Height"));
                    let range = if session.camera.mode == Mode::Chase {
                        -10.0..=-3.5
                    } else {
                        -1.5..=0.7
                    };
                    ui.add(egui::Slider::new(&mut view.fore_aft, range).text("Forward / back"));
                    ui.add(egui::Slider::new(&mut view.pitch, -10.0..=20.).text("Look down"));
                    ui.add(egui::Slider::new(&mut view.fov, 40.0..=95.).text("Field of view"));
                    ui.separator();
                    ui.add(egui::Slider::new(&mut tuning.shake, 0.0..=1.).text("Vibration"));
                    ui.add(egui::Slider::new(&mut tuning.horizon, 0.0..=1.).text("Stable horizon"));
                    if ui.button("Reset this view").clicked() {
                        *tuning.view_mut(session.camera.mode) =
                            crate::camera::Tuning::for_car(car).view(session.camera.mode);
                    }
                    session.camera.configure(tuning);
                    ui.separator();
                    if ui
                        .button(RichText::new("Done / resume  F4 or Esc").strong())
                        .clicked()
                    {
                        resume(session);
                    }
                });
        });
}

fn warm_fonts(context: &egui::Context) {
    let glyphs: String = (32u8..=126).map(char::from).chain("°·—ü".chars()).collect();
    let mut fonts: Vec<_> = context
        .style_of(egui::Theme::Dark)
        .text_styles
        .values()
        .cloned()
        .collect();
    fonts.extend([15., 36.].map(egui::FontId::proportional));
    let scale = context.viewport_rect().height() / 1080.;
    for family in [
        egui::FontFamily::Name("regular".into()),
        egui::FontFamily::Name("bold".into()),
    ] {
        for size in [
            12., 14., 16., 18., 20., 22., 24., 26., 28., 30., 32., 34., 36., 40., 44., 48., 52.,
            56., 64., 68., 72., 80., 96.,
        ] {
            fonts.push(egui::FontId::new(size * scale, family.clone()));
        }
    }
    context.fonts_mut(|atlas| {
        for font in fonts {
            atlas.layout_no_wrap(glyphs.clone(), font, Color32::WHITE);
        }
    });
}

fn graphics_panel(
    ui: &mut egui::Ui,
    selected: &mut crate::graphics::Quality,
    session: &mut Session,
) {
    ui.heading("Graphics preset");
    ui.label("Changes apply immediately.");
    ui.add_space(6.);
    for quality in crate::graphics::Quality::ALL {
        ui.radio_value(selected, quality, quality.label());
        ui.label(RichText::new(quality.description()).size(15.).color(DIM));
    }
    ui.add_space(8.);
    ui.checkbox(&mut session.show_performance, "Show frame rate  F3");
}

fn controls_panel(ui: &mut egui::Ui, selected: &mut Mode, session: &mut Session) {
    ui.heading("Camera");
    for mode in Mode::ALL {
        if ui.radio_value(selected, mode, mode.label()).changed() {
            session.camera.select(mode);
        }
    }
    if session.ready && ui.button("Adjust camera in scene  F4").clicked() {
        session.screen = Screen::CameraEditor;
        session.paused = true;
    }
    ui.separator();
    egui::Grid::new("controls-grid")
        .num_columns(3)
        .spacing([28., 12.])
        .show(ui, |ui| {
            for row in [
                ["Action", "Keyboard", "Gamepad"],
                ["Throttle / brake", "W / S or arrows", "RT / LT"],
                ["Steer", "A / D or arrows", "Left stick"],
                ["Shift up / down", "E / Q", "RB / LB"],
                ["Cycle camera", "C (or 1 / 2 / 3)", "Y"],
                ["Adjust camera", "F4", "-"],
                ["Pause / resume", "Esc or P", "Start"],
                ["Recover car", "R", "Back / Select"],
                ["Mute / unmute", "M", "-"],
            ] {
                for cell in row {
                    ui.label(cell);
                }
                ui.end_row();
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_distinguishes_complete_incomplete_and_deferred_tracks() {
        let root = std::env::temp_dir().join(format!(
            "f1-catalog-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for (directory, complete) in [
            ("tracks/ready", true),
            ("tracks/missing", false),
            ("deferred/needs-repair", true),
        ] {
            let path = root.join(directory);
            std::fs::create_dir_all(&path).unwrap();
            let track = serde_json::json!({"name":directory, "centerline":vec![[0.,0.,0.];8]});
            std::fs::write(path.join("track.json"), track.to_string()).unwrap();
            if complete {
                std::fs::write(path.join("visual.glb"), []).unwrap();
                std::fs::write(path.join("collision.bin"), []).unwrap();
            }
        }
        let catalog = Catalog::discover(&root);
        assert_eq!(catalog.circuits.len(), 3);
        assert_eq!(
            catalog
                .circuits
                .iter()
                .filter(|track| track.unavailable.is_none())
                .count(),
            1
        );
        assert_eq!(catalog.circuits[0].id, "ready");
        assert!(catalog
            .circuits
            .iter()
            .find(|track| track.id == "needs-repair")
            .unwrap()
            .unavailable
            .as_ref()
            .unwrap()
            .contains("deferred"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
