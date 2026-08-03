// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ConicMC contributors

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::Arc;
use std::thread::JoinHandle;

use parking_lot::Mutex;
use tokio::runtime::{Handle, Runtime};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::command::Command;
use crate::event::Event;
use crate::flow;
use crate::room::Room;
use crate::scaffolding::server::ServerHandle;
use crate::session::{Session, SessionError, SessionStateId};

pub const EVENT_QUEUE_CAPACITY: usize = 256;

#[derive(Debug, Clone)]
pub struct ContextConfig {
    pub public_nodes: Vec<String>,
    pub data_dir: PathBuf,
    pub motd: Option<String>,
}

impl Default for ContextConfig {
    fn default() -> Self {
        ContextConfig {
            public_nodes: Vec::new(),
            data_dir: std::env::temp_dir().join("conic-terracotta"),
            motd: None,
        }
    }
}

#[derive(Clone)]
pub struct Emitter {
    session: Arc<Mutex<Session>>,
    tx: SyncSender<Event>,
    next_seq: Arc<AtomicU64>,
    dropped: Arc<AtomicU64>,
    shutdown: Arc<AtomicBool>,
    config: Arc<Mutex<ContextConfig>>,
    scaffolding_port: Arc<AtomicU16>,
}

impl Emitter {
    pub fn emit(&self, event: Event) {
        self.next_seq.fetch_add(1, Ordering::Relaxed);
        match self.tx.try_send(event) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Full(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {}
        }
    }

    pub fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::Acquire)
    }

    pub fn dropped_count(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    pub(crate) fn next_sequence(&self) -> u64 {
        self.next_seq.load(Ordering::Relaxed)
    }

    pub fn scaffolding_port(&self) -> u16 {
        self.scaffolding_port.load(Ordering::Acquire)
    }

    pub(crate) fn set_scaffolding_port(&self, port: u16) {
        self.scaffolding_port.store(port, Ordering::Release);
    }

    pub fn can_capture(&self, token: u64) -> bool {
        self.session.lock().can_capture(token)
    }

    pub fn config(&self) -> ContextConfig {
        self.config.lock().clone()
    }

    pub fn session(&self) -> &Arc<Mutex<Session>> {
        &self.session
    }

    pub fn commit_shared(&self, f: impl FnOnce(&mut Session)) {
        let (state, version) = {
            let mut s = self.session.lock();
            f(&mut s);
            let version = s.commit_shared();
            (s.state, version)
        };
        self.emit(Event::StateChanged { state, version });
    }

    pub fn set_exception(&self, token: u64, error: SessionError) -> bool {
        self.mutate(token, |s| {
            s.state = SessionStateId::Exception;
            s.error = Some(error);
        })
    }

    pub fn transition(
        &self,
        token: u64,
        f: impl FnOnce(&mut Session) -> SessionStateId,
    ) -> Option<u64> {
        let (state, version) = {
            let mut s = self.session.lock();
            if !s.can_capture(token) {
                return None;
            }
            let state = f(&mut s);
            s.state = state;
            let version = s.commit();
            (state, version)
        };
        self.emit(Event::StateChanged { state, version });
        Some(version)
    }

    pub fn mutate(&self, token: u64, f: impl FnOnce(&mut Session)) -> bool {
        let (state, version) = {
            let mut s = self.session.lock();
            if !s.can_capture(token) {
                return false;
            }
            f(&mut s);
            let version = s.commit();
            (s.state, version)
        };
        self.emit(Event::StateChanged { state, version });
        true
    }

    pub fn mutate_shared(&self, token: u64, f: impl FnOnce(&mut Session)) -> bool {
        let (state, version) = {
            let mut s = self.session.lock();
            if !s.can_capture(token) {
                return false;
            }
            f(&mut s);
            let version = s.commit_shared();
            (s.state, version)
        };
        self.emit(Event::StateChanged { state, version });
        true
    }

    pub fn reset_to_waiting(&self) -> bool {
        let changed = self.reset_to_waiting_quietly();
        if changed {
            let (state, version) = {
                let s = self.session.lock();
                (s.state, s.version())
            };
            self.emit(Event::StateChanged { state, version });
        }
        changed
    }

    pub fn reset_to_waiting_quietly(&self) -> bool {
        let mut s = self.session.lock();
        if s.state == SessionStateId::Waiting {
            return false;
        }
        s.reset_to_waiting();
        true
    }
}

