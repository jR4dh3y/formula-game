//! Graphical lobby and the bridge from authoritative networking into the existing Session.
use crate::{
    menu::{section, setting_row, Frontend, Screen, Tab, DIM, HOUSING, LABEL, RED, WELL, WELL_RIM},
    options::Options,
    pose::Poses,
    Session,
};
use bevy::prelude::*;
use bevy_egui::egui::{self, Color32, RichText};
use racecraft_net::{
    Client, ClientMessage, Endpoint, Lobby, RaceSettings, Server, ServerHandle, Stage, MAX_PLAYERS,
};
use std::{net::SocketAddr, time::Duration};

#[derive(Resource)]
pub struct Online {
    pub client: Option<Client>,
    host: Option<ServerHandle>,
    open: bool,
    identity: Option<racecraft_net::DriverIdentity>,
    name: String,
    address: String,
    port: u16,
    settings: RaceSettings,
    draft_owner: Option<u64>,
    pub error: Option<String>,
    install: Option<Lobby>,
    loading: Option<u64>,
    last_event: u64,
    request: Option<bool>,
    pub pilot: Option<racecraft_sim::AiDriver>,
    test_started: bool,
    page: Page,
    intent: Intent,
}
impl Online {
    pub fn new(options: &Options) -> Self {
        Self {
            pilot: None,
            test_started: false,
            page: Page::Connect,
            intent: Intent::default(),
            client: None,
            host: None,
            open: options.host.is_some() || options.join.is_some(),
            identity: if !options.team.is_empty() && !options.driver.is_empty() {
                Some(racecraft_net::DriverIdentity {
                    team: options.team.clone(),
                    driver: options.driver.clone(),
                })
            } else {
                None
            },
            name: options.name.clone(),
            address: options
                .join
                .clone()
                .unwrap_or_else(|| "127.0.0.1:7777".into()),
            port: options.host.unwrap_or(7777),
            draft_owner: None,
            settings: RaceSettings {
                track: options.track.clone(),
                car: options.car.clone(),
                laps: options.laps,
                ai_cars: options.ai_cars,
                ai_power: options.ai_power,
                setup: options.setup,
                assists: options.player_assists(),
                corner_braking: options.corner_braking,
                ..RaceSettings::default()
            },
            error: None,
            install: None,
            loading: None,
            last_event: 0,
            request: if options.host.is_some() {
                Some(true)
            } else if options.join.is_some() {
                Some(false)
            } else {
                None
            },
        }
    }
    fn begin(
        &mut self,
        host: bool,
        options: &Options,
        aids: racecraft_sim::Assists,
    ) -> Result<(), String> {
        if self.name.trim().is_empty() || self.name.len() > 32 {
            return Err("Choose a driver name of 1–32 bytes".into());
        }
        let id = racecraft_net::random_id();
        let address = if host {
            // Dual-stack where available, so IPv6 friends can join too; plain
            // IPv4 otherwise (no IPv6 on this machine, or the OS refuses).
            let bind = |address: SocketAddr| {
                Server::bind(&options.assets, address, self.settings.clone(), Some(id))
            };
            let server = bind(SocketAddr::from((
                std::net::Ipv6Addr::UNSPECIFIED,
                self.port,
            )))
            .or_else(|_| bind(SocketAddr::from(([0, 0, 0, 0], self.port))))?;
            self.host = Some(ServerHandle::spawn(server));
            Endpoint::Udp(SocketAddr::from(([127, 0, 0, 1], self.port)))
        } else {
            self.address.parse::<Endpoint>()?
        };
        match Client::connect(
            &options.assets,
            address,
            self.name.trim().to_owned(),
            aids,
            Some(id),
        ) {
            Ok(mut client) => {
                client.identity = self.identity.clone();
                client.observe = options.test_driver;
                self.client = Some(client);
            }
            Err(error) => {
                self.host = None;
                return Err(error);
            }
        }
        self.error = None;
        self.last_event = 0;
        Ok(())
    }
    fn leave(&mut self, session: &mut Session) {
        self.pilot = None;
        self.test_started = false;
        self.client = None;
        self.host = None;
        self.draft_owner = None;
        self.install = None;
        self.loading = None;
        session.paused = true;
        session.screen = Screen::Main;
        session.controls = default();
        session.pings.clear();
    }
    /// Switch the menu to multiplayer.
    pub fn show(&mut self) {
        self.open = true;
    }
    /// Switch the menu back to solo; only possible outside a room.
    pub fn close(&mut self) {
        if self.client.is_none() {
            self.open = false;
            self.error = None;
        }
    }
    /// The menu is in multiplayer mode.
    pub fn active(&self) -> bool {
        self.client.is_some() || self.open
    }
    pub fn in_room(&self) -> bool {
        self.client.is_some()
    }
    /// Results take the screen over a finished race even while still "driving".
    pub fn owns_racing_screen(&self) -> bool {
        self.client
            .as_ref()
            .is_some_and(|c| c.active() && c.stage() == Stage::Results)
    }
    /// Refresh the page from the network once per menu frame.
    pub fn prepare(&mut self) {
        if let Some(error) = self.client.as_mut().and_then(|c| c.error.take()) {
            self.error = Some(error);
        }
        self.page = match &self.client {
            None => Page::Connect,
            Some(client) => match &client.lobby {
                None => Page::Connecting,
                Some(lobby) => match lobby.stage {
                    Stage::Lobby => Page::Lobby(lobby.clone()),
                    Stage::Loading => Page::Loading(lobby.clone()),
                    Stage::Grid | Stage::Racing => Page::Live(lobby.clone()),
                    Stage::Results | Stage::Closed => Page::Results(lobby.clone()),
                },
            },
        };
        if let Page::Lobby(lobby) = &self.page {
            let owner = self.client.as_ref().is_some_and(|c| c.owner());
            // Owners edit a draft; everyone else always sees the room's rules.
            if self.draft_owner != Some(lobby.owner) || !owner {
                self.settings = lobby.settings.clone();
                self.draft_owner = Some(lobby.owner);
            }
        }
    }
    /// Page heading and description for a multiplayer tab.
    pub fn heading(&self, tab: Tab) -> (String, String) {
        match tab {
            Tab::Join => (
                "Join room".into(),
                "Enter the host's LAN or VPN address, or a hosted server URL.".into(),
            ),
            Tab::Host => (
                "Host room".into(),
                format!(
                    "Race up to {MAX_PLAYERS} drivers. Others join with your LAN or VPN address."
                ),
            ),
            _ => {
                let (title, subtitle) = self.page.heading(self.host.is_some());
                (title.into(), subtitle)
            }
        }
    }
    pub fn join_tab(&mut self, ui: &mut egui::Ui, frontend: &mut Frontend) {
        self.driver_section(ui, frontend);
        ui.separator();
        section(ui, "Room");
        field(ui, "Address", |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.address)
                    .hint_text("192.168.1.20:7777 or wss://…")
                    .desired_width(ui.available_width()),
            );
        });
    }
    pub fn host_tab(&mut self, ui: &mut egui::Ui, frontend: &mut Frontend) {
        self.settings.ai_power = frontend.draft.ai_power;
        self.driver_section(ui, frontend);
        ui.separator();
        section(ui, "Room");
        crate::menu::circuit_choice(ui, &frontend.catalog, &mut frontend.draft);
        self.settings.track = frontend.draft.track.clone();
        field(ui, "Car", |ui| {
            egui::ComboBox::from_id_salt("online-car")
                .width(ui.available_width())
                .selected_text(car_name(&self.settings.car))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.settings.car, "rb6".into(), "RB6");
                    ui.selectable_value(&mut self.settings.car, "gen22".into(), "GEN22");
                });
        });
        field(ui, "UDP port", |ui| {
            ui.add(egui::DragValue::new(&mut self.port).range(1024..=65535));
        });
        ui.separator();
        section(ui, "Race rules");
        rules(ui, &mut self.settings, MAX_PLAYERS - 1);
        frontend.draft.ai_power = self.settings.ai_power;
    }
    /// The room tab: connecting, lobby, loading, live standings or results.
    pub fn room_tab(&mut self, ui: &mut egui::Ui, frontend: &Frontend, session: &Session) {
        let mut intent = std::mem::take(&mut self.intent);
        let page = std::mem::replace(&mut self.page, Page::Connect);
        match &page {
            Page::Connect => {}
            Page::Connecting => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Connecting and checking that your assets match the room…");
                });
            }
            Page::Lobby(lobby) => self.lobby_page(ui, lobby, frontend, &mut intent),
            Page::Loading(lobby) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(
                        "Loading the circuit and cars. The race starts when every driver is ready.",
                    );
                });
                ui.separator();
                self.roster(ui, lobby, frontend, false, &mut intent);
            }
            Page::Live(lobby) => self.standings(ui, session, lobby, false),
            Page::Results(lobby) => self.standings(ui, session, lobby, true),
        }
        self.page = page;
        self.intent = intent;
    }
    /// Footer actions for the current page, before the menu's own Quit.
    pub fn actions(&mut self, ui: &mut egui::Ui, tab: Tab, session: &Session, size: egui::Vec2) {
        let mut intent = std::mem::take(&mut self.intent);
        let page = std::mem::replace(&mut self.page, Page::Connect);
        self.footer(ui, &page, tab, session, size, &mut intent);
        self.page = page;
        self.intent = intent;
    }
    /// Apply what the player asked for this frame.
    pub fn finish(&mut self, frontend: &Frontend, session: &mut Session, options: &Options) {
        let intent = std::mem::take(&mut self.intent);
        if let Some(host) = intent.start {
            self.identity = Some(racecraft_net::DriverIdentity {
                team: frontend.draft.team.clone(),
                driver: frontend.draft.driver.clone(),
            });
            if let Err(error) = self.begin(host, options, frontend.aids) {
                self.error = Some(error);
            }
        }
        if let Some(message) = intent.send {
            if let Some(client) = &mut self.client {
                if let Err(error) = client.send(message) {
                    self.error = Some(error);
                }
            }
        }
        if intent.resume {
            crate::menu::resume(session);
        }
        if intent.leave {
            self.leave(session);
        }
    }
    fn driver_section(&mut self, ui: &mut egui::Ui, frontend: &mut Frontend) {
        section(ui, "Your driver");
        field(ui, "Driver name", |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.name)
                    .char_limit(32)
                    .hint_text("Name shown to other drivers")
                    .desired_width(ui.available_width()),
            );
        });
        if let Ok((team, driver)) = frontend
            .teams
            .selection(&frontend.draft.team, &frontend.draft.driver)
        {
            ui.label(format!(
                "{} · #{} {}",
                team.name, driver.number, driver.name
            ));
        }
        if ui.button("Choose team & driver").clicked() {
            frontend.tab = Tab::Car;
        }
    }

    fn footer(
        &mut self,
        ui: &mut egui::Ui,
        page: &Page,
        tab: Tab,
        session: &Session,
        size: egui::Vec2,
        intent: &mut Intent,
    ) {
        let primary = |ui: &mut egui::Ui, text: &str, enabled: bool| {
            ui.add_enabled(
                enabled,
                egui::Button::new(RichText::new(text).strong())
                    .fill(RED)
                    .min_size(size),
            )
            .clicked()
        };
        let secondary = |ui: &mut egui::Ui, text: &str| {
            ui.add(
                egui::Button::new(RichText::new(text).strong())
                    .fill(HOUSING)
                    .min_size(size),
            )
            .clicked()
        };
        match page {
            // Like Start race offline: the main action stays on every page.
            Page::Connect => {
                let host = tab == Tab::Host;
                if primary(ui, if host { "Host room" } else { "Join room" }, true) {
                    intent.start = Some(host);
                }
            }
            Page::Lobby(lobby) => {
                let Some(client) = &self.client else { return };
                let ready = lobby
                    .players
                    .iter()
                    .find(|p| p.id == client.id)
                    .is_some_and(|p| p.ready);
                let toggle = ui.add(
                    egui::Button::new(
                        RichText::new(if ready { "Not ready" } else { "Ready" }).strong(),
                    )
                    .fill(if ready { HOUSING } else { RED })
                    .min_size(size),
                );
                if toggle.clicked() {
                    intent.send = Some(ClientMessage::Ready(!ready));
                }
                if client.owner() {
                    let unapplied = self.settings != lobby.settings;
                    if primary(ui, "Start race", can_start(lobby, unapplied)) {
                        intent.send = Some(ClientMessage::Start);
                    }
                }
                if secondary(ui, "Leave room") {
                    intent.leave = true;
                }
            }
            Page::Live(_) => {
                if session.screen != Screen::Racing && primary(ui, "Resume driving", session.ready)
                {
                    intent.resume = true;
                }
                if secondary(ui, "Leave race") {
                    intent.leave = true;
                }
            }
            Page::Results(_) => {
                if self.client.as_ref().is_some_and(|c| c.owner())
                    && primary(ui, "Back to lobby", true)
                {
                    intent.send = Some(ClientMessage::Return);
                }
                if secondary(ui, "Leave room") {
                    intent.leave = true;
                }
            }
            Page::Connecting | Page::Loading(_) => {
                if secondary(ui, "Leave room") {
                    intent.leave = true;
                }
            }
        }
    }

    fn lobby_page(
        &mut self,
        ui: &mut egui::Ui,
        lobby: &Lobby,
        frontend: &Frontend,
        intent: &mut Intent,
    ) {
        let owner = self.client.as_ref().is_some_and(|c| c.owner());
        let wide = ui.available_width() >= 900.;
        let draw_rules = |ui: &mut egui::Ui, this: &mut Self, intent: &mut Intent| {
            card(ui, "Race rules", |ui| {
                if owner {
                    let before = lobby.settings.clone();
                    let mut settings = this.settings.clone();
                    rules(ui, &mut settings, MAX_PLAYERS - lobby.players.len());
                    ui.add_space(4.);
                    if ui.button("Reset to defaults").clicked() {
                        settings = RaceSettings {
                            car: lobby.settings.car.clone(),
                            track: lobby.settings.track.clone(),
                            ..RaceSettings::default()
                        };
                        intent.send = Some(ClientMessage::Settings(settings.clone()));
                    }
                    // The controls edit a draft; the room changes only on Apply.
                    if settings != before && intent.send.is_none() {
                        ui.add_space(6.);
                        notice(
                            ui,
                            "Changes are not applied yet. Applying clears everyone's ready state.",
                            AMBER,
                        );
                        ui.horizontal_wrapped(|ui| {
                            if ui
                                .add(egui::Button::new(RichText::new("Apply").strong()).fill(RED))
                                .clicked()
                            {
                                intent.send = Some(ClientMessage::Settings(settings.clone()));
                            }
                            if ui.button("Discard").clicked() {
                                settings = before.clone();
                            }
                        });
                    }
                    this.settings = settings;
                } else {
                    let s = &lobby.settings;
                    egui::Grid::new("online-rules-summary")
                        .num_columns(2)
                        .spacing([24., 10.])
                        .show(ui, |ui| {
                            summary(ui, "Laps", s.laps.to_string());
                            summary(ui, "AI drivers", s.ai_cars.to_string());
                            summary(ui, "AI engine power", format!("{:.0}%", s.ai_power * 100.));
                            summary(
                                ui,
                                "Driving aids",
                                if s.lock_aids {
                                    format!("Host's · {:.0}%", s.assists.steering * 100.)
                                } else {
                                    "Each driver's own".into()
                                },
                            );
                            summary(ui, "Automatic gears", yes(s.assists.automatic_gears));
                            summary(ui, "DRS anywhere", yes(s.assists.drs_anywhere));
                            summary(ui, "Tyres", format!("{:?}", s.setup.compound));
                            summary(
                                ui,
                                "Wings F / R",
                                format!("{:.1} / {:.1}", s.setup.front_wing, s.setup.rear_wing),
                            );
                        });
                    ui.add_space(8.);
                    ui.label(
                        RichText::new("Only the room owner can change the rules.").color(LABEL),
                    );
                }
            });
        };
        let address = self.host.is_some().then_some(self.port);
        let draw_grid = |ui: &mut egui::Ui, this: &mut Self, intent: &mut Intent| {
            if let Some(port) = address {
                notice(
                    ui,
                    &format!("Hosting on UDP port {port}. Share your LAN or VPN IP address."),
                    DIM,
                );
                ui.add_space(8.);
            }
            this.roster(ui, lobby, frontend, owner, intent);
        };
        if wide {
            ui.columns(2, |columns| {
                draw_grid(&mut columns[0], self, intent);
                draw_rules(&mut columns[1], self, intent);
            });
        } else {
            draw_grid(ui, self, intent);
            ui.separator();
            draw_rules(ui, self, intent);
        }
    }

    /// The room's drivers in grid order, with readiness and ping.
    fn roster(
        &self,
        ui: &mut egui::Ui,
        lobby: &Lobby,
        frontend: &Frontend,
        reorder: bool,
        intent: &mut Intent,
    ) {
        let Some(client) = &self.client else { return };
        let ready = lobby.players.iter().filter(|p| p.ready).count();
        card(
            ui,
            &format!(
                "Grid · {} of {} ready · {} cars",
                ready,
                lobby.players.len(),
                lobby.players.len() + lobby.settings.ai_cars
            ),
            |ui| {
                ui.spacing_mut().item_spacing.y = 6.;
                let ids: Vec<_> = lobby.players.iter().map(|p| p.id).collect();
                for (slot, player) in lobby.players.iter().enumerate() {
                    let status = if !player.connected {
                        ("AI TAKEOVER", AMBER)
                    } else if lobby.stage != Stage::Lobby {
                        ("IN ROOM", DIM)
                    } else if player.ready {
                        ("READY", GREEN)
                    } else {
                        ("WAITING", LABEL)
                    };
                    let mut badges = Vec::new();
                    if player.id == lobby.owner {
                        badges.push(("HOST", RED));
                    }
                    if player.id == client.id {
                        badges.push(("YOU", DIM));
                    }
                    let row = Row {
                        position: slot + 1,
                        name: &player.name,
                        subtitle: player
                            .identity
                            .as_ref()
                            .map(|i| identity_label(frontend, i)),
                        team: player
                            .identity
                            .as_ref()
                            .and_then(|i| team_color(frontend, &i.team))
                            .unwrap_or(LABEL),
                        badges,
                        status: Some(status),
                        detail: None,
                        ping: Some(client.ping_ms(player.id).filter(|_| player.connected)),
                        mine: player.id == client.id,
                    };
                    driver_row(ui, &row, |ui| {
                        if !reorder {
                            return;
                        }
                        if arrow(ui, false, slot + 1 < ids.len()) {
                            let mut ids = ids.clone();
                            ids.swap(slot, slot + 1);
                            intent.send = Some(ClientMessage::Grid(ids));
                        }
                        if arrow(ui, true, slot > 0) {
                            let mut ids = ids.clone();
                            ids.swap(slot, slot - 1);
                            intent.send = Some(ClientMessage::Grid(ids));
                        }
                    });
                }
                if lobby.settings.ai_cars > 0 {
                    egui::Frame::new()
                        .stroke(egui::Stroke::new(1., WELL_RIM))
                        .corner_radius(6)
                        .inner_margin(egui::Margin::symmetric(12, 8))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(
                                RichText::new(format!(
                                    "+ {} AI driver{} fill the rest of the grid",
                                    lobby.settings.ai_cars,
                                    if lobby.settings.ai_cars == 1 { "" } else { "s" }
                                ))
                                .color(LABEL),
                            );
                        });
                }
            },
        );
    }

    /// Race order with gaps (or final times) and each human driver's ping.
    fn standings(&self, ui: &mut egui::Ui, session: &Session, lobby: &Lobby, results: bool) {
        let Some(client) = &self.client else { return };
        let race = &session.race;
        let order = race.order();
        let leader = order.first().map(|&i| &race.entrants[i].timing);
        card(
            ui,
            if results {
                "Classification"
            } else {
                "Standings"
            },
            |ui| {
                ui.spacing_mut().item_spacing.y = 6.;
                for (rank, &index) in order.iter().enumerate() {
                    let entrant = &race.entrants[index];
                    let player = lobby.players.get(index);
                    let identity = session.identities.get(index).and_then(Option::as_ref);
                    let detail = if let Some(time) = entrant.timing.finish_time {
                        match leader.and_then(|l| l.finish_time) {
                            Some(first) if rank > 0 => format!("+{:.3}", time - first),
                            _ => crate::hud::format_time(time),
                        }
                    } else if rank == 0 {
                        format!("Lap {}", (entrant.timing.laps_completed + 1).min(race.laps))
                    } else {
                        let behind = leader.map_or(0., |l| l.distance) - entrant.timing.distance;
                        if behind > race.track.length {
                            format!("+{} lap", (behind / race.track.length) as u32)
                        } else {
                            format!("+{:.1}", behind / entrant.car.speed().max(30.))
                        }
                    };
                    let mut badges = Vec::new();
                    if player.is_none() {
                        badges.push(("AI", LABEL));
                    } else if player.is_some_and(|p| !p.connected) {
                        badges.push(("AI TAKEOVER", AMBER));
                    }
                    // Humans by their lobby name; AI cars by the driver they represent.
                    let name = match (player, identity) {
                        (Some(p), _) => p.name.as_str(),
                        (None, Some(identity)) => identity.name.as_str(),
                        (None, None) => entrant.name.as_str(),
                    };
                    let row = Row {
                        position: rank + 1,
                        name,
                        subtitle: player.and(identity).map(|i| i.name.clone()),
                        team: identity.map_or(LABEL, |i| {
                            let [r, g, b] = i.primary;
                            Color32::from_rgb(r, g, b)
                        }),
                        badges,
                        status: None,
                        detail: Some(detail),
                        ping: player.map(|p| client.ping_ms(p.id).filter(|_| p.connected)),
                        mine: index == session.player,
                    };
                    driver_row(ui, &row, |_| {});
                }
            },
        );
    }
}

