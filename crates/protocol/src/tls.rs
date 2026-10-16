//! Certificate pinning shared by pairing and all subsequent Machine links.
use rustls::{
    DigitallySignedStruct, Error, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use sha2::{Digest, Sha256};
use std::{io, sync::Arc};

pub fn fingerprint(cert: &[u8]) -> String {
    Sha256::digest(cert)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Plaintext is restricted to local discovery. A pin requires TLS.
pub fn validate_url(url: &str, pin: Option<&str>) -> io::Result<()> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let request = url.into_client_request().map_err(io::Error::other)?;
    let uri = request.uri();
    match uri.scheme_str() {
        Some("wss") => {}
        Some("ws")
            if pin.is_none()
                && uri.host().is_some_and(|host| {
                    host.eq_ignore_ascii_case("localhost")
                        || host
                            .trim_matches(['[', ']'])
                            .parse::<std::net::IpAddr>()
                            .is_ok_and(|ip| ip.is_loopback())
                }) => {}
        _ => {
            return Err(io::Error::other(
                "Use a secure address to connect to another machine.",
            ));
        }
    }
    if let Some(pin) = pin {
        let _ = client_config(pin)?;
    }
    Ok(())
}

pub fn client_config(pin: &str) -> io::Result<rustls::ClientConfig> {
    if pin.len() != 64 || !pin.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(io::Error::other("Invalid certificate pin."));
    }
    Ok(rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(io::Error::other)?
    .dangerous()
    .with_custom_certificate_verifier(Arc::new(PinnedCertificate(pin.to_ascii_lowercase())))
    .with_no_client_auth())
}
#[derive(Debug)]
struct PinnedCertificate(String);
impl ServerCertVerifier for PinnedCertificate {
    fn verify_server_cert(
        &self,
        cert: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        if fingerprint(cert.as_ref()) != self.0 {
            return Err(Error::General(
                "Certificate pin mismatch. Check the copied command.".into(),
            ));
        }
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            signature,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            signature,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}