pub struct TerracottaContext {
    _runtime: Arc<Runtime>,
    command_tx: UnboundedSender<Command>,
    emitter: Emitter,
    events_rx: Mutex<Receiver<Event>>,
    runtime_thread: Mutex<Option<JoinHandle<()>>>,
    server_handle: ServerHandle,
}

impl TerracottaContext {
    pub fn create(config: ContextConfig) -> std::io::Result<Arc<TerracottaContext>> {
        crate::easytier::initialize(&config.data_dir)?;

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("conic-terracotta-worker")
            .build()?;

        let (command_tx, command_rx) = tokio::sync::mpsc::unbounded_channel::<Command>();
        let (events_tx, events_rx) = mpsc::sync_channel::<Event>(EVENT_QUEUE_CAPACITY);

        let emitter = Emitter {
            session: Arc::new(Mutex::new(Session::waiting())),
            tx: events_tx,
            next_seq: Arc::new(AtomicU64::new(1)),
            dropped: Arc::new(AtomicU64::new(0)),
            shutdown: Arc::new(AtomicBool::new(false)),
            config: Arc::new(Mutex::new(config)),
            scaffolding_port: Arc::new(AtomicU16::new(0)),
        };

        let (scaffolding_port, server_handle) = {
            let (port, server_handle) = match crate::scaffolding::server::start(
                crate::scaffolding::protocols::build_handlers(&emitter),
                13448,
            ) {
                Ok(value) => value,
                Err(_) => crate::scaffolding::server::start(
                    crate::scaffolding::protocols::build_handlers(&emitter),
                    0,
                )
                .unwrap(),
            };
            (port, server_handle)
        };
        emitter.set_scaffolding_port(scaffolding_port);

        let runtime = Arc::new(runtime);
        let rt = runtime.clone();
        let handle = rt.handle().clone();
        let thread_emitter = emitter.clone();
        let runtime_thread = std::thread::Builder::new()
            .name("conic-terracotta".into())
            .spawn(move || {
                rt.block_on(run_main(thread_emitter, command_rx, handle));
            })?;

        let ctx = TerracottaContext {
            _runtime: runtime,
            command_tx,
            emitter,
            events_rx: Mutex::new(events_rx),
            runtime_thread: Mutex::new(Some(runtime_thread)),
            server_handle,
        };

        Ok(Arc::new(ctx))
    }

    pub fn set_config(&self, config: ContextConfig) {
        let mut guard = self.emitter.config.lock();
        let active = {
            let session = self.emitter.session.lock();
            session.state() != SessionStateId::Waiting
        };
        if active {
            return;
        }
        let result = crate::easytier::initialize(&config.data_dir);
        if let Err(e) = result {
            logging!("Easytier", "Failed to relocate easytier to {}: {e}", config.data_dir.to_string_lossy());
        }
        *guard = config;
    }

    pub fn emitter(&self) -> Emitter {
        self.emitter.clone()
    }

    pub fn poll_event(&self) -> Option<Event> {
        self.events_rx.lock().try_recv().ok()
    }

    pub fn session_snapshot(&self) -> crate::snapshot::Snapshot {
        let session = self.emitter.session.lock();
        crate::snapshot::Snapshot::from(&session)
    }

    pub fn request_shutdown(&self) {
        self.emitter.shutdown.store(true, Ordering::Release);
        let _ = self.command_tx.send(Command::Shutdown);
    }

