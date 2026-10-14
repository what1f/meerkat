//! Test-only network boundary. Cuts after a byte count, never after a timer.
use lankide_system_e2e::Result;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};

#[derive(Clone, Debug)]
pub struct Download {
    pub execution: String,
    pub attempt: String,
    pub token: String,
    pub range: Option<String>,
}
pub struct Cutter {
    pub url: String,
    downloads: Arc<Mutex<Vec<Download>>>,
    task: JoinHandle<()>,
}
impl Cutter {
    pub async fn start(upstream: &str, cut_after: Option<usize>, corrupt: bool) -> Result<Self> {
        let upstream = reqwest::Url::parse(upstream)?;
        let address = format!(
            "{}:{}",
            upstream.host_str().ok_or("missing host")?,
            upstream.port_or_known_default().ok_or("missing port")?
        );
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("ws://{}", listener.local_addr()?);
        let downloads = Arc::new(Mutex::new(Vec::new()));
        let records = downloads.clone();
        let cut = Arc::new(AtomicBool::new(false));
        let task = tokio::spawn(async move {
            let mut tasks = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((mut client, _)) = accepted else { break };
                        let address = address.clone(); let records = records.clone(); let cut = cut.clone();
                        tasks.spawn(async move {
                            let result: Result<()> = async {
                                let header = headers(&mut client).await?;
                                let text = String::from_utf8(header.clone())?;
                                let mut server = TcpStream::connect(address).await?;
                                server.write_all(&header).await?;
                                if text.starts_with("GET /resources/") {
                                    let fields: Vec<_> = text.lines().filter_map(|line| line.split_once(':')).map(|(key,value)| (key.to_ascii_lowercase(), value.trim().to_owned())).collect();
                                    let get = |name: &str| fields.iter().find(|(key,_)| key == name).map(|(_,value)| value.clone());
                                    records.lock().unwrap().push(Download { execution: get("x-lankide-execution").unwrap_or_default(), attempt: get("x-lankide-attempt").unwrap_or_default(), token: get("authorization").unwrap_or_default().trim_start_matches("Bearer ").to_owned(), range: get("range") });
                                    let header = headers(&mut server).await?;
                                    client.write_all(&header).await?;
                                    if let Some(count) = cut_after.filter(|_| !cut.swap(true, Ordering::SeqCst)) {
                                        let mut limited = (&mut server).take(count as u64);
                                        let sent = tokio::io::copy(&mut limited, &mut client).await?;
                                        if sent != count as u64 { return Err("response too small to cut".into()); }
                                        client.shutdown().await?;
                                        return Ok(());
                                    }
                                    if corrupt {
                                        let mut byte = [0];
                                        server.read_exact(&mut byte).await?;
                                        byte[0] ^= 1;
                                        client.write_all(&byte).await?;
                                    }
                                }
                                tokio::io::copy_bidirectional(&mut client, &mut server).await?;
                                Ok(())
                            }.await;
                            if let Err(error) = result { eprintln!("resource cutter: {error}"); }
                        });
                    }
                    result = tasks.join_next(), if !tasks.is_empty() => { let _ = result; }
                }
            }
        });
        Ok(Self {
            url,
            downloads,
            task,
        })
    }
    pub fn downloads(&self) -> Vec<Download> {
        self.downloads.lock().unwrap().clone()
    }
}
impl Drop for Cutter {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn headers(socket: &mut TcpStream) -> Result<Vec<u8>> {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let mut bytes = Vec::new();
        loop {
            bytes.push(socket.read_u8().await?);
            if bytes.ends_with(b"\r\n\r\n") {
                return Ok(bytes);
            }
            if bytes.len() > 32 * 1024 {
                return Err("HTTP headers too large".into());
            }
        }
    })
    .await?
}
