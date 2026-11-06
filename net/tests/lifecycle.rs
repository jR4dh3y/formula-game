use racecraft_net::*;
use racecraft_sim::{Assists, CarInput, Driver, PHYSICS_RATE};
use std::{
    net::{SocketAddr, UdpSocket},
    path::PathBuf,
    time::{Duration, Instant},
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("racecraft-net-test-{}", random_id()));
        std::fs::create_dir_all(root.join("tracks/spa")).unwrap();
        std::fs::create_dir_all(root.join("cars/rb6")).unwrap();
        let points: Vec<_> = (0..64)
            .map(|i| {
                let a = i as f64 * std::f64::consts::TAU / 64.;
                [150. * a.cos(), 0., 150. * a.sin()]
            })
            .collect();
        let track = serde_json::json!({"name":"Network oval","source":"test","centerline":points,"half_width":vec![12.;64]});
        std::fs::write(
            root.join("tracks/spa/track.json"),
            serde_json::to_vec(&track).unwrap(),
        )
        .unwrap();
        let mut bytes = b"F1CL".to_vec();
        for n in [1u32, 2, 0] {
            bytes.extend(n.to_le_bytes());
        }
        for vertex in [
            [-500f32, 0., -500.],
            [-500., 0., 500.],
            [500., 0., -500.],
            [500., 0., -500.],
            [-500., 0., 500.],
            [500., 0., 500.],
        ] {
            for x in vertex {
                bytes.extend(x.to_le_bytes());
            }
        }
        bytes.extend([0u8; 2]);
        std::fs::write(root.join("tracks/spa/collision.bin"), bytes).unwrap();
        std::fs::write(
            root.join("cars/rb6/car.json"),
            include_bytes!("fixtures/car.json"),
        )
        .unwrap();
        Self(root)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
/// A real UDP forwarding boundary with deterministic loss, duplication and delayed packets.
struct Proxy {
    socket: UdpSocket,
    server: SocketAddr,
    client: Option<SocketAddr>,
    count: u64,
    queue: Vec<(u64, SocketAddr, Vec<u8>)>,
    tick: u64,
    loss: bool,
}
impl Proxy {
    fn new(server: SocketAddr, loss: bool) -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        Self {
            socket,
            server,
            client: None,
            count: 0,
            queue: vec![],
            tick: 0,
            loss,
        }
    }
    fn addr(&self) -> SocketAddr {
        self.socket.local_addr().unwrap()
    }
    fn pump(&mut self) {
        self.tick += 1;
        let mut buf = [0u8; 2048];
        while let Ok((len, from)) = self.socket.recv_from(&mut buf) {
            let to = if from == self.server {
                self.client.unwrap()
            } else {
                self.client = Some(from);
                self.server
            };
            self.count += 1;
            if self.loss && self.count.is_multiple_of(20) {
                continue;
            }
            let delay = if self.loss && self.count.is_multiple_of(7) {
                3
            } else {
                0
            };
            self.queue
                .push((self.tick + delay, to, buf[..len].to_vec()));
            if self.loss && self.count.is_multiple_of(31) {
                self.queue.push((self.tick + 1, to, buf[..len].to_vec()));
            }
        }
        let mut later = vec![];
        for (at, to, bytes) in self.queue.drain(..) {
            if at <= self.tick {
                self.socket.send_to(&bytes, to).unwrap();
            } else {
                later.push((at, to, bytes));
            }
        }
        self.queue = later;
    }
}
#[test]
fn shipped_and_legacy_room_settings_default_ai_power() {
    let shipped: RaceSettings =
        serde_json::from_str(include_str!("../../deploy/render/rules.json")).unwrap();
    assert_eq!(shipped.ai_power, 1.);
    shipped.validate().unwrap();

    let mut legacy = serde_json::to_value(RaceSettings::default()).unwrap();
    legacy.as_object_mut().unwrap().remove("ai_power");
    let legacy: RaceSettings = serde_json::from_value(legacy).unwrap();
    assert_eq!(legacy.ai_power, 1.);
    legacy.validate().unwrap();
}