    pub fn destroy(&self) {
        // Kill any running EasyTier child process and stop the FakeServer
        // first, and invalidate the flow token so in-flight flows return.
        self.emitter.reset_to_waiting_quietly();
        // Stop the scaffolding server's accept loop.
        self.server_handle.shutdown();
        self.request_shutdown();
        if let Some(thread) = self.runtime_thread.lock().take() {
            let _ = thread.join();
        }
    }

    pub fn submit_create_room(
        &self,
        player_name: Option<String>,
        room_code: Option<String>,
        public_nodes: Vec<String>,
    ) {
        let _ = self.command_tx.send(Command::CreateRoom {
            player_name,
            room_code,
            public_nodes,
        });
    }

    pub fn submit_join_room(
        &self,
        room_code: String,
        player_name: Option<String>,
        public_nodes: Vec<String>,
    ) {
        let _ = self.command_tx.send(Command::JoinRoom {
            room_code,
            player_name,
            public_nodes,
        });
    }

    pub fn submit_set_waiting(&self) {
        let _ = self.command_tx.send(Command::SetWaiting);
    }
}

async fn run_main(emitter: Emitter, mut command_rx: UnboundedReceiver<Command>, handle: Handle) {
    loop {
        if emitter.is_shutdown() {
            break;
        }
        let Some(command) = command_rx.recv().await else {
            break;
        };
        if emitter.is_shutdown() {
            break;
        }
        if handle_command(&emitter, command, &handle) {
            break;
        }
    }
}

