use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use guardian_core::snapshot::HealthProbe;

fn os_roots() -> Result<ureq::tls::RootCerts, String> {
    let found = rustls_native_certs::load_native_certs();
    let certs: Vec<ureq::tls::Certificate<'static>> = found.certs.iter().map(|c| ureq::tls::Certificate::from_der(c.as_ref()).to_owned()).collect();
    if certs.is_empty() {
        return Err("no trusted root certificates found on this computer".into());
    }
    Ok(ureq::tls::RootCerts::new_with_certs(&certs))
}

/// Anonymous `GET {server}/health` (spec R7). Never disables certificate checks.
pub fn probe(server: &str, timeout: Duration) -> HealthProbe {
    let url = format!("{}/health", server.trim_end_matches('/'));
    let mut cfg = ureq::Agent::config_builder().timeout_global(Some(timeout)).http_status_as_error(false);
    if url.starts_with("https://") {
        match os_roots() {
            Ok(roots) => cfg = cfg.tls_config(ureq::tls::TlsConfig::builder().root_certs(roots).build()),
            Err(e) => return HealthProbe { status: None, latency_ms: 0, server_date: None, error: Some(e) },
        }
    }
    let agent = cfg.build().new_agent();
    let start = Instant::now();
    match agent.get(&url).header("User-Agent", concat!("Guardian/", env!("CARGO_PKG_VERSION"))).call() {
        Ok(resp) => {
            let server_date = resp
                .headers()
                .get("date")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| DateTime::parse_from_rfc2822(s).ok())
                .map(|d| d.with_timezone(&Utc));
            let status = resp.status().as_u16();
            HealthProbe {
                status: Some(status),
                latency_ms: start.elapsed().as_millis() as u64,
                server_date,
                error: (status != 200).then(|| format!("HTTP {status}")),
            }
        }
        Err(e) => HealthProbe { status: None, latency_ms: start.elapsed().as_millis() as u64, server_date: None, error: Some(e.to_string()) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read as _, Write as _};

    fn serve_once(resp: &'static str) -> String {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = l.accept() {
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf);
                let _ = s.write_all(resp.as_bytes());
            }
        });
        format!("http://{addr}")
    }

    #[test]
    fn probe_ok_with_date() {
        let url = serve_once("HTTP/1.1 200 OK\r\nDate: Wed, 14 Oct 2026 10:00:00 GMT\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
        let h = probe(&url, Duration::from_secs(5));
        assert_eq!(h.status, Some(200));
        assert_eq!(h.server_date.unwrap().to_rfc3339(), "2026-10-14T10:00:00+00:00");
        assert_eq!(h.error, None);
    }

    #[test]
    fn probe_503_and_refused() {
        let url = serve_once("HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        let h = probe(&url, Duration::from_secs(5));
        assert_eq!(h.status, Some(503));
        let h = probe("http://127.0.0.1:1", Duration::from_secs(2));
        assert_eq!(h.status, None);
        assert!(h.error.is_some());
    }
}