#[test]
fn sixteen_real_udp_clients_race_with_loss_and_ai_takeover() {
    let fixture = Fixture::new();
    let settings = RaceSettings {
        ai_cars: 0,
        ..RaceSettings::default()
    };
    let mut server =
        Server::bind(&fixture.0, "127.0.0.1:0".parse().unwrap(), settings, None).unwrap();
    let mut proxies: Vec<_> = (0..16)
        .map(|_| Proxy::new(server.address(), true))
        .collect();
    let mut clients: Vec<_> = (0..16)
        .map(|i| {
            Client::connect(
                &fixture.0,
                proxies[i].addr().into(),
                format!("Driver {i}"),
                Assists::default(),
                None,
            )
            .unwrap()
        })
        .collect();
    let dt = Duration::from_secs_f64(1. / PHYSICS_RATE);
    let mut races: Vec<Option<racecraft_sim::Race>> = (0..16).map(|_| None).collect();
    let mut ready = [false; 16];
    let mut started = false;
    let start = Instant::now();
    for _ in 0..6000 {
        for proxy in &mut proxies {
            proxy.pump();
        }
        server.update(dt).unwrap();
        for (i, client) in clients.iter_mut().enumerate() {
            client.poll(dt).unwrap();
            assert!(!client.closed, "{:?}", client.error);
            if !ready[i] && client.lobby.as_ref().is_some_and(|l| l.players.len() == 16) {
                client.send(ClientMessage::Ready(true)).unwrap();
                ready[i] = true;
            }
            if let Some(lobby) = client.load.take() {
                let race = load_race(&fixture.0, &lobby.settings, &lobby.players).unwrap();
                races[i] = Some(race);
                client.loaded().unwrap();
            }
            if let Some(race) = &mut races[i] {
                client.reconcile(race).unwrap();
                client
                    .predict(
                        race,
                        CarInput {
                            throttle: 0.7,
                            ..CarInput::default()
                        },
                    )
                    .unwrap();
            }
            client.flush().unwrap();
        }
        if !started
            && server.lobby.players.len() == 16
            && server.lobby.players.iter().all(|p| p.ready)
        {
            let owner = clients
                .iter_mut()
                .find(|c| c.id == server.lobby.owner)
                .unwrap();
            owner.send(ClientMessage::Start).unwrap();
            started = true;
        }
        if server.race.as_ref().is_some_and(|r| r.tick > 3000)
            && clients
                .iter()
                .all(|c| c.authoritative_tick.is_some_and(|t| t > 2950))
        {
            break;
        }
        std::thread::sleep(Duration::from_micros(100));
    }
    assert!(started);
    assert_eq!(server.lobby.players.len(), 16);
    assert!(server.race.as_ref().unwrap().tick > 2950);
    let mut slots: Vec<_> = clients.iter().map(|c| c.slot().unwrap()).collect();
    slots.sort_unstable();
    assert_eq!(slots, (0..16).collect::<Vec<_>>());
    for client in &clients {
        let lag = server
            .race
            .as_ref()
            .unwrap()
            .tick
            .saturating_sub(client.authoritative_tick.unwrap());
        assert!(lag < 180, "stale authority: lag={lag} ticks");
    }
    assert!(server.snapshot_bytes < MAX_MESSAGE);
    let skipped: u64 = clients.iter().map(|c| c.resims_skipped).sum();
    let replayed: u64 = clients.iter().map(|c| c.resims).sum();
    println!("16 scripted clients, oval, 5% loss, duplication 1/31, reorder 3 ticks/7: delta snapshot={} bytes, full={} bytes, exact predictions={skipped}, rollbacks={replayed}, tick={}, harness elapsed={:?}",server.snapshot_bytes,server.full_snapshot_bytes,server.race.as_ref().unwrap().tick,start.elapsed());
    assert!(
        server.snapshot_bytes < server.full_snapshot_bytes,
        "clients should receive deltas against acknowledged snapshots"
    );
    // Identical scripted inputs on one machine: most snapshots must match the
    // prediction exactly, which skips the rollback and replay entirely.
    assert!(
        skipped > replayed,
        "exact predictions {skipped} vs rollbacks {replayed}"
    );
    let departed = clients.iter().position(|c| !c.owner()).unwrap();
    let slot = clients[departed].slot().unwrap();
    clients.remove(departed);
    for _ in 0..100 {
        for p in &mut proxies {
            p.pump();
        }
        server.update(dt).unwrap();
    }
    assert!(matches!(
        server.race.as_ref().unwrap().entrants[slot].driver,
        Driver::Ai(_)
    ));
}
#[test]
fn complete_snapshot_round_trip_replays_identically_and_rejects_wrong_field() {
    let fixture = Fixture::new();
    let players: Vec<_> = (0..16)
        .map(|i| Player {
            id: i,
            name: format!("P{i}"),
            ready: true,
            connected: true,
            identity: None,
            assists: Assists::default(),
        })
        .collect();
    let mut race = load_race(
        &fixture.0,
        &RaceSettings {
            ai_cars: 0,
            ..RaceSettings::default()
        },
        &players,
    )
    .unwrap();
    let mut other = race.clone();
    for _ in 0..2500 {
        race.step(
            &[CarInput {
                throttle: 0.8,
                ..CarInput::default()
            }; 16],
        );
    }
    let state = encode(&race.snapshot()).unwrap();
    let snapshot = decode(&state).unwrap();
    other.restore(&snapshot).unwrap();
    for _ in 0..240 {
        race.step(
            &[CarInput {
                throttle: 0.4,
                ..CarInput::default()
            }; 16],
        );
        other.step(
            &[CarInput {
                throttle: 0.4,
                ..CarInput::default()
            }; 16],
        );
    }
    assert_eq!(
        encode(&race.snapshot()).unwrap(),
        encode(&other.snapshot()).unwrap()
    );
    other.entrants.pop();
    assert!(other.restore(&snapshot).is_err());
}
#[test]
fn wire_rejects_oversize_trailing_garbage_nonfinite_and_bad_settings() {
    assert!(decode::<ClientMessage>(&vec![0; MAX_MESSAGE + 1]).is_err());
    let mut message = encode(&ClientMessage::Ready(true)).unwrap();
    message.push(1);
    assert!(decode::<ClientMessage>(&message).is_err());
    assert!(!valid_input(&CarInput {
        throttle: f64::NAN,
        ..CarInput::default()
    }));
    let mut settings = RaceSettings {
        track: "../spa".into(),
        ..RaceSettings::default()
    };
    assert!(settings.validate().is_err());
    settings.track = "spa".into();
    settings.assists.traction = f64::INFINITY;
    assert!(settings.validate().is_err());
}