enum Page {
    Connect,
    Connecting,
    Lobby(Lobby),
    Loading(Lobby),
    Live(Lobby),
    Results(Lobby),
}
impl Page {
    fn heading(&self, hosting: bool) -> (&'static str, String) {
        let room = |lobby: &Lobby| {
            format!(
                "{} · {} · {} lap{}",
                lobby.settings.track.to_uppercase(),
                car_name(&lobby.settings.car),
                lobby.settings.laps,
                if lobby.settings.laps == 1 { "" } else { "s" }
            )
        };
        match self {
            Self::Connect => ("Room", "You are not in a room.".into()),
            Self::Connecting => ("Joining room", "Waiting for the room to answer.".into()),
            Self::Lobby(lobby) => (
                if hosting { "Your room" } else { "Room lobby" },
                room(lobby),
            ),
            Self::Loading(lobby) => ("Loading race", room(lobby)),
            Self::Live(lobby) => ("Live race", room(lobby)),
            Self::Results(lobby) => ("Race results", room(lobby)),
        }
    }
}

#[derive(Default)]
struct Intent {
    start: Option<bool>,
    send: Option<ClientMessage>,
    leave: bool,
    resume: bool,
}

const GREEN: Color32 = Color32::from_rgb(64, 230, 115);
const AMBER: Color32 = Color32::from_rgb(255, 184, 38);

