use lankide_provider_feishu::http::{Method, Request, ReqwestTransport, Transport};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

// A real HTTP/1.1 IM endpoint keeps connections open across requests.
#[test]
fn production_transport_delivers_sequential_and_concurrent_im_requests() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!(
        "http://{}/botTEST_SECRET/reaction",
        listener.local_addr().unwrap()
    );
    let stopped = Arc::new(AtomicBool::new(false));
    let flag = stopped.clone();
    let server = std::thread::spawn(move || {
        let mut peers = Vec::new();
        while !flag.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((mut stream, _)) => peers.push(std::thread::spawn(move || {
                    stream.set_nonblocking(false).unwrap();
                    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    loop {
                        let mut line = String::new();
                        if !matches!(reader.read_line(&mut line), Ok(n) if n > 0) { break; }
                        let mut length = 0;
                        loop {
                            line.clear();
                            if !matches!(reader.read_line(&mut line), Ok(n) if n > 0) { return; }
                            if line == "\r\n" { break; }
                            if let Some((key, value)) = line.split_once(':') {
                                if key.eq_ignore_ascii_case("content-length") { length = value.trim().parse().unwrap(); }
                            }
                        }
                        let mut body = vec![0; length];
                        if reader.read_exact(&mut body).is_err() { break; }
                        if stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Type: application/json\r\nConnection: keep-alive\r\n\r\n{}").is_err() { break; }
                    }
                })),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(1)),
                Err(error) => panic!("accept: {error}"),
            }
        }
        for peer in peers {
            peer.join().unwrap();
        }
    });
    let transport = ReqwestTransport::new();
    let mut failures = Vec::new();
    for index in 0..30 {
        if let Err(error) = transport
            .execute(Request::new(Method::Post, &url).body(b"{}".to_vec(), "application/json"))
        {
            failures.push(format!("sequential request {index}: {error}"));
        }
    }
    std::thread::scope(|scope| {
        let workers = (0..12)
            .map(|worker| {
                let transport = transport.clone();
                let url = &url;
                scope.spawn(move || {
                    let mut errors = Vec::new();
                    for index in 0..30 {
                        match transport.execute(
                            Request::new(Method::Post, url)
                                .body(b"{}".to_vec(), "application/json"),
                        ) {
                            Ok(response) => assert_eq!(response.body, b"{}"),
                            Err(error) => {
                                errors.push(format!("worker {worker} request {index}: {error}"))
                            }
                        }
                    }
                    errors
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            failures.extend(worker.join().unwrap());
        }
    });
    drop(transport);
    stopped.store(true, Ordering::Release);
    server.join().unwrap();
    assert!(
        failures.is_empty(),
        "IM requests failed ({}): {:?}",
        failures.len(),
        &failures[..failures.len().min(8)]
    );
}

#[test]
fn production_transport_preserves_failure_details_without_bot_urls() {
    for body_failure in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://{}/botTEST_SECRET/reaction",
            listener.local_addr().unwrap()
        );
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
            }
            if body_failure {
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nshort")
                    .unwrap();
            }
        });
        let error = ReqwestTransport::new()
            .execute(Request::new(Method::Post, url))
            .unwrap_err()
            .to_string();
        server.join().unwrap();
        let prefix = if body_failure {
            "HTTP response body read failed: "
        } else {
            "HTTP request failed: "
        };
        assert!(
            error.starts_with(prefix) && error.len() > prefix.len(),
            "missing reqwest detail: {error}"
        );
        assert!(
            !error.contains("TEST_SECRET") && !error.contains("http://"),
            "URL leaked: {error}"
        );
    }
}
