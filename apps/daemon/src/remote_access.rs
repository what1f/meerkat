use meerkat_protocol::{
    Status,
    management::{RemoteAccess, RemoteAccessControl},
};
use meerkat_runtime::RuntimePeerAdapter;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{Mutex, oneshot};

#[derive(Serialize, Deserialize)]
struct Identity {
    certificate: Vec<u8>,
    key: Vec<u8>,
}
struct Listener {
    stop: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<Result<(), meerkat_protocol::link::LinkError>>,
}
struct State {
    access: RemoteAccess,
    listener: Option<Listener>,
}
pub(super) struct RemoteMachines {
    home: PathBuf,
    peer: RuntimePeerAdapter,
    secret: String,
    settings: meerkat_protocol::link::LinkSettings,
    default_port: u16,
    state: Mutex<State>,
}
impl RemoteMachines {
    pub async fn open(
        home: PathBuf,
        peer: RuntimePeerAdapter,
        secret: String,
        settings: meerkat_protocol::link::LinkSettings,
        default_port: u16,
    ) -> Result<Arc<Self>, Status> {
        let address: Option<String> = match fs::read(home.join("remote-access.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(internal)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(internal(error)),
        };
        let this = Arc::new(Self {
            home,
            peer,
            secret,
            settings,
            default_port,
            state: Mutex::new(State {
                access: RemoteAccess::default(),
                listener: None,
            }),
        });
        if address.is_some() {
            this.set(address).await?;
        }
        Ok(this)
    }
    pub async fn stop(&self) {
        let mut state = self.state.lock().await;
        if let Some(listener) = state.listener.take() {
            let _ = listener.stop.send(());
            let _ = listener.task.await;
        }
    }
    fn identity(&self) -> Result<Identity, Status> {
        let path = self.home.join("machine-link-identity.json");
        match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(internal),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let rcgen::CertifiedKey { cert, signing_key } =
                    rcgen::generate_simple_self_signed(vec!["meerkat.local".into()])
                        .map_err(internal)?;
                let identity = Identity {
                    certificate: cert.der().to_vec(),
                    key: signing_key.serialize_der(),
                };
                private_write(&path, &serde_json::to_vec(&identity).map_err(internal)?)
                    .map_err(internal)?;
                Ok(identity)
            }
            Err(error) => Err(internal(error)),
        }
    }
}
#[async_trait::async_trait]
impl RemoteAccessControl for RemoteMachines {
    async fn enable(&self) -> Result<RemoteAccess, Status> {
        let access = self.get().await?;
        if access.address.is_some() {
            return Ok(access);
        }
        let Some(address) = primary_lan_ipv4() else {
            return Ok(access);
        };
        self.configure(Some(format!("{address}:{}", self.default_port)), true)
            .await
    }
    async fn get(&self) -> Result<RemoteAccess, Status> {
        Ok(self.state.lock().await.access.clone())
    }
    async fn set(&self, address: Option<String>) -> Result<RemoteAccess, Status> {
        self.configure(address, false).await
    }
}
impl RemoteMachines {
    async fn configure(
        &self,
        address: Option<String>,
        automatic: bool,
    ) -> Result<RemoteAccess, Status> {
        let mut address = address
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
        let port = address
            .as_deref()
            .map(|address| {
                if automatic {
                    Ok(self.default_port)
                } else {
                    validate_address(address)
                }
            })
            .transpose()?;
        let mut state = self.state.lock().await;
        if automatic && state.access.address.is_some() {
            return Ok(state.access.clone());
        }
        let mut next = RemoteAccess {
            address: address.clone(),
            cert_sha256: None,
        };
        let prepared = if let Some(port) = port {
            let identity = self.identity()?;
            next.cert_sha256 = Some(meerkat_protocol::tls::fingerprint(&identity.certificate));
            let config = rustls::ServerConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .map_err(internal)?
            .with_no_client_auth()
            .with_single_cert(
                vec![identity.certificate.into()],
                rustls::pki_types::PrivatePkcs8KeyDer::from(identity.key).into(),
            )
            .map_err(internal)?;
            // The advertised host may change while the port stays the same.
            // Reuse the listening socket in that case; existing links stay valid.
            if state
                .access
                .address
                .as_deref()
                .and_then(|a| validate_address(a).ok())
                == Some(port)
                && state.listener.is_some()
            {
                private_write(
                    &self.home.join("remote-access.json"),
                    &serde_json::to_vec(&address).map_err(internal)?,
                )
                .map_err(internal)?;
                state.access = next.clone();
                return Ok(next);
            }
            let socket = bind_all(port).map_err(|_| {
                Status::unavailable("This address cannot be used. Choose another port.")
            })?;
            if port == 0 {
                let host = address.as_deref().unwrap().rsplit_once(':').unwrap().0;
                address = Some(format!(
                    "{host}:{}",
                    socket.local_addr().map_err(internal)?.port()
                ));
                next.address = address.clone();
            }
            Some((socket, tokio_rustls::TlsAcceptor::from(Arc::new(config))))
        } else {
            None
        };
        private_write(
            &self.home.join("remote-access.json"),
            &serde_json::to_vec(&address).map_err(internal)?,
        )
        .map_err(internal)?;
        if let Some(listener) = state.listener.take() {
            let _ = listener.stop.send(());
            let _ = listener.task.await;
        }
        if let Some((socket, tls)) = prepared {
            let (stop, stopped) = oneshot::channel();
            let peer = self.peer.clone();
            let secret = self.secret.clone();
            let settings = self.settings.clone();
            let task = tokio::spawn(meerkat_protocol::resources::serve_configured(
                socket,
                secret,
                peer,
                settings,
                None,
                Some(tls),
                None,
                async {
                    let _ = stopped.await;
                },
            ));
            state.listener = Some(Listener { stop, task });
        }
        state.access = next.clone();
        Ok(next)
    }
}
fn validate_address(address: &str) -> Result<u16, Status> {
    let invalid = || Status::invalid_argument("Enter an address such as 192.168.1.20:7443.");
    let (host, port) = address.rsplit_once(':').ok_or_else(invalid)?;
    if host.is_empty()
        || host.starts_with('-')
        || !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-:[]".contains(&b))
    {
        return Err(invalid());
    }
    if host.contains(':') {
        if !host.starts_with('[')
            || !host.ends_with(']')
            || host[1..host.len() - 1]
                .parse::<std::net::Ipv6Addr>()
                .is_err()
        {
            return Err(invalid());
        }
    } else if host.contains(['[', ']'])
        || host
            .trim_end_matches('.')
            .split('.')
            .any(|label| label.is_empty() || label.starts_with('-') || label.ends_with('-'))
    {
        return Err(invalid());
    }
    let port: u16 = port.parse().map_err(|_| invalid())?;
    if port == 0 {
        return Err(invalid());
    }
    Ok(port)
}
fn internal(error: impl std::fmt::Display) -> Status {
    Status::internal(error.to_string())
}
fn private_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = path.with_extension(format!("tmp-{}-{nonce}", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        fs::File::open(path.parent().expect("home"))?.sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn bind_all(port: u16) -> io::Result<tokio::net::TcpListener> {
    let socket = socket2::Socket::new(
        socket2::Domain::IPV6,
        socket2::Type::STREAM,
        Some(socket2::Protocol::TCP),
    )?;
    socket.set_only_v6(false)?;
    socket.set_reuse_address(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&std::net::SocketAddr::new(std::net::Ipv6Addr::UNSPECIFIED.into(), port).into())?;
    socket.listen(128)?;
    tokio::net::TcpListener::from_std(socket.into())
}

/// Prefer the interface selected by the routing table. Enumerating interfaces
/// also covers LANs without a default route. UDP connect sends no packets.
fn primary_lan_ipv4() -> Option<std::net::Ipv4Addr> {
    let preferred = std::net::UdpSocket::bind("0.0.0.0:0")
        .ok()
        .and_then(|socket| {
            socket.connect("192.0.2.1:9").ok()?;
            match socket.local_addr().ok()?.ip() {
                std::net::IpAddr::V4(ip) => Some(ip),
                _ => None,
            }
        });
    let addresses: Vec<_> = nix::ifaddrs::getifaddrs()
        .ok()?
        .filter(|interface| {
            !["utun", "tun", "tap"]
                .iter()
                .any(|prefix| interface.interface_name.starts_with(prefix))
                && interface
                    .flags
                    .contains(nix::net::if_::InterfaceFlags::IFF_UP)
                && !interface.flags.intersects(
                    nix::net::if_::InterfaceFlags::IFF_LOOPBACK
                        | nix::net::if_::InterfaceFlags::IFF_POINTOPOINT,
                )
        })
        .filter_map(|interface| {
            interface
                .address?
                .as_sockaddr_in()
                .map(|address| address.ip())
        })
        .filter(|ip| {
            let octets = ip.octets();
            !ip.is_loopback()
                && !ip.is_unspecified()
                && !ip.is_link_local()
                && !(octets[0] == 198 && matches!(octets[1], 18 | 19))
        })
        .collect();
    preferred
        .filter(|ip| ip.is_private() && addresses.contains(ip))
        .or_else(|| addresses.iter().copied().find(|ip| ip.is_private()))
        .or_else(|| preferred.filter(|ip| addresses.contains(ip)))
        .or_else(|| addresses.first().copied())
}