/// A titled group of settings, laid out like the menu's own sections.
fn card(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.vertical(|ui| {
        ui.set_width(ui.available_width());
        section(ui, title);
        add(ui);
    });
}

fn notice(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::new()
        .fill(WELL)
        .stroke(egui::Stroke::new(1., color.gamma_multiply(0.6)))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(RichText::new(text).color(color)).wrap());
        });
}

fn field(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    setting_row(ui, label, add);
}

/// One read-only rule as a two-column grid row.
fn summary(ui: &mut egui::Ui, label: &str, value: String) {
    ui.label(RichText::new(label).color(DIM));
    ui.label(RichText::new(value).strong());
    ui.end_row();
}

fn yes(on: bool) -> String {
    if on { "On" } else { "Off" }.into()
}

fn car_name(car: &str) -> &'static str {
    match car {
        "gen22" => "GEN22",
        "rb6" => "RB6",
        _ => "Car",
    }
}

fn team_color(frontend: &Frontend, team: &str) -> Option<Color32> {
    let team = frontend.teams.teams.iter().find(|t| t.id == team)?;
    let [r, g, b] = crate::teams::rgb(&team.colors.primary)?;
    Some(Color32::from_rgb(r, g, b))
}

fn identity_label(frontend: &Frontend, identity: &racecraft_net::DriverIdentity) -> String {
    frontend
        .teams
        .teams
        .iter()
        .find(|t| t.id == identity.team)
        .and_then(|team| {
            let driver = team.drivers.iter().find(|d| d.id == identity.driver)?;
            Some(format!("{} · {}", team.name, driver.name))
        })
        .unwrap_or_else(|| format!("{} · {}", identity.team, identity.driver))
}