fn handle_command(emitter: &Emitter, command: Command, handle: &Handle) -> bool {
    match command {
        Command::Shutdown => true,
        Command::SetWaiting => {
            emitter.reset_to_waiting();
            false
        }
        Command::CreateRoom {
            player_name,
            room_code,
            public_nodes,
        } => {
            let Some(room) =
                room_code.as_deref().map(Room::parse).unwrap_or(Some(Room::create()))
            else {
                emitter.emit(Event::Error {
                    code: crate::ErrorCode::InvalidRoomCode as u32,
                    message: format!("invalid room code: {:?}", room_code),
                });
                return false;
            };

            let token = {
                let mut session = emitter.session.lock();
                if session.state() != SessionStateId::Waiting {
                    emitter.emit(Event::Error {
                        code: crate::ErrorCode::Busy as u32,
                        message: "a session is already active".into(),
                    });
                    return false;
                }
                session.room = Some(room.clone());
                session.port = None;
                session.state = SessionStateId::HostScanning;
                session.commit()
            };

            let (state, version) = {
                let session = emitter.session.lock();
                (session.state(), session.version())
            };
            emitter.emit(Event::StateChanged { state, version });

            let running = emitter.clone();
            let config = emitter.config();
            handle.spawn_blocking(move || {
                flow::run_host(
                    running,
                    token,
                    room,
                    player_name,
                    public_nodes,
                    config,
                );
            });
            false
        }
        Command::JoinRoom {
            room_code,
            player_name,
            public_nodes,
        } => {
            let Some(room) = Room::parse(&room_code) else {
                emitter.emit(Event::Error {
                    code: crate::ErrorCode::InvalidRoomCode as u32,
                    message: format!("invalid room code: {room_code}"),
                });
                return false;
            };

            let token = {
                let mut session = emitter.session.lock();
                if session.state() != SessionStateId::Waiting {
                    emitter.emit(Event::Error {
                        code: crate::ErrorCode::Busy as u32,
                        message: "a session is already active".into(),
                    });
                    return false;
                }
                session.room = Some(room.clone());
                session.port = None;
                session.state = SessionStateId::GuestConnecting;
                session.commit()
            };

            let (state, version) = {
                let session = emitter.session.lock();
                (session.state(), session.version())
            };
            emitter.emit(Event::StateChanged { state, version });

            let running = emitter.clone();
            let config = emitter.config();
            handle.spawn_blocking(move || {
                flow::run_guest(
                    running,
                    token,
                    room,
                    player_name,
                    public_nodes,
                    config,
                );
            });
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn create() -> Arc<TerracottaContext> {
        TerracottaContext::create(ContextConfig::default()).unwrap()
    }

    #[test]
    fn lifecycle_poll_events() {
        let ctx = create();
        ctx.submit_create_room(None, None, vec![]);

        let mut saw_state = false;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            while let Some(event) = ctx.poll_event() {
                if let Event::StateChanged { .. } = event {
                    saw_state = true;
                }
            }
            if saw_state {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(saw_state, "expected at least one StateChanged event");

        let snapshot = ctx.session_snapshot();
        assert_eq!(snapshot.state, SessionStateId::HostScanning);

        ctx.submit_set_waiting();
        std::thread::sleep(Duration::from_millis(50));

        let snapshot = ctx.session_snapshot();
        assert_eq!(snapshot.state, SessionStateId::Waiting);

        while ctx.poll_event().is_some() {}

        ctx.destroy();
        assert!(ctx.poll_event().is_none());
    }

    #[test]
    fn event_queue_bounded_and_droppable() {
        let ctx = create();
        for _ in 0..(EVENT_QUEUE_CAPACITY * 4) {
            ctx.emitter().emit(Event::Error {
                code: 0,
                message: "x".into(),
            });
        }
        assert!(ctx.emitter().dropped_count() > 0);

        let mut drained = 0;
        while ctx.poll_event().is_some() {
            drained += 1;
        }
        assert_eq!(drained, EVENT_QUEUE_CAPACITY);
        ctx.destroy();
    }

    #[test]
    fn waiting_invalidates_running_flow() {
        let ctx = create();
        ctx.submit_create_room(None, None, vec![]);
        std::thread::sleep(Duration::from_millis(100));

        ctx.submit_set_waiting();
        std::thread::sleep(Duration::from_millis(100));

        let snapshot = ctx.session_snapshot();
        assert_eq!(snapshot.state, SessionStateId::Waiting);
        ctx.destroy();
    }

    #[test]
    fn destroy_stops_scaffolding_server() {
        let ctx = create();
        let port = ctx.emitter().scaffolding_port();
        assert_ne!(port, 0, "scaffolding server must be listening");

        let connection = std::net::TcpStream::connect(("127.0.0.1", port));
        assert!(connection.is_ok(), "scaffolding server must accept connections");
        drop(connection);

        ctx.destroy();

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            match std::net::TcpStream::connect(("127.0.0.1", port)) {
                Err(_) => break,
                Ok(_) => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "scaffolding server still accepting after destroy"
                    );
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        }
    }

    #[test]
    fn destroy_kills_easytier_child() {
        let ctx = create();
        let emitter = ctx.emitter();

        let data_dir = std::env::temp_dir().join(format!(
            "conic-terracotta-destroy-kill-test-{}",
            std::process::id()
        ));
        let easytier = crate::easytier::create(
            &data_dir,
            vec![
                crate::easytier::argument::Argument::NoTun,
                crate::easytier::argument::Argument::NetworkName(
                    "conic-terracotta-destroy-kill-test".into(),
                ),
                crate::easytier::argument::Argument::NetworkSecret("kill-test".into()),
                crate::easytier::argument::Argument::P2POnly,
            ],
        );

        {
            let mut session = emitter.session().lock();
            session.easytier = Some(easytier);
            session.state = SessionStateId::HostStarting;
            session.commit();
        }
        assert!(
            emitter.session().lock().easytier.as_ref().unwrap().is_alive(),
            "easytier child must be alive before destroy"
        );

        ctx.destroy();

        let session = emitter.session().lock();
        assert!(
            session.easytier.is_none(),
            "destroy must reset the session so the EasyTier child is killed"
        );
        assert_eq!(session.state, SessionStateId::Waiting);
    }
}