#[test]
fn requested_ai_count_creates_that_exact_field_size() {
    let fixture = Fixture::new();
    let settings = RaceSettings {
        ai_cars: 3,
        ..RaceSettings::default()
    };
    let players = (0..2)
        .map(|id| Player {
            id,
            name: format!("Human {id}"),
            ready: false,
            connected: true,
            identity: None,
            assists: Assists::default(),
        })
        .collect::<Vec<_>>();
    let race = load_race(&fixture.0, &settings, &players).unwrap();
    assert_eq!(race.entrants.len(), 5);
    assert_eq!(
        race.entrants
            .iter()
            .filter(|entrant| matches!(entrant.driver, racecraft_sim::Driver::Ai(_)))
            .count(),
        3
    );

    let full = RaceSettings {
        ai_cars: 15,
        ..RaceSettings::default()
    };
    assert!(load_race(&fixture.0, &full, &players).is_err());
}

fn pump(
    server: &mut Server,
    clients: &mut [Client],
    races: &mut [Option<racecraft_sim::Race>],
    root: &std::path::Path,
    ticks: usize,
) {
    let dt = Duration::from_secs_f64(1. / PHYSICS_RATE);
    for _ in 0..ticks {
        server.update(dt).unwrap();
        for (i, client) in clients.iter_mut().enumerate() {
            client.poll(dt).unwrap();
            if let Some(lobby) = client.load.take() {
                races[i] = Some(load_race(root, &lobby.settings, &lobby.players).unwrap());
                client.loaded().unwrap();
            }
            if let Some(race) = &mut races[i] {
                client.reconcile(race).unwrap();
                client.predict(race, CarInput::default()).unwrap();
            }
            client.flush().unwrap();
        }
        std::thread::sleep(Duration::from_micros(100));
    }
}
#[test]
fn owner_rules_commands_results_return_and_host_loss() {
    let fixture = Fixture::new();
    let settings = RaceSettings {
        ai_cars: 0,
        assists: Assists {
            automatic_gears: false,
            traction: 0.3,
            ..Assists::default()
        },
        ..RaceSettings::default()
    };
    let mut server = Server::bind(
        &fixture.0,
        "127.0.0.1:0".parse().unwrap(),
        settings.clone(),
        None,
    )
    .unwrap();
    let mut clients: Vec<_> = (0..2)
        .map(|i| {
            Client::connect(
                &fixture.0,
                server.address().into(),
                format!("Human {i}"),
                Assists::default(),
                None,
            )
            .unwrap()
        })
        .collect();
    let mut races = vec![None, None];
    pump(&mut server, &mut clients, &mut races, &fixture.0, 80);
    assert_eq!(server.lobby.players.len(), 2);
    let owner = clients.iter().position(Client::owner).unwrap();
    let guest = 1 - owner;
    clients[guest].send(ClientMessage::Start).unwrap();
    pump(&mut server, &mut clients, &mut races, &fixture.0, 10);
    assert_eq!(server.lobby.stage, Stage::Lobby);
    assert!(clients[guest].error.take().is_some());
    let order: Vec<_> = server.lobby.players.iter().rev().map(|p| p.id).collect();
    clients[owner]
        .send(ClientMessage::Grid(order.clone()))
        .unwrap();
    pump(&mut server, &mut clients, &mut races, &fixture.0, 10);
    assert_eq!(
        server
            .lobby
            .players
            .iter()
            .map(|p| p.id)
            .collect::<Vec<_>>(),
        order
    );
    for client in &mut clients {
        client.send(ClientMessage::Ready(true)).unwrap();
    }
    pump(&mut server, &mut clients, &mut races, &fixture.0, 10);
    clients[owner].send(ClientMessage::Start).unwrap();
    pump(&mut server, &mut clients, &mut races, &fixture.0, 80);
    assert_eq!(server.lobby.stage, Stage::Grid);
    let slot = clients[guest].slot().unwrap();
    assert_eq!(server.race.as_ref().unwrap().entrants.len(), 2);
    assert_eq!(
        server.race.as_ref().unwrap().entrants[slot]
            .car
            .assists
            .traction,
        0.3
    );
    let epoch = server.lobby.epoch;
    let starting_gear = server.race.as_ref().unwrap().entrants[slot]
        .car
        .powertrain
        .gear;
    let tick = server.race.as_ref().unwrap().tick + 12;
    for _ in 0..3 {
        clients[guest]
            .send(ClientMessage::Command {
                epoch,
                sequence: 1,
                tick,
                kind: Command::ShiftUp,
            })
            .unwrap();
    }
    pump(&mut server, &mut clients, &mut races, &fixture.0, 50);
    assert_eq!(
        server.commands_applied, 1,
        "a duplicate command was applied twice"
    );
    assert_eq!(
        server.race.as_ref().unwrap().entrants[slot]
            .car
            .powertrain
            .gear,
        starting_gear + 1
    );
    let tick = server.race.as_ref().unwrap().tick + 12;
    for _ in 0..3 {
        clients[guest]
            .send(ClientMessage::Command {
                epoch,
                sequence: 2,
                tick,
                kind: Command::Recover,
            })
            .unwrap();
    }
    pump(&mut server, &mut clients, &mut races, &fixture.0, 50);
    assert_eq!(server.commands_applied, 2);
    // Cross a real lap boundary to exercise authoritative timing, finish events and results.
    let race = server.race.as_mut().unwrap();
    race.phase = racecraft_sim::Phase::Racing;
    race.time = 90.;
    race.laps = 1;
    for entrant in &mut race.entrants {
        entrant.timing.distance = race.track.length - 0.2;
        entrant.car.position = race.track.frame_at(0.2).point_at(0.);
        entrant.car.velocity = Default::default();
    }
    pump(&mut server, &mut clients, &mut races, &fixture.0, 50);
    assert_eq!(server.lobby.stage, Stage::Results);
    let authoritative = encode(&server.race.as_ref().unwrap().snapshot()).unwrap();
    for race in &races {
        assert_eq!(
            encode(&race.as_ref().unwrap().snapshot()).unwrap(),
            authoritative
        );
    }
    let finish_ids: Vec<_> = clients[guest]
        .events
        .iter()
        .filter(|e| matches!(e.event, racecraft_sim::RaceEvent::Finished { .. }))
        .map(|e| e.id)
        .collect();
    assert_eq!(finish_ids.len(), 2);
    assert_ne!(finish_ids[0], finish_ids[1]);
    clients[owner].send(ClientMessage::Return).unwrap();
    pump(&mut server, &mut clients, &mut races, &fixture.0, 30);
    assert_eq!(server.lobby.stage, Stage::Lobby);
    for client in &mut clients {
        client.send(ClientMessage::Ready(true)).unwrap();
    }
    pump(&mut server, &mut clients, &mut races, &fixture.0, 10);
    clients[owner].send(ClientMessage::Start).unwrap();
    pump(&mut server, &mut clients, &mut races, &fixture.0, 50);
    assert_eq!(server.lobby.epoch, epoch + 1);
    assert_eq!(server.lobby.stage, Stage::Grid);
    // This server has no hosting player, so the room passes to the guest.
    let guest_id = clients[guest].id;
    drop(clients.remove(owner));
    server.update(Duration::from_millis(3)).unwrap();
    assert_eq!(server.lobby.stage, Stage::Grid);
    assert_eq!(server.lobby.owner, guest_id);
}

