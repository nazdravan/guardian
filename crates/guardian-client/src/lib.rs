//! Client side of the Guardian 3.0 server connection: TLS with an optional
//! certificate pin, gRPC channels, the bearer-token interceptor, and the
//! WebSocket connector for agents.

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use rustls_pki_types::{CertificateDer, ServerName, UnixTime};
use sha2::{Digest, Sha256};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tonic::service::Interceptor;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::{Channel, Endpoint, Uri};

/// A connection could not be set up.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// The pin is not 64 hex characters.
    #[error("certificate pin must be 64 hex characters (SHA-256)")]
    BadPin,
    /// The URL is not `http://` or `https://host[:port]`.
    #[error("invalid server url: {0}")]
    Url(String),
    /// TLS configuration failed.
    #[error("tls: {0}")]
    Tls(String),
    /// Connecting failed.
    #[error("connect: {0}")]
    Connect(#[from] tonic::transport::Error),
    /// The token cannot be sent as a header.
    #[error("token contains characters not allowed in a header")]
    BadToken,
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn parse_pin(pin: &str) -> Result<[u8; 32], ClientError> {
    let bytes = hex::decode(pin.trim()).map_err(|_| ClientError::BadPin)?;
    bytes.try_into().map_err(|_| ClientError::BadPin)
}

/// Accepts exactly one certificate, identified by the SHA-256 of its DER, whatever
/// its chain or names. The handshake signature is still verified.
#[derive(Debug)]
struct PinVerifier {
    pin: [u8; 32],
    algs: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let got: [u8; 32] = Sha256::digest(end_entity.as_ref()).into();
        if got == self.pin {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General(
                "server certificate does not match the pinned SHA-256".into(),
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algs)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}

fn config(pin: Option<&str>, alpn: &[&[u8]]) -> Result<Arc<ClientConfig>, ClientError> {
    let provider = provider();
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| ClientError::Tls(e.to_string()))?;
    let mut cfg = match pin {
        Some(p) => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PinVerifier {
                pin: parse_pin(p)?,
                algs: provider.signature_verification_algorithms,
            }))
            .with_no_client_auth(),
        None => {
            let mut roots = RootCertStore::empty();
            for c in rustls_native_certs::load_native_certs().certs {
                let _ = roots.add(c);
            }
            builder.with_root_certificates(roots).with_no_client_auth()
        }
    };
    cfg.alpn_protocols = alpn.iter().map(|a| a.to_vec()).collect();
    Ok(Arc::new(cfg))
}

/// TLS config for gRPC (ALPN `h2`). With a pin, only the certificate whose SHA-256
/// (hex) equals `pin_sha256` is accepted; without one, the system roots are used.
pub fn rustls_config(pin_sha256: Option<&str>) -> Result<Arc<ClientConfig>, ClientError> {
    config(pin_sha256, &[b"h2"])
}

/// The WebSocket connector (ALPN `http/1.1`) for agents and test agents.
pub fn ws_connector(pin_sha256: Option<&str>) -> Result<tokio_tungstenite::Connector, ClientError> {
    Ok(tokio_tungstenite::Connector::Rustls(config(
        pin_sha256,
        &[b"http/1.1"],
    )?))
}

/// A gRPC channel to `server_url`. `https://` uses TLS (pinned if a pin is given);
/// plain `http://` only when the URL says so explicitly.
pub async fn connect(server_url: &str, pin_sha256: Option<&str>) -> Result<Channel, ClientError> {
    let uri: Uri = server_url
        .parse()
        .map_err(|_| ClientError::Url(server_url.into()))?;
    let host = uri
        .host()
        .ok_or_else(|| ClientError::Url(server_url.into()))?
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();
    match uri.scheme_str() {
        Some("http") => Ok(Endpoint::from_shared(server_url.to_string())?
            .connect()
            .await?),
        Some("https") => {
            let port = uri.port_u16().unwrap_or(443);
            let tls = TlsConnector::from(rustls_config(pin_sha256)?);
            let name = ServerName::try_from(host.clone())
                .map_err(|_| ClientError::Url(server_url.into()))?;
            // TLS happens in the connector, so tonic itself sees plain HTTP/2.
            let authority = uri
                .authority()
                .map(|a| a.as_str().to_string())
                .ok_or_else(|| ClientError::Url(server_url.into()))?;
            let endpoint = Endpoint::from_shared(format!("http://{authority}"))?;
            let channel = endpoint
                .connect_with_connector(tower::service_fn(move |_: Uri| {
                    let tls = tls.clone();
                    let name = name.clone();
                    let host = host.clone();
                    async move {
                        let tcp = TcpStream::connect((host.as_str(), port)).await?;
                        let _ = tcp.set_nodelay(true);
                        let stream = tls.connect(name, tcp).await?;
                        Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(stream))
                    }
                }))
                .await?;
            Ok(channel)
        }
        _ => Err(ClientError::Url(server_url.into())),
    }
}

/// Adds `authorization: Bearer <token>` to every request.
#[derive(Debug, Clone)]
pub struct TokenInterceptor {
    value: tonic::metadata::MetadataValue<tonic::metadata::Ascii>,
}

impl Interceptor for TokenInterceptor {
    fn call(&mut self, mut req: tonic::Request<()>) -> Result<tonic::Request<()>, tonic::Status> {
        req.metadata_mut()
            .insert("authorization", self.value.clone());
        Ok(req)
    }
}

/// A channel that authenticates every call with `token`.
pub type Authed = InterceptedService<Channel, TokenInterceptor>;

/// Wraps `channel` so every call carries `token`.
pub fn with_token(channel: Channel, token: &str) -> Result<Authed, ClientError> {
    let value = format!("Bearer {token}")
        .parse()
        .map_err(|_| ClientError::BadToken)?;
    Ok(InterceptedService::new(channel, TokenInterceptor { value }))
}
