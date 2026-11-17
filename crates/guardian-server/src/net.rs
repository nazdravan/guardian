//! The listener: one TCP port serving gRPC (HTTP/2) and WebSocket (HTTP/1.1 upgrade).

use std::net::SocketAddr;
use std::time::Duration;

use axum::extract::ConnectInfo;
use hyper::body::Incoming;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto;
use hyper_util::service::TowerToHyperService;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::task::JoinSet;
use tower::ServiceExt;

/// How long open connections get to finish after shutdown starts.
const GRACE: Duration = Duration::from_secs(2);

/// Serves `router` on `listener` until `shutdown` turns `true`.
pub async fn serve(
    listener: TcpListener,
    router: axum::Router,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut conns = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, peer) = match accepted {
                    Ok(x) => x,
                    Err(e) => {
                        tracing::warn!("accept failed: {e}");
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        continue;
                    }
                };
                let _ = stream.set_nodelay(true);
                conns.spawn(serve_conn(stream, peer, router.clone(), shutdown.clone()));
            }
            _ = shutdown.changed() => break,
            Some(_) = conns.join_next(), if !conns.is_empty() => {}
        }
    }
    drop(listener);
    let _ = tokio::time::timeout(GRACE, async { while conns.join_next().await.is_some() {} }).await;
    conns.abort_all();
}

async fn serve_conn<S>(
    stream: S,
    peer: SocketAddr,
    router: axum::Router,
    mut shutdown: watch::Receiver<bool>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let svc = router.map_request(move |mut r: hyper::Request<Incoming>| {
        r.extensions_mut().insert(ConnectInfo(peer));
        r
    });
    let builder = auto::Builder::new(TokioExecutor::new());
    let conn =
        builder.serve_connection_with_upgrades(TokioIo::new(stream), TowerToHyperService::new(svc));
    tokio::pin!(conn);
    tokio::select! {
        r = conn.as_mut() => {
            if let Err(e) = r {
                tracing::debug!("connection from {peer} ended: {e}");
            }
        }
        _ = shutdown.changed() => {
            conn.as_mut().graceful_shutdown();
            let _ = tokio::time::timeout(GRACE, conn).await;
        }
    }
}