/// Laps, AI field size and the shared aids, as setting rows.
fn rules(ui: &mut egui::Ui, settings: &mut RaceSettings, max_ai: usize) {
    field(ui, "Laps", |ui| {
        ui.add(egui::Slider::new(&mut settings.laps, 1..=100));
    });
    settings.ai_cars = settings.ai_cars.min(max_ai);
    field(ui, "AI drivers", |ui| {
        ui.add(egui::Slider::new(&mut settings.ai_cars, 0..=max_ai));
    });
    crate::menu::ai_power_slider(ui, &mut settings.ai_power);
    field(ui, "Lock aids", |ui| {
        ui.checkbox(&mut settings.lock_aids, "Everyone uses the host's aids");
    });
    egui::CollapsingHeader::new(RichText::new("Shared aids and setup").color(DIM))
        .id_salt("online-shared-rules")
        .show(ui, |ui| {
            let mut level = settings.assists.steering;
            field(ui, "Driving aids", |ui| {
                if ui.add(egui::Slider::new(&mut level, 0. ..=1.)).changed() {
                    settings.assists.steering = level;
                    settings.assists.braking = level;
                    settings.assists.traction = level;
                    settings.assists.stability = level;
                    settings.corner_braking = level;
                }
            });
            field(ui, "Automatic gears", |ui| {
                ui.checkbox(&mut settings.assists.automatic_gears, "");
            });
            field(ui, "DRS anywhere", |ui| {
                ui.checkbox(&mut settings.assists.drs_anywhere, "");
            });
            field(ui, "Front wing", |ui| {
                ui.add(egui::Slider::new(&mut settings.setup.front_wing, 0. ..=10.));
            });
            field(ui, "Rear wing", |ui| {
                ui.add(egui::Slider::new(&mut settings.setup.rear_wing, 0. ..=10.));
            });
            field(ui, "Tyres", |ui| {
                egui::ComboBox::from_id_salt("online-tyres")
                    .width(ui.available_width())
                    .selected_text(format!("{:?}", settings.setup.compound))
                    .show_ui(ui, |ui| {
                        for compound in [
                            racecraft_sim::Compound::Soft,
                            racecraft_sim::Compound::Medium,
                            racecraft_sim::Compound::Hard,
                        ] {
                            ui.selectable_value(
                                &mut settings.setup.compound,
                                compound,
                                format!("{compound:?}"),
                            );
                        }
                    });
            });
        });
}

