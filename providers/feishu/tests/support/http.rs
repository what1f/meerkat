//! Loopback HTTP peer. Unknown requests fail instead of escaping to the network.
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

#[derive(Clone, Debug)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
impl Request {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap()
    }
    pub fn header(&self, key: &str) -> &str {
        self.headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, value)| value.as_str())
            .unwrap_or("")
    }
    pub fn part(&self, name: &str) -> Vec<u8> {
        let boundary = self
            .header("content-type")
            .split("boundary=")
            .nth(1)
            .unwrap();
        let marker = format!("--{boundary}");
        let mut rest = self.body.as_slice();
        while let Some(start) = find(rest, marker.as_bytes()) {
            rest = &rest[start + marker.len()..];
            let Some(end) = find(rest, marker.as_bytes()) else {
                break;
            };
            let part = &rest[..end];
            let split = find(part, b"\r\n\r\n").unwrap();
            let headers = String::from_utf8_lossy(&part[..split]);
            if headers.contains(&format!("name=\"{name}\"")) {
                return part[split + 4..part.len() - 2].to_vec();
            }
        }
        panic!("missing multipart field {name}");
    }
}
fn find(bytes: &[u8], pattern: &[u8]) -> Option<usize> {
    bytes
        .windows(pattern.len())
        .position(|value| value == pattern)
}

pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}
impl Response {
    pub fn json(value: Value) -> Self {
        Self {
            status: 200,
            content_type: "application/json",
            body: serde_json::to_vec(&value).unwrap(),
        }
    }
    pub fn file(bytes: &[u8]) -> Self {
        Self {
            status: 200,
            content_type: "application/octet-stream",
            body: bytes.to_vec(),
        }
    }
}
pub struct Server {
    pub endpoint: String,
    stopped: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl Server {
    pub fn start(handler: impl Fn(Request) -> Response + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let stopped = Arc::new(AtomicBool::new(false));
        let flag = stopped.clone();
        let handler = Arc::new(handler);
        let thread = thread::spawn(move || {
            let mut requests = Vec::new();
            while !flag.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let handler = handler.clone();
                        requests.push(thread::spawn(move || serve(stream, handler.as_ref())));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("accept: {error}"),
                }
            }
            for request in requests {
                request.join().unwrap();
            }
        });
        Self {
            endpoint,
            stopped,
            thread: Some(thread),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        let result = self.thread.take().unwrap().join();
        if !thread::panicking() {
            result.unwrap();
        }
    }
}
fn serve(mut stream: TcpStream, handler: &impl Fn(Request) -> Response) {
    // Accepted sockets can inherit the listener's non-blocking mode on macOS.
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let mut reader = BufReader::new(&mut stream);
    let mut line = String::new();
    // Shutdown can cancel a poll before it sends any request bytes.
    if reader.read_line(&mut line).unwrap() == 0 {
        return;
    }
    let parts: Vec<_> = line.split_whitespace().collect();
    let method = parts[0].to_owned();
    let path = parts[1].to_owned();
    let mut headers = vec![];
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if line == "\r\n" {
            break;
        }
        let (name, value) = line.trim().split_once(':').unwrap();
        headers.push((name.to_owned(), value.trim().to_owned()));
    }
    let length = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| value.parse().unwrap())
        .unwrap_or(0);
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    let response = handler(Request {
        method,
        path,
        headers,
        body,
    });
    // Shutdown can also cancel a poll while its response is being written.
    let _ = write!(
        stream,
        "HTTP/1.1 {} OK\r\nContent-Length: {}\r\nContent-Type: {}\r\nConnection: close\r\n\r\n",
        response.status,
        response.body.len(),
        response.content_type
    );
    let _ = stream.write_all(&response.body);
}
