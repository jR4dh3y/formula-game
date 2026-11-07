//! WebSocket I/O runs off the simulation/render thread. Each peer has bounded
//! packet queues with backpressure, so a full handoff queue never silently drops
//! a reliable Renet packet. Independent readers/writers avoid a full-duplex
//! backpressure deadlock.
use crate::{MAX_PLAYERS, PROTOCOL_ID, TIMEOUT};
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    serve::ListenerExt,
    Router,
};
use futures_util::{SinkExt, StreamExt};
use renet::{RenetClient, RenetServer};
use std::{
    collections::HashMap,
    future::Future,
    net::SocketAddr,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot, Semaphore};
use tokio_tungstenite::tungstenite::{protocol::WebSocketConfig, Message as ClientMessage};

const QUEUE: usize = 2048;
const SEND_CAPACITY_GUARD: usize = 128;
const MAX_PACKET: usize = 2048;
const CONNECTIONS: usize = MAX_PLAYERS + 4;

struct Worker {
    stop: Option<oneshot::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Worker {
    fn spawn(task: impl Future<Output = ()> + Send + 'static) -> Result<Self, String> {
        // Construct before spawning, so initialization failures reach the caller.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        let (stop, stopped) = oneshot::channel();
        let thread = thread::Builder::new()
            .name("racecraft-websocket".into())
            .spawn(move || {
                runtime.block_on(async {
                    tokio::select! { _ = task => {}, _ = stopped => {} }
                });
                // DNS resolution may use blocking OS calls; cancellation must not
                // hold the game window open while the resolver finishes.
                runtime.shutdown_timeout(Duration::from_millis(100));
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            stop: Some(stop),
            thread: Some(thread),
        })
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

enum Received {
    Connected,
    Packet(Vec<u8>),
    Error(String),
}
pub(crate) struct WsClient {
    incoming: mpsc::Receiver<Received>,
    outgoing: mpsc::Sender<Vec<u8>>,
    last_received: Instant,
    worker: Option<Worker>,
}
impl WsClient {
    pub fn connect(url: String, id: u64) -> Result<Self, String> {
        // TLS dependencies do not choose a crypto backend automatically.
        let _ = rustls::crypto::ring::default_provider().install_default();
        // Renet has already removed packets from its send queue when this
        // handoff happens. A bounded channel that silently drops on `Full`
        // therefore turns reliable packets into loss under jitter.
        let (events, incoming) = mpsc::channel(QUEUE);
        let (outgoing, mut packets) = mpsc::channel::<Vec<u8>>(QUEUE);
        let worker = Worker::spawn(async move {
            let session = async {
                let config = WebSocketConfig::default()
                    .write_buffer_size(0)
                    .max_write_buffer_size(MAX_PACKET * 2)
                    .max_message_size(Some(MAX_PACKET))
                    .max_frame_size(Some(MAX_PACKET));
                let (mut socket, _) = tokio::time::timeout(
                    TIMEOUT,
                    tokio_tungstenite::connect_async_with_config(url, Some(config), true),
                )
                .await
                .map_err(|_| "WebSocket connection timed out".to_owned())?
                .map_err(|e| format!("WebSocket connection failed: {e}"))?;
                let mut hello = PROTOCOL_ID.to_le_bytes().to_vec();
                hello.extend(id.to_le_bytes());
                socket
                    .send(ClientMessage::Binary(hello.into()))
                    .await
                    .map_err(|e| e.to_string())?;
                events
                    .send(Received::Connected)
                    .await
                    .map_err(|_| "WebSocket receive queue is closed".to_owned())?;
                let (mut writer, mut reader) = socket.split();
                let sending = async {
                    while let Some(packet) = packets.recv().await {
                        tokio::time::timeout(
                            TIMEOUT,
                            writer.send(ClientMessage::Binary(packet.into())),
                        )
                        .await
                        .map_err(|_| "WebSocket write timed out".to_owned())?
                        .map_err(|e| e.to_string())?;
                    }
                    Ok::<(), String>(())
                };
                let receiving = async {
                    while let Some(message) = reader.next().await {
                        match message.map_err(|e| e.to_string())? {
                            ClientMessage::Binary(bytes) => {
                                if events.send(Received::Packet(bytes.to_vec())).await.is_err() {
                                    return Ok(());
                                }
                            }
                            ClientMessage::Ping(_) | ClientMessage::Pong(_) => {}
                            _ => break,
                        }
                    }
                    Err("Server closed the WebSocket connection".into())
                };
                tokio::select! {
                    result = sending => result,
                    result = receiving => result,
                }
            }
            .await;
            if let Err(error) = session {
                eprintln!("WebSocket client: {error}");
                let _ = events.send(Received::Error(error)).await;
            }
        })?;
        Ok(Self {
            incoming,
            outgoing,
            last_received: Instant::now(),
            worker: Some(worker),
        })
    }
    pub fn update(&mut self, net: &mut RenetClient) -> Result<(), String> {
        for _ in 0..QUEUE {
            match self.incoming.try_recv() {
                Ok(Received::Connected) => net.set_connected(),
                Ok(Received::Packet(packet)) => {
                    net.process_packet(&packet);
                    self.last_received = Instant::now();
                }
                Ok(Received::Error(error)) => {
                    net.disconnect();
                    return Err(error);
                }
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    net.disconnect();
                    return Err("WebSocket connection ended".into());
                }
            }
        }
        if let Some(reason) = net.disconnect_reason() {
            return Err(reason.to_string());
        }
        Ok(())
    }
    pub fn send_packets(&mut self, net: &mut RenetClient) -> Result<(), String> {
        if net.is_connected() {
            // Do not pull packets out of Renet unless the socket handoff can
            // absorb a normal transport burst. Reliable packets remain in
            // Renet and are retried on the next pass when the writer catches
            // up.
            if self.outgoing.capacity() < SEND_CAPACITY_GUARD {
                return Ok(());
            }
            for packet in net.get_packets_to_send() {
                if self.outgoing.blocking_send(packet).is_err() {
                    return Err("WebSocket connection ended".into());
                }
            }
        }
        Ok(())
    }
    pub fn elapsed(&self) -> Duration {
        self.last_received.elapsed()
    }
    pub fn disconnect(&mut self) {
        self.worker.take();
    }
}

struct Peer {
    id: u64,
    incoming: mpsc::Receiver<Vec<u8>>,
    outgoing: mpsc::Sender<Vec<u8>>,
    last_received: Instant,
}
#[derive(Clone)]
struct ListenerState {
    accepted: mpsc::Sender<Peer>,
    slots: Arc<Semaphore>,
    heartbeat: Arc<Mutex<Instant>>,
}
async fn health(State(state): State<ListenerState>) -> StatusCode {
    if state.heartbeat.lock().unwrap().elapsed() < TIMEOUT {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}
async fn upgrade(State(state): State<ListenerState>, ws: WebSocketUpgrade) -> Response {
    let Ok(slot) = state.slots.clone().try_acquire_owned() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    ws.write_buffer_size(0)
        .max_write_buffer_size(MAX_PACKET * 2)
        .max_message_size(MAX_PACKET)
        .max_frame_size(MAX_PACKET)
        .on_upgrade(move |mut socket: WebSocket| async move {
            let _slot = slot;
            let Ok(Some(Ok(Message::Binary(hello)))) =
                tokio::time::timeout(TIMEOUT, socket.recv()).await
            else {
                return;
            };
            if hello.len() != 16 || hello[..8] != PROTOCOL_ID.to_le_bytes() {
                return;
            }
            let id = u64::from_le_bytes(hello[8..].try_into().unwrap());
            let (received, incoming) = mpsc::channel(QUEUE);
            let (outgoing, mut packets) = mpsc::channel::<Vec<u8>>(QUEUE);
            if state
                .accepted
                .try_send(Peer {
                    id,
                    incoming,
                    outgoing,
                    last_received: Instant::now(),
                })
                .is_err()
            {
                return;
            }
            let (mut writer, mut reader) = socket.split();
            let sending = async {
                while let Some(packet) = packets.recv().await {
                    if !matches!(
                        tokio::time::timeout(TIMEOUT, writer.send(Message::Binary(packet.into())))
                            .await,
                        Ok(Ok(()))
                    ) {
                        break;
                    }
                }
            };
            let receiving = async {
                while let Some(Ok(message)) = reader.next().await {
                    match message {
                        Message::Binary(bytes) => {
                            if received.send(bytes.to_vec()).await.is_err() {
                                break;
                            }
                        }
                        Message::Ping(_) | Message::Pong(_) => {}
                        _ => break,
                    }
                }
            };
            tokio::select! {
                _ = received.closed() => {},
                _ = sending => {},
                _ = receiving => {},
            }
        })
}

pub(crate) struct WsServer {
    pub address: SocketAddr,
    accepted: mpsc::Receiver<Peer>,
    peers: HashMap<u64, Peer>,
    heartbeat: Arc<Mutex<Instant>>,
    _worker: Worker,
}
impl WsServer {
    pub fn bind(address: SocketAddr) -> Result<Self, String> {
        let listener = std::net::TcpListener::bind(address).map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let address = listener.local_addr().map_err(|e| e.to_string())?;
        let (accepted, incoming) = mpsc::channel(CONNECTIONS);
        let heartbeat = Arc::new(Mutex::new(Instant::now()));
        let state = ListenerState {
            accepted,
            slots: Arc::new(Semaphore::new(CONNECTIONS)),
            heartbeat: heartbeat.clone(),
        };
        let worker = Worker::spawn(async move {
            let app = Router::new()
                .route("/health", get(health))
                .route("/ws", get(upgrade))
                .with_state(state);
            match tokio::net::TcpListener::from_std(listener) {
                Ok(listener) => {
                    let listener = listener.tap_io(|stream| {
                        if let Err(e) = stream.set_nodelay(true) {
                            eprintln!("WebSocket TCP_NODELAY: {e}");
                        }
                    });
                    if let Err(e) = axum::serve(listener, app).await {
                        eprintln!("WebSocket listener: {e}");
                    }
                }
                Err(e) => eprintln!("WebSocket listener: {e}"),
            }
        })?;
        Ok(Self {
            address,
            accepted: incoming,
            peers: HashMap::new(),
            heartbeat,
            _worker: worker,
        })
    }
    pub fn update(&mut self, net: &mut RenetServer) {
        *self.heartbeat.lock().unwrap() = Instant::now();
        for _ in 0..CONNECTIONS {
            let Ok(peer) = self.accepted.try_recv() else {
                break;
            };
            // A second socket with the same unauthenticated ID cannot replace the first.
            if self.peers.contains_key(&peer.id) || self.peers.len() >= CONNECTIONS {
                continue;
            }
            net.add_connection(peer.id);
            self.peers.insert(peer.id, peer);
        }
        self.peers.retain(|id, peer| {
            let mut live = net.is_connected(*id);
            for _ in 0..QUEUE {
                match peer.incoming.try_recv() {
                    Ok(packet) => {
                        let _ = net.process_packet_from(&packet, *id);
                        peer.last_received = Instant::now();
                    }
                    Err(mpsc::error::TryRecvError::Empty) => break,
                    Err(mpsc::error::TryRecvError::Disconnected) => {
                        live = false;
                        break;
                    }
                }
            }
            if peer.last_received.elapsed() > TIMEOUT {
                live = false;
            }
            if !live {
                if let Some(reason) = net.disconnect_reason(*id) {
                    eprintln!("WebSocket peer {id} disconnected: {reason}");
                }
                net.remove_connection(*id);
            }
            live
        });
    }
    pub fn elapsed(&self, id: u64) -> Option<Duration> {
        self.peers.get(&id).map(|p| p.last_received.elapsed())
    }
    pub fn send_packets(&mut self, net: &mut RenetServer) {
        self.peers.retain(|id, peer| {
            let live = net.is_connected(*id)
                && (peer.outgoing.capacity() < SEND_CAPACITY_GUARD
                    || net.get_packets_to_send(*id).is_ok_and(|packets| {
                        packets
                            .into_iter()
                            .all(|packet| peer.outgoing.blocking_send(packet).is_ok())
                    }));
            if !live {
                if let Some(reason) = net.disconnect_reason(*id) {
                    eprintln!("WebSocket peer {id} disconnected: {reason}");
                }
                net.remove_connection(*id);
            }
            live
        });
    }
    pub fn disconnect_all(&mut self, net: &mut RenetServer) {
        for id in self.peers.keys() {
            net.remove_connection(*id);
        }
        self.peers.clear();
    }
}