struct Row<'a> {
    position: usize,
    name: &'a str,
    subtitle: Option<String>,
    team: Color32,
    badges: Vec<(&'static str, Color32)>,
    status: Option<(&'static str, Color32)>,
    detail: Option<String>,
    /// `None`: not a human driver. `Some(None)`: human with no measurement yet.
    ping: Option<Option<u16>>,
    mine: bool,
}

/// One driver as an inset row: position, team stripe and name on the left;
/// status, timing and ping in fixed right-hand columns.
fn driver_row(ui: &mut egui::Ui, row: &Row, controls: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(if row.mine {
            Color32::from_rgb(30, 33, 36)
        } else {
            WELL
        })
        .stroke(egui::Stroke::new(
            1.,
            if row.mine {
                RED.gamma_multiply(0.8)
            } else {
                WELL_RIM
            },
        ))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let narrow = ui.available_width() < 340.;
            let height = if row.subtitle.is_some() && !narrow {
                38.
            } else {
                28.
            };
            let size = egui::vec2(ui.available_width(), height);
            ui.allocate_ui_with_layout(
                size,
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 10.;
                    ping_cell(ui, row.ping);
                    if let Some(detail) = &row.detail {
                        ui.add_sized(
                            egui::vec2(if narrow { 64. } else { 86. }, 24.),
                            egui::Label::new(
                                RichText::new(detail)
                                    .strong()
                                    .color(egui::Color32::from_gray(225)),
                            ),
                        );
                    }
                    if let Some((text, color)) = row.status {
                        if !narrow {
                            pill(ui, text, color);
                        } else {
                            let (rect, _) =
                                ui.allocate_exact_size(egui::vec2(10., 10.), egui::Sense::hover());
                            ui.painter().circle_filled(rect.center(), 4., color);
                        }
                    }
                    controls(ui);
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 8.;
                        ui.add_sized(
                            egui::vec2(24., 24.),
                            egui::Label::new(
                                RichText::new(row.position.to_string())
                                    .size(18.)
                                    .strong()
                                    .color(if row.mine { Color32::WHITE } else { DIM }),
                            ),
                        );
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(4., 26.), egui::Sense::hover());
                        ui.painter().rect_filled(rect, 2., row.team);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.;
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 6.;
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(row.name).strong().color(Color32::WHITE),
                                    )
                                    .truncate(),
                                );
                                for &(text, color) in &row.badges {
                                    pill(ui, text, color);
                                }
                            });
                            if let Some(subtitle) = row.subtitle.as_deref().filter(|_| !narrow) {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(subtitle).size(13.).color(LABEL),
                                    )
                                    .truncate(),
                                );
                            }
                        });
                    });
                },
            );
        });
}

