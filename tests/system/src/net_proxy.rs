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
    cut_before: Option<&'static str>,
    discard_once: Option<&'static str>,
    frame_delay: std::time::Duration,
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
        let controls = state.clone();
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
                        let fault = controls.clone();
                        let count = count.clone();
                        connections.spawn(async move {
                            let Ok(server) = TcpStream::connect(upstream).await else { return };
                            count.fetch_add(1, Ordering::SeqCst);
                            struct Active(Arc<AtomicUsize>);
                            impl Drop for Active { fn drop(&mut self) { self.0.fetch_sub(1, Ordering::SeqCst); } }
                            let _active = Active(count);
                            let (cr, cw) = client.into_split(); let (sr, sw) = server.into_split();
                            tokio::select! {
                                _ = forward(cr, sw, rx.clone(), false, generation, fault.clone()) => {},
                                _ = forward(sr, cw, rx, true, generation, fault) => {},
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
    /// Drop a complete client WebSocket frame before forwarding any of it.
    pub fn cut_before_computer_frame(&self, kind: &'static str) {
        self.state.send_modify(|s| s.cut_before = Some(kind));
    }
    pub fn discard_computer_frame(&self, kind: &'static str) {
        self.state.send_modify(|s| s.discard_once = Some(kind));
    }
    pub fn delay_computer_frames(&self, delay: std::time::Duration) {
        self.state.send_modify(|s| s.frame_delay = delay);
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
    fault: watch::Sender<State>,
) {
    let mut buffer = [0; 16384];
    let mut pending = Vec::new();
    let mut upgraded = false;
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
        if reply {
            if writer.write_all(&buffer[..length]).await.is_err() {
                return;
            }
        } else {
            pending.extend_from_slice(&buffer[..length]);
            if !upgraded {
                let Some(end) = pending
                    .windows(4)
                    .position(|w| w == b"\r\n\r\n")
                    .map(|p| p + 4)
                else {
                    continue;
                };
                if writer.write_all(&pending[..end]).await.is_err() {
                    return;
                }
                pending.drain(..end);
                upgraded = true;
            }
            while let Some((end, payload)) = client_frame(&pending) {
                let current = *state.borrow();
                if current.cut_before.is_some_and(|kind| {
                    serde_json::from_slice::<serde_json::Value>(&payload)
                        .is_ok_and(|v| v["type"] == kind)
                }) {
                    fault.send_modify(|s| {
                        s.cut_before = None;
                        s.cut = true;
                        s.generation += 1;
                    });
                    return;
                }
                if current.discard_once.is_some_and(|kind| {
                    serde_json::from_slice::<serde_json::Value>(&payload)
                        .is_ok_and(|v| v["type"] == kind)
                }) {
                    fault.send_modify(|s| s.discard_once = None);
                    pending.drain(..end);
                    continue;
                }
                if !current.frame_delay.is_zero() {
                    tokio::time::sleep(current.frame_delay).await;
                }
                if writer.write_all(&pending[..end]).await.is_err() {
                    return;
                }
                pending.drain(..end);
            }
        }
    }
}

// Decode framing only at the test TCP boundary; forwarded bytes remain intact.
fn client_frame(bytes: &[u8]) -> Option<(usize, Vec<u8>)> {
    if bytes.len() < 2 {
        return None;
    }
    let (mut offset, length) = match bytes[1] & 127 {
        126 => (
            4,
            u16::from_be_bytes(bytes.get(2..4)?.try_into().ok()?) as usize,
        ),
        127 => (
            10,
            usize::try_from(u64::from_be_bytes(bytes.get(2..10)?.try_into().ok()?)).ok()?,
        ),
        n => (2, n as usize),
    };
    let mask = if bytes[1] & 128 != 0 {
        let mask: [u8; 4] = bytes.get(offset..offset + 4)?.try_into().ok()?;
        offset += 4;
        mask
    } else {
        [0; 4]
    };
    let end = offset.checked_add(length)?;
    let payload = bytes
        .get(offset..end)?
        .iter()
        .enumerate()
        .map(|(i, b)| b ^ mask[i % 4])
        .collect();
    Some((end, payload))
}