#[test]
fn mismatched_assets_are_rejected_before_admission_and_silence_times_out() {
    let fixture = Fixture::new();
    let wrong = Fixture::new();
    let file = wrong.0.join("cars/rb6/car.json");
    let mut bytes = std::fs::read(&file).unwrap();
    bytes.push(b' ');
    std::fs::write(file, bytes).unwrap();
    let mut server = Server::bind(
        &fixture.0,
        "127.0.0.1:0".parse().unwrap(),
        RaceSettings::default(),
        None,
    )
    .unwrap();
    let mut bad = Client::connect(
        &wrong.0,
        server.address().into(),
        "Mismatch".into(),
        Assists::default(),
        None,
    )
    .unwrap();
    let dt = Duration::from_millis(3);
    let mut rejected = false;
    for _ in 0..100 {
        server.update(dt).unwrap();
        if let Err(error) = bad.poll(dt) {
            assert!(error.contains("assets"));
            rejected = true;
            break;
        }
        bad.flush().unwrap();
        std::thread::sleep(Duration::from_micros(100));
    }
    assert!(rejected && bad.closed);
    assert!(server.lobby.players.is_empty());
    let mut lonely = Client::connect(
        &fixture.0,
        "127.0.0.1:9".parse().unwrap(),
        "Timeout".into(),
        Assists::default(),
        None,
    )
    .unwrap();
    assert!(lonely.poll(TIMEOUT + Duration::from_secs(1)).is_err());
    assert!(lonely.closed);
}