fn pill(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::new()
        .stroke(egui::Stroke::new(1., color.gamma_multiply(0.7)))
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(6, 1))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(12.).strong().color(color));
        });
}

/// Signal bars and milliseconds in a fixed-width column; blank for AI cars.
fn ping_cell(ui: &mut egui::Ui, ping: Option<Option<u16>>) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(74., 24.), egui::Sense::hover());
    let Some(ping) = ping else { return };
    let painter = ui.painter_at(rect);
    let (color, lit, text) = match ping {
        Some(ms) => (
            crate::hud::ping_color(ms),
            match ms {
                0..=59 => 3,
                60..=119 => 2,
                _ => 1,
            },
            format!("{} ms", ms.min(999)),
        ),
        None => (LABEL, 0, "– ms".into()),
    };
    for bar in 0..3 {
        let height = 5. + 4. * bar as f32;
        let x = rect.left() + 2. + bar as f32 * 5.;
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(x, rect.center().y + 7. - height),
                egui::pos2(x + 3., rect.center().y + 7.),
            ),
            1.,
            if bar < lit { color } else { WELL_RIM },
        );
    }
    painter.text(
        egui::pos2(rect.right(), rect.center().y),
        egui::Align2::RIGHT_CENTER,
        text,
        egui::FontId::proportional(15.),
        if ping.is_some() { color } else { LABEL },
    );
}

/// A small painted up/down button; the bundled font has no arrow glyphs.
fn arrow(ui: &mut egui::Ui, up: bool, enabled: bool) -> bool {
    let response = ui.add_enabled(
        enabled,
        egui::Button::new("").min_size(egui::vec2(26., 24.)),
    );
    let color = if enabled {
        Color32::from_gray(220)
    } else {
        WELL_RIM
    };
    let c = response.rect.center();
    let (tip, base) = if up { (-4., 3.) } else { (4., -3.) };
    ui.painter().add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(c.x, c.y + tip),
            egui::pos2(c.x + 5., c.y + base),
            egui::pos2(c.x - 5., c.y + base),
        ],
        color,
        egui::Stroke::NONE,
    ));
    response.clicked()
}

