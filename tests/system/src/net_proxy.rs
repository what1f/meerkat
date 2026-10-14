//! Test-only TCP boundary shared by network business scenarios.
use crate::Result;
use std::{
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::watch,
    task::JoinHandle,
    time::Instant,
};

#[derive(Clone, Copy, Default)]
struct State {
    paused: bool,
    replies_paused: bool,
    cut: bool,
    generation: u64,
}
pub struct NetProxy {
    address: SocketAddr,
    state: watch::Sender<State>,
    attempts: Arc<Mutex<Vec<Instant>>>,
    active: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}
impl NetProxy {
    pub async fn start(upstream: SocketAddr) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let (state, rx) = watch::channel(State::default());
        let attempts = Arc::new(Mutex::new(Vec::new()));
        let active = Arc::new(AtomicUsize::new(0));
        let recorded = attempts.clone();
        let count = active.clone();
        let task = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    _ = connections.join_next(), if !connections.is_empty() => {},
                    accepted = listener.accept() => {
                        let Ok((client, _)) = accepted else { break };
                        recorded.lock().expect("attempts").push(Instant::now());
                        if rx.borrow().cut { continue; }
                        let generation = rx.borrow().generation;
                        let rx = rx.clone();
                        let count = count.clone();
                        connections.spawn(async move {
                            let Ok(server) = TcpStream::connect(upstream).await else { return };
                            count.fetch_add(1, Ordering::SeqCst);
                            struct Active(Arc<AtomicUsize>);
                            impl Drop for Active { fn drop(&mut self) { self.0.fetch_sub(1, Ordering::SeqCst); } }
                            let _active = Active(count);
                            let (cr, cw) = client.into_split(); let (sr, sw) = server.into_split();
                            tokio::select! {
                                _ = forward(cr, sw, rx.clone(), false, generation) => {},
                                _ = forward(sr, cw, rx, true, generation) => {},
                            }
                        });
                    }
                }
            }
        });
        Ok(Self {
            address,
            state,
            attempts,
            active,
            task,
        })
    }
    pub fn address(&self) -> SocketAddr {
        self.address
    }
    pub fn url(&self) -> String {
        format!("ws://{}", self.address)
    }
    pub fn pause(&self) {
        self.state.send_modify(|s| s.paused = true);
    }
    pub fn pause_replies(&self) {
        self.state.send_modify(|s| s.replies_paused = true);
    }
    pub fn cut(&self) {
        self.state.send_modify(|s| {
            s.cut = true;
            s.generation += 1;
        });
    }
    pub fn restore(&self) {
        self.state.send_modify(|s| {
            s.cut = false;
            s.paused = false;
            s.replies_paused = false;
        });
    }
    pub fn attempts(&self) -> Vec<Instant> {
        self.attempts.lock().expect("attempts").clone()
    }
    pub fn active_connections(&self) -> usize {
        self.active.load(Ordering::SeqCst)
    }
}
impl Drop for NetProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn forward(
    mut reader: tokio::net::tcp::OwnedReadHalf,
    mut writer: tokio::net::tcp::OwnedWriteHalf,
    mut state: watch::Receiver<State>,
    reply: bool,
    generation: u64,
) {
    let mut buffer = [0; 16384];
    loop {
        let current = *state.borrow_and_update();
        if current.cut || current.generation != generation {
            return;
        }
        if current.paused || (reply && current.replies_paused) {
            if state.changed().await.is_err() {
                return;
            }
            continue;
        }
        let length = tokio::select! {
            _ = state.changed() => continue,
            read = reader.read(&mut buffer) => match read { Ok(0) | Err(_) => return, Ok(n) => n },
        };
        // Keep bytes already read while a pause begins; never discard stalled frames.
        loop {
            let current = *state.borrow_and_update();
            if current.cut || current.generation != generation {
                return;
            }
            if !current.paused && !(reply && current.replies_paused) {
                break;
            }
            if state.changed().await.is_err() {
                return;
            }
        }
        if writer.write_all(&buffer[..length]).await.is_err() {
            return;
        }
    }
}
