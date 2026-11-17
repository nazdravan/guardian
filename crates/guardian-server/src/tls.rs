//! Server certificates: from files, or self-signed on first start.

use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use rustls::ServerConfig as RustlsConfig;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use sha2::{Digest, Sha256};

use crate::StartError;
use crate::config::TlsMode;

fn tls_err(e: impl std::fmt::Display) -> StartError {
    StartError::Tls(e.to_string())
}

/// SHA-256 (lowercase hex) of a DER certificate: the pin clients use.
pub fn fingerprint(der: &[u8]) -> String {
    hex::encode(Sha256::digest(der))
}

fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(path)?.write_all(data)
}

/// Creates `dir/cert.pem` + `dir/key.pem` unless both exist: a certificate for
/// `localhost`, this machine's hostname and `127.0.0.1`, valid for 10 years.
fn ensure_self_signed(dir: &Path) -> Result<(), StartError> {
    let (cert, key) = (dir.join("cert.pem"), dir.join("key.pem"));
    if cert.exists() && key.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    let mut names = vec!["localhost".to_string(), "127.0.0.1".to_string()];
    if let Ok(h) = gethostname::gethostname().into_string() {
        if !h.is_empty() && !names.contains(&h) {
            names.push(h);
        }
    }
    let mut params = rcgen::CertificateParams::new(names).map_err(tls_err)?;
    let year = 1970 + i32::try_from(crate::db::now_unix() / 31_556_952).unwrap_or(56);
    params.not_before = rcgen::date_time_ymd(year - 1, 1, 1);
    params.not_after = rcgen::date_time_ymd(year + 10, 1, 1);
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "Guardian Server");
    let kp = rcgen::KeyPair::generate().map_err(tls_err)?;
    let c = params.self_signed(&kp).map_err(tls_err)?;
    write_private(&key, kp.serialize_pem().as_bytes())?;
    std::fs::write(&cert, c.pem())?;
    tracing::info!("generated a self-signed certificate in {}", dir.display());
    Ok(())
}

/// The rustls config (ALPN `h2` + `http/1.1`) and the certificate pin for `mode`;
/// `None` for plaintext.
pub fn load(
    mode: &TlsMode,
    data_dir: &Path,
) -> Result<Option<(Arc<RustlsConfig>, String)>, StartError> {
    let (cert_path, key_path) = match mode {
        TlsMode::InsecurePlaintext => return Ok(None),
        TlsMode::Files { cert, key } => (cert.clone(), key.clone()),
        TlsMode::SelfSigned => {
            let dir = data_dir.join("tls");
            ensure_self_signed(&dir)?;
            (dir.join("cert.pem"), dir.join("key.pem"))
        }
    };
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(&cert_path)
        .map_err(|e| tls_err(format!("{}: {e}", cert_path.display())))?
        .collect::<Result<_, _>>()
        .map_err(|e| tls_err(format!("{}: {e}", cert_path.display())))?;
    let first = certs
        .first()
        .ok_or_else(|| tls_err(format!("{}: no certificate", cert_path.display())))?;
    let pin = fingerprint(first.as_ref());
    let key = PrivateKeyDer::from_pem_file(&key_path)
        .map_err(|e| tls_err(format!("{}: {e}", key_path.display())))?;
    let mut cfg =
        RustlsConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(tls_err)?
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .map_err(tls_err)?;
    cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(Some((Arc::new(cfg), pin)))
}