/// The owner may start once every driver is ready and the rules shown are the room's rules.
fn can_start(lobby: &Lobby, unapplied: bool) -> bool {
    !unapplied && lobby.players.iter().all(|p| p.ready)
}
/// Poll on real display time; loading and opening menus never stops the network.
pub fn poll(
    mut online: ResMut<Online>,
    time: Res<Time<Real>>,
    mut frontend: ResMut<Frontend>,
    mut session: ResMut<Session>,
    options: Res<Options>,
    mut log_stage: Local<Option<(Stage, usize)>>,
) {
    if let Some(host) = online.request.take() {
        if let Err(error) = online.begin(host, &options, frontend.aids) {
            online.error = Some(error);
        }
    }
    let mut test_started = online.test_started;
    let Some(client) = &mut online.client else {
        return;
    };
    if let Err(error) = client.poll(Duration::from_secs_f64(time.delta_secs_f64())) {
        client.error = Some(error);
        client.closed = true;
    }
    // Grid slots match the room's player order; AI and dropped drivers have no ping.
    session.pings = match &client.lobby {
        Some(lobby) if matches!(lobby.stage, Stage::Grid | Stage::Racing | Stage::Results) => (0
            ..session.race.entrants.len())
            .map(|slot| {
                lobby
                    .players
                    .get(slot)
                    .filter(|p| p.connected)
                    .and_then(|p| client.ping_ms(p.id))
            })
            .collect(),
        _ => Vec::new(),
    };
    if let Some(lobby) = &client.lobby {
        if *log_stage != Some((lobby.stage, lobby.players.len())) {
            info!(
                "Online phase {:?}, {} drivers admitted",
                lobby.stage,
                lobby.players.len()
            );
            *log_stage = Some((lobby.stage, lobby.players.len()));
        }
    }
    if let Some(expected) = options.test_lobby {
        if let Some(lobby) = client.lobby.clone() {
            if lobby.stage == Stage::Lobby && lobby.players.len() >= expected {
                if lobby
                    .players
                    .iter()
                    .find(|p| p.id == client.id)
                    .is_some_and(|p| !p.ready)
                {
                    let _ = client.send(ClientMessage::Ready(true));
                }
                if client.owner() && !test_started && lobby.players.iter().all(|p| p.ready) {
                    let _ = client.send(ClientMessage::Start);
                    test_started = true;
                }
            }
        }
    }
    if client.closed {
        let error = client
            .error
            .take()
            .unwrap_or_else(|| "The room closed.".into());
        online.leave(&mut session);
        online.error = Some(error);
        return;
    }
    if let Some(lobby) = client.load.take() {
        let Some(slot) = lobby.players.iter().position(|p| p.id == client.id) else {
            return;
        };
        let settings = &lobby.settings;
        frontend.draft.track = settings.track.clone();
        frontend.draft.car = settings.car.clone();
        frontend.draft.laps = settings.laps;
        frontend.draft.setup = settings.setup;
        frontend.draft.opponents = settings.ai_cars + lobby.players.len() - 1;
        frontend.draft.grid_slot = slot + 1;
        if let Some(identity) = &lobby.players[slot].identity {
            frontend.draft.team = identity.team.clone();
            frontend.draft.driver = identity.driver.clone();
        }
        frontend.aids = if settings.lock_aids {
            settings.assists
        } else {
            lobby.players[slot].assists
        };
        // DRS is always a room rule, even when ordinary aids are personal.
        frontend.aids.drs_anywhere = settings.assists.drs_anywhere;
        frontend.corner_braking = if settings.lock_aids {
            settings.corner_braking
        } else {
            frontend.corner_braking
        };
        frontend.pending = true;
        online.last_event = 0;
        online.loading = Some(lobby.epoch);
        online.install = Some(lobby);
    }
    online.test_started = test_started;
}
pub fn install(
    mut online: ResMut<Online>,
    mut session: ResMut<Session>,
    mut frontend: ResMut<Frontend>,
    options: Res<Options>,
    time: Res<Time<Real>>,
) {
    if options.test_driver {
        frontend.vsync = true;
    }
    session.poses.advance_correction(time.delta_secs_f64());
    if let Some(lobby) = online.install.take() {
        if let Some(error) = frontend.error.clone() {
            online.error = Some(error);
            online.leave(&mut session);
            return;
        }
        match racecraft_net::load_race(&options.assets, &lobby.settings, &lobby.players) {
            Ok(race) => {
                session.guidance = crate::driving::Guidance::new(&race, session.player);
                session.poses = Poses::new(&race);
                session.race = race;
                if options.test_driver {
                    online.pilot = Some(racecraft_sim::AiDriver::new(
                        racecraft_sim::DriverProfile {
                            pace: 0.93,
                            aggression: 0.5,
                            consistency: 1.,
                        },
                        &session.race.entrants[session.player].car,
                        &session.race.track,
                        session.player as u64 + 100,
                    ));
                }
                if options.car == "gen22" {
                    let mut field = Vec::new();
                    for player in &lobby.players {
                        if let Some(identity) = &player.identity {
                            match frontend.teams.selection(&identity.team, &identity.driver) {
                                Ok((team, driver)) => {
                                    field.push(Some(crate::teams::Teams::identity(team, driver)))
                                }
                                Err(error) => {
                                    online.error = Some(error);
                                    online.leave(&mut session);
                                    return;
                                }
                            }
                        }
                    }
                    let remaining: Vec<_> = frontend
                        .teams
                        .teams
                        .iter()
                        .flat_map(|team| {
                            team.drivers
                                .iter()
                                .map(move |driver| crate::teams::Teams::identity(team, driver))
                        })
                        .filter(|identity| {
                            !field.iter().flatten().any(|i| i.driver == identity.driver)
                        })
                        .collect();
                    field.extend(
                        remaining
                            .into_iter()
                            .map(Some)
                            .take(session.race.entrants.len().saturating_sub(field.len())),
                    );
                    session.identities = field;
                }
            }
            Err(error) => {
                online.error = Some(error);
                online.leave(&mut session);
                return;
            }
        }
    }
    let ready = session.ready;
    if ready && online.loading.is_some() {
        online.loading = None;
        if let Some(client) = &mut online.client {
            if let Err(error) = client.loaded() {
                online.error = Some(error);
            }
        }
    }
    let mut last_event = online.last_event;
    if let Some(client) = &mut online.client {
        if client.active() && ready {
            let before: Vec<_> = (0..session.race.entrants.len())
                .map(|i| session.poses.at(&session.race, i, 1.))
                .collect();
            match client.reconcile(&mut session.race) {
                Ok(true) => {
                    if let Some(observation) = client.observation.take() {
                        if let Ok(json) = serde_json::to_string(&observation) {
                            info!("ONLINE_TEST {json}");
                        }
                    }
                    let Session { poses, race, .. } = &mut *session;
                    poses.correct(&before, race);
                }
                Err(error) => {
                    client.error = Some(error);
                    client.closed = true;
                }
                _ => {}
            }
            // Predicted events never reach the HUD; authoritative reliable events do.
            for event in client.events.drain(..) {
                if event.id > last_event {
                    last_event = event.id;
                    session.race.events.push(event.event);
                }
            }
            if session.race.events.len() > 1024 {
                session.race.events.clear();
                session.messages.seen_events = 0;
            }
            if let Some(lobby) = &client.lobby {
                let slot = session.player;
                let mut aids = if lobby.settings.lock_aids {
                    lobby.settings.assists
                } else {
                    lobby.players[slot].assists
                };
                aids.drs_anywhere = lobby.settings.assists.drs_anywhere;
                session.race.entrants[slot].car.assists = aids;
                frontend.aids = aids;
                session.corner_braking = if lobby.settings.lock_aids {
                    lobby.settings.corner_braking
                } else {
                    frontend.corner_braking
                };
            }
        }
    }
    online.last_event = last_event;
}