#[test]
fn snapshots_restore_ai_decisions_and_cached_plans_for_a_mixed_grid() {
    let fixture = Fixture::new();
    let players: Vec<_> = (0..2)
        .map(|i| Player {
            id: i,
            name: format!("P{i}"),
            ready: true,
            connected: true,
            identity: None,
            assists: Assists::default(),
        })
        .collect();
    let mut race = load_race(
        &fixture.0,
        &RaceSettings {
            ai_cars: 14,
            ..RaceSettings::default()
        },
        &players,
    )
    .unwrap();
    let mut other = race.clone();
    for _ in 0..2800 {
        race.step(
            &[CarInput {
                throttle: 0.65,
                ..CarInput::default()
            }; 2],
        );
    }
    let snapshot = decode(&encode(&race.snapshot()).unwrap()).unwrap();
    other.restore(&snapshot).unwrap();
    for _ in 0..720 {
        race.step(
            &[CarInput {
                brake: 0.1,
                ..CarInput::default()
            }; 2],
        );
        other.step(
            &[CarInput {
                brake: 0.1,
                ..CarInput::default()
            }; 2],
        );
    }
    assert_eq!(
        encode(&race.snapshot()).unwrap(),
        encode(&other.snapshot()).unwrap()
    );
}

#[test]
fn gen22_assigns_distinct_stock_driver_identities_and_rejects_duplicates() {
    let fixture = Fixture::new();
    std::fs::create_dir_all(fixture.0.join("cars/gen22")).unwrap();
    let data = serde_json::json!({"teams":[{"id":"red_bull","drivers":[{"id":"verstappen"},{"id":"perez"}]},{"id":"ferrari","drivers":[{"id":"leclerc"}]}]});
    std::fs::write(
        fixture.0.join("cars/gen22/teams.json"),
        serde_json::to_vec(&data).unwrap(),
    )
    .unwrap();
    let first = select_identity(&fixture.0, "gen22", None, &[])
        .unwrap()
        .unwrap();
    let players = vec![Player {
        id: 1,
        name: "Alice".into(),
        ready: false,
        connected: true,
        identity: Some(first.clone()),
        assists: Assists::default(),
    }];
    let second = select_identity(&fixture.0, "gen22", None, &players)
        .unwrap()
        .unwrap();
    assert_ne!(first, second);
    assert!(select_identity(&fixture.0, "gen22", Some(first), &players).is_err());
    assert!(select_identity(
        &fixture.0,
        "gen22",
        Some(DriverIdentity {
            team: "no_team".into(),
            driver: "no_driver".into()
        }),
        &players
    )
    .is_err());
    assert_eq!(
        select_identity(&fixture.0, "rb6", Some(second), &players).unwrap(),
        None
    );
}

fn join(fixture: &Fixture, server: &Server, name: &str, id: Option<u64>) -> Client {
    Client::connect(
        &fixture.0,
        server.address().into(),
        name.into(),
        Assists::default(),
        id,
    )
    .unwrap()
}

#[test]
fn hosted_room_closes_when_its_host_leaves() {
    let fixture = Fixture::new();
    let host_id = 77;
    let mut server = Server::bind(
        &fixture.0,
        "127.0.0.1:0".parse().unwrap(),
        RaceSettings::default(),
        Some(host_id),
    )
    .unwrap();
    let mut clients = vec![
        join(&fixture, &server, "Host", Some(host_id)),
        join(&fixture, &server, "Guest", None),
    ];
    let mut races = vec![None, None];
    pump(&mut server, &mut clients, &mut races, &fixture.0, 80);
    assert_eq!(server.lobby.players.len(), 2);
    assert_eq!(server.lobby.owner, host_id);
    drop(clients.remove(0));
    pump(&mut server, &mut clients, &mut races[..1], &fixture.0, 20);
    assert_eq!(server.lobby.stage, Stage::Closed);
    assert!(clients[0].closed);
}

#[test]
fn dedicated_room_hands_over_and_resets_when_everyone_leaves() {
    let fixture = Fixture::new();
    let mut server = Server::bind(
        &fixture.0,
        "127.0.0.1:0".parse().unwrap(),
        RaceSettings::default(),
        None,
    )
    .unwrap();
    let mut clients = vec![
        join(&fixture, &server, "First", None),
        join(&fixture, &server, "Second", None),
    ];
    let mut races = vec![None, None];
    pump(&mut server, &mut clients, &mut races, &fixture.0, 80);
    assert_eq!(server.lobby.players.len(), 2);
    let first = server.lobby.owner;
    let other = clients.iter().position(|c| c.id != first).unwrap();
    let other_id = clients[other].id;
    clients.retain(|c| c.id != first);
    pump(&mut server, &mut clients, &mut races[..1], &fixture.0, 20);
    assert_eq!(
        server.lobby.owner, other_id,
        "ownership passes to the next driver"
    );
    assert_eq!(server.lobby.stage, Stage::Lobby);
    clients.clear();
    pump(&mut server, &mut clients, &mut [], &fixture.0, 20);
    assert!(server.lobby.players.is_empty());
    assert_eq!(server.lobby.owner, 0);
    // The server keeps running; the next driver owns a fresh room.
    let mut clients = vec![join(&fixture, &server, "Later", None)];
    let mut races = vec![None];
    pump(&mut server, &mut clients, &mut races, &fixture.0, 80);
    assert_eq!(server.lobby.owner, clients[0].id);
}

#[test]
fn rejected_join_learns_why_immediately() {
    let fixture = Fixture::new();
    let mut server = Server::bind(
        &fixture.0,
        "127.0.0.1:0".parse().unwrap(),
        RaceSettings::default(),
        None,
    )
    .unwrap();
    let mut clients = vec![join(&fixture, &server, "Sam", None)];
    let mut races = vec![None, None];
    pump(&mut server, &mut clients, &mut races, &fixture.0, 80);
    clients.push(join(&fixture, &server, " sam ", None));
    // 80 ticks is about 0.2 s, far below the eight-second admission timeout.
    pump(&mut server, &mut clients, &mut races, &fixture.0, 80);
    assert!(clients[1].closed);
    let error = clients[1].error.clone().unwrap_or_default();
    assert!(error.contains("already in use"), "{error}");
    assert_eq!(server.lobby.players.len(), 1);
}

#[test]
fn fingerprint_covers_simulation_assets_but_not_visuals() {
    let fixture = Fixture::new();
    let settings = RaceSettings::default();
    std::fs::write(fixture.0.join("tracks/spa/visual.glb"), b"one").unwrap();
    let before = fingerprint(&fixture.0, &settings).unwrap();
    std::fs::write(fixture.0.join("tracks/spa/visual.glb"), b"two").unwrap();
    assert_eq!(fingerprint(&fixture.0, &settings).unwrap(), before);
    let collision = fixture.0.join("tracks/spa/collision.bin");
    let mut bytes = std::fs::read(&collision).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    std::fs::write(&collision, bytes).unwrap();
    assert_ne!(fingerprint(&fixture.0, &settings).unwrap(), before);
}

#[test]
fn dual_stack_room_admits_ipv4_and_ipv6_drivers() {
    let fixture = Fixture::new();
    let Ok(mut server) = Server::bind(
        &fixture.0,
        "[::]:0".parse().unwrap(),
        RaceSettings::default(),
        None,
    ) else {
        eprintln!("IPv6 unavailable on this machine; skipping");
        return;
    };
    let port = server.address().port();
    let connect = |address: &str, name: &str| {
        Client::connect(
            &fixture.0,
            address.parse().unwrap(),
            name.into(),
            Assists::default(),
            None,
        )
        .unwrap()
    };
    let mut clients = vec![
        connect(&format!("127.0.0.1:{port}"), "Four"),
        connect(&format!("[::1]:{port}"), "Six"),
    ];
    let mut races = vec![None, None];
    pump(&mut server, &mut clients, &mut races, &fixture.0, 120);
    assert_eq!(server.lobby.players.len(), 2);
}

#[test]
fn websocket_room_races_rejects_duplicate_ids_and_resets_after_disconnect() {
    let fixture = Fixture::new();
    let mut server = Server::bind_websocket(
        &fixture.0,
        "127.0.0.1:0".parse().unwrap(),
        RaceSettings::default(),
    )
    .unwrap();
    let endpoint: Endpoint = format!("ws://{}", server.address()).parse().unwrap();
    let mut clients = vec![Client::connect(
        &fixture.0,
        endpoint.clone(),
        "WebSocket owner".into(),
        Assists::default(),
        None,
    )
    .unwrap()];
    let mut races = vec![None];
    pump(&mut server, &mut clients, &mut races, &fixture.0, 150);
    assert_eq!(server.lobby.players.len(), 1);
    assert!(clients[0].owner());
    let owner = clients[0].id;
    // A new socket cannot take over an existing socket's ID, even in a private room.
    let mut duplicate = Client::connect(
        &fixture.0,
        endpoint.clone(),
        "Duplicate ID".into(),
        Assists::default(),
        Some(owner),
    )
    .unwrap();
    for _ in 0..60 {
        let _ = duplicate.poll(Duration::from_millis(2));
        pump(&mut server, &mut clients, &mut races, &fixture.0, 1);
    }
    assert_eq!(server.lobby.players.len(), 1);
    assert!(!clients[0].closed);
    drop(duplicate);
    clients.push(
        Client::connect(
            &fixture.0,
            endpoint.clone(),
            "WebSocket guest".into(),
            Assists::default(),
            None,
        )
        .unwrap(),
    );
    races.push(None);
    pump(&mut server, &mut clients, &mut races, &fixture.0, 150);
    assert_eq!(server.lobby.players.len(), 2);
    for client in &mut clients {
        client.send(ClientMessage::Ready(true)).unwrap();
    }
    pump(&mut server, &mut clients, &mut races, &fixture.0, 60);
    clients[0].send(ClientMessage::Start).unwrap();
    pump(&mut server, &mut clients, &mut races, &fixture.0, 1500);
    assert!(matches!(server.lobby.stage, Stage::Grid | Stage::Racing));
    for client in &clients {
        assert!(client.authoritative_tick.is_some_and(|t| t > 100));
    }
    let guest = clients[1].id;
    clients.remove(0);
    races.remove(0);
    pump(&mut server, &mut clients, &mut races, &fixture.0, 80);
    assert_eq!(server.lobby.owner, guest);
    assert!(matches!(
        server.race.as_ref().unwrap().entrants[0].driver,
        Driver::Ai(_)
    ));
    clients.clear();
    races.clear();
    pump(&mut server, &mut clients, &mut races, &fixture.0, 80);
    assert_eq!(server.lobby.stage, Stage::Lobby);
    assert!(server.lobby.players.is_empty());
    clients.push(
        Client::connect(
            &fixture.0,
            endpoint,
            "Next room owner".into(),
            Assists::default(),
            None,
        )
        .unwrap(),
    );
    races.push(None);
    pump(&mut server, &mut clients, &mut races, &fixture.0, 150);
    assert_eq!(server.lobby.owner, clients[0].id);
}