#[cfg(test)]
mod tests {
    use super::*;
    use racecraft_net::Player;
    #[test]
    fn online_pause_keeps_prediction_alive_but_offline_pause_stops_the_race() {
        let options = Options::parse(std::iter::empty()).unwrap();
        let mut session = crate::tests::session();
        session.ready = true;
        session.paused = true;
        session.screen = Screen::Paused;
        let mut client = Client::connect(
            std::path::Path::new("."),
            "127.0.0.1:9".parse().unwrap(),
            "Test".into(),
            racecraft_sim::Assists::default(),
            Some(1),
        )
        .unwrap();
        client.lobby = Some(Lobby {
            settings: RaceSettings::default(),
            players: vec![Player {
                id: 1,
                name: "Test".into(),
                ready: true,
                connected: true,
                identity: None,
                assists: racecraft_sim::Assists::default(),
            }],
            owner: 1,
            epoch: 0,
            stage: Stage::Racing,
        });
        client.authoritative_tick = Some(session.race.tick);
        let mut online = Online::new(&options);
        online.client = Some(client);
        let mut app = App::new();
        app.insert_resource(session)
            .insert_resource(online)
            .add_systems(Update, crate::step_race);
        app.update();
        assert_eq!(app.world().resource::<Session>().race.tick, 1);
        app.world_mut().resource_mut::<Online>().client = None;
        app.update();
        assert_eq!(app.world().resource::<Session>().race.tick, 1);
    }
    #[test]
    fn start_waits_for_unapplied_rule_changes() {
        let lobby = Lobby {
            settings: RaceSettings::default(),
            players: vec![Player {
                id: 1,
                name: "Owner".into(),
                ready: true,
                connected: true,
                identity: None,
                assists: racecraft_sim::Assists::default(),
            }],
            owner: 1,
            epoch: 0,
            stage: Stage::Lobby,
        };
        assert!(can_start(&lobby, false));
        // e.g. AI cars moved to 0 on the slider but not applied yet.
        assert!(!can_start(&lobby, true));
    }

    #[test]
    fn host_join_panel_renders_with_existing_frontend() {
        let options = Options::parse(std::iter::empty()).unwrap();
        let mut online = Online::new(&options);
        online.open = true;
        let mut frontend = Frontend::new(&options);
        let mut session = crate::tests::session();
        let context = egui::Context::default();
        online.prepare();
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            online.join_tab(ui, &mut frontend);
            online.host_tab(ui, &mut frontend);
            online.room_tab(ui, &frontend, &session);
        });
        online.finish(&frontend, &mut session, &options);
        assert!(!output.shapes.is_empty());
        output.textures_delta.clear();
    }
}