#[test]
fn websocket_reports_failed_connections_and_drop_cancels_connect() {
    let fixture = Fixture::new();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let endpoint = format!("ws://{address}").parse().unwrap();
    let mut client = Client::connect(
        &fixture.0,
        endpoint,
        "Offline".into(),
        Assists::default(),
        None,
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Err(error) = client.poll(Duration::from_millis(5)) {
            assert!(error.contains("WebSocket"), "{error}");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "connection failure must reach the game"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(client.closed);
    let stalled = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("ws://{}", stalled.local_addr().unwrap())
        .parse()
        .unwrap();
    let client = Client::connect(
        &fixture.0,
        endpoint,
        "Cancelled".into(),
        Assists::default(),
        None,
    )
    .unwrap();
    let before = Instant::now();
    drop(client);
    assert!(
        before.elapsed() < Duration::from_secs(1),
        "leaving cancels a stalled handshake"
    );
}

#[test]
fn websocket_rejects_oversized_frames_and_mismatched_assets() {
    let fixture = Fixture::new();
    let mut server = Server::bind_websocket(
        &fixture.0,
        "127.0.0.1:0".parse().unwrap(),
        RaceSettings::default(),
    )
    .unwrap();
    let url = format!("ws://{}/ws", server.address());
    let (mut raw, _) = tokio_tungstenite::tungstenite::connect(url.as_str()).unwrap();
    let mut hello = PROTOCOL_ID.to_le_bytes().to_vec();
    hello.extend(random_id().to_le_bytes());
    raw.send(tokio_tungstenite::tungstenite::Message::Binary(
        hello.into(),
    ))
    .unwrap();
    // The message/frame limit is applied before delivering data to the simulation.
    raw.send(tokio_tungstenite::tungstenite::Message::Binary(
        vec![0; 4096].into(),
    ))
    .unwrap();
    if let tokio_tungstenite::tungstenite::stream::MaybeTlsStream::Plain(stream) = raw.get_mut() {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
    }
    assert!(raw.read().is_err(), "oversized frame must close the socket");
    pump(&mut server, &mut [], &mut [], &fixture.0, 20);
    assert!(server.lobby.players.is_empty());
    let car = fixture.0.join("cars/rb6/car.json");
    let mut data = std::fs::read(&car).unwrap();
    data.push(b' ');
    std::fs::write(car, data).unwrap();
    let mut client = Client::connect(
        &fixture.0,
        url.parse().unwrap(),
        "Wrong assets".into(),
        Assists::default(),
        None,
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        server.update(Duration::from_millis(2)).unwrap();
        match client.poll(Duration::from_millis(2)) {
            Err(error) => {
                assert!(error.contains("assets differ"), "{error}");
                break;
            }
            Ok(()) => assert!(
                Instant::now() < deadline,
                "mismatched content must fail admission"
            ),
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(server.lobby.players.is_empty());
}

#[test]
fn recovery_while_loading_does_not_consume_command_sequence() {
    let fixture = Fixture::new();
    let mut server = Server::bind_websocket(
        &fixture.0,
        "127.0.0.1:0".parse().unwrap(),
        RaceSettings::default(),
    )
    .unwrap();
    let mut client = Client::connect(
        &fixture.0,
        format!("ws://{}", server.address()).parse().unwrap(),
        "Recovery".into(),
        Assists::default(),
        None,
    )
    .unwrap();
    let dt = Duration::from_millis(2);
    let deadline = Instant::now() + Duration::from_secs(3);
    while client.slot().is_none() {
        server.update(dt).unwrap();
        client.poll(dt).unwrap();
        assert!(Instant::now() < deadline);
        std::thread::sleep(dt);
    }
    client.send(ClientMessage::Ready(true)).unwrap();
    for _ in 0..40 {
        server.update(dt).unwrap();
        client.poll(dt).unwrap();
        std::thread::sleep(dt);
    }
    client.send(ClientMessage::Start).unwrap();
    let loading = loop {
        server.update(dt).unwrap();
        client.poll(dt).unwrap();
        if let Some(lobby) = client.load.take() {
            break lobby;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(dt);
    };
    assert_eq!(client.stage(), Stage::Loading);
    client.recover(1).unwrap();
    let mut race = load_race(&fixture.0, &loading.settings, &loading.players).unwrap();
    client.loaded().unwrap();
    while client.authoritative_tick.is_none() {
        server.update(dt).unwrap();
        client.poll(dt).unwrap();
        client.reconcile(&mut race).unwrap();
        assert!(Instant::now() < deadline);
        std::thread::sleep(dt);
    }
    client.reconcile(&mut race).unwrap();
    client.recover(race.tick + 1).unwrap();
    let mut clients = vec![client];
    let mut races = vec![Some(race)];
    pump(&mut server, &mut clients, &mut races, &fixture.0, 100);
    assert_eq!(
        server.commands_applied, 1,
        "the first accepted recovery must still have sequence 1"
    );
}

#[test]
fn websocket_paused_reader_recovers_without_blocking_other_drivers() {
    let fixture = Fixture::new();
    let mut server = Server::bind_websocket(
        &fixture.0,
        "127.0.0.1:0".parse().unwrap(),
        RaceSettings::default(),
    )
    .unwrap();
    let endpoint: Endpoint = format!("ws://{}", server.address()).parse().unwrap();
    let mut clients: Vec<_> = ["Paused reader", "Active reader"]
        .into_iter()
        .map(|name| {
            Client::connect(
                &fixture.0,
                endpoint.clone(),
                name.into(),
                Assists::default(),
                None,
            )
            .unwrap()
        })
        .collect();
    let mut races = vec![None, None];
    pump(&mut server, &mut clients, &mut races, &fixture.0, 100);
    assert_eq!(server.lobby.players.len(), 2);
    // Enough ACK traffic to fill the paused client's bounded raw-packet queue.
    let dt = Duration::from_millis(1);
    for _ in 0..1200 {
        server.update(dt).unwrap();
        clients[1].poll(dt).unwrap();
        std::thread::sleep(Duration::from_micros(200));
    }
    clients[1].send(ClientMessage::Ready(true)).unwrap();
    pump(&mut server, &mut clients, &mut races, &fixture.0, 150);
    assert_eq!(server.lobby.players.len(), 2);
    for client in &clients {
        assert!(!client.closed);
        assert!(client
            .lobby
            .as_ref()
            .unwrap()
            .players
            .iter()
            .any(|p| p.id == clients[1].id && p.ready));
    }
}

#[test]
fn every_driver_learns_every_other_drivers_ping() {
    let fixture = Fixture::new();
    let mut server = Server::bind(
        &fixture.0,
        "127.0.0.1:0".parse().unwrap(),
        RaceSettings::default(),
        None,
    )
    .unwrap();
    let mut clients = vec![
        join(&fixture, &server, "First", None),
        join(&fixture, &server, "Second", None),
    ];
    let mut races = vec![None, None];
    pump(&mut server, &mut clients, &mut races, &fixture.0, 80);
    assert_eq!(server.lobby.players.len(), 2);
    // The room shares round-trip times once a second.
    pump(&mut server, &mut clients, &mut races, &fixture.0, 400);
    let ids: Vec<_> = clients.iter().map(|c| c.id).collect();
    for client in &clients {
        for &id in &ids {
            assert!(
                client.pings.contains_key(&id),
                "{} has no ping for {id}",
                client.id
            );
            assert!(client.ping_ms(id).is_some());
        }
    }
    let server_ids: Vec<_> = server.pings().iter().map(|(id, _)| *id).collect();
    assert_eq!(server_ids.len(), 2);
}

#[test]
fn a_slow_frame_rate_does_not_inflate_ping() {
    let fixture = Fixture::new();
    let mut server = Server::bind(
        &fixture.0,
        "127.0.0.1:0".parse().unwrap(),
        RaceSettings::default(),
        None,
    )
    .unwrap();
    let mut clients = vec![join(&fixture, &server, "Slow", None)];
    let mut races = vec![None];
    pump(&mut server, &mut clients, &mut races, &fixture.0, 80);
    assert_eq!(server.lobby.players.len(), 1);
    // The game thread only polls ten times a second, as at 10 fps. The server
    // keeps its real-time loop.
    let started = Instant::now();
    let mut last_poll = Instant::now();
    let mut previous = Instant::now();
    while started.elapsed() < Duration::from_secs(3) {
        let now = Instant::now();
        server.update(now - previous).unwrap();
        previous = now;
        if last_poll.elapsed() >= Duration::from_millis(100) {
            clients[0].poll(last_poll.elapsed()).unwrap();
            last_poll = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let id = clients[0].id;
    let (_, ping) = server.pings().into_iter().find(|(p, _)| *p == id).unwrap();
    assert!(
        ping < 40,
        "a 10 fps client measured {ping} ms on loopback; the frame rate leaked into the ping"
    );
}
