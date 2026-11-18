//! Local IPC between the agent and the worker: length-prefixed protobuf frames
//! over a Unix socket (Linux) or a named pipe (Windows), with the peer's PID.

use std::io;
use std::path::PathBuf;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};

/// Largest frame (64 KiB).
pub const MAX_FRAME: usize = 64 * 1024;

/// Where the agent listens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcEndpoint {
    /// Unix socket path.
    Unix(PathBuf),
    /// Windows pipe name (`\\.\pipe\…`).
    Pipe(String),
}

const PIPE_PREFIX: &str = r"\\.\pipe\";

impl IpcEndpoint {
    /// `/run/guardian-agent/worker.sock` or `\\.\pipe\guardian-agent-worker`.
    pub fn system_default() -> IpcEndpoint {
        if cfg!(windows) {
            IpcEndpoint::Pipe(format!("{PIPE_PREFIX}guardian-agent-worker"))
        } else {
            IpcEndpoint::Unix("/run/guardian-agent/worker.sock".into())
        }
    }

    /// The form passed on the worker's command line.
    pub fn to_arg(&self) -> String {
        match self {
            IpcEndpoint::Unix(p) => p.to_string_lossy().into_owned(),
            IpcEndpoint::Pipe(n) => n.clone(),
        }
    }

    /// Inverse of [`IpcEndpoint::to_arg`].
    pub fn parse(s: &str) -> IpcEndpoint {
        if s.starts_with(PIPE_PREFIX) {
            IpcEndpoint::Pipe(s.to_string())
        } else {
            IpcEndpoint::Unix(s.into())
        }
    }
}

/// Writes one frame: big-endian u32 length, then the message.
pub async fn write_frame<W: AsyncWrite + Unpin, M: prost::Message>(
    w: &mut W,
    m: &M,
) -> io::Result<()> {
    let bytes = m.encode_to_vec();
    if bytes.len() > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    let len =
        u32::try_from(bytes.len()).map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
    w.write_all(&len.to_be_bytes()).await?;
    w.write_all(&bytes).await?;
    w.flush().await
}

/// Reads one frame; `InvalidData` if it is larger than [`MAX_FRAME`] or undecodable.
pub async fn read_frame<R: AsyncRead + Unpin, M: prost::Message + Default>(
    r: &mut R,
) -> io::Result<M> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len).await?;
    let len = usize::try_from(u32::from_be_bytes(len)).unwrap_or(usize::MAX);
    if len > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await?;
    M::decode(buf.as_slice()).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// One connection.
pub enum IpcStream {
    /// Unix socket.
    #[cfg(unix)]
    Unix(tokio::net::UnixStream),
    /// Server end of a pipe.
    #[cfg(windows)]
    PipeServer(tokio::net::windows::named_pipe::NamedPipeServer),
    /// Client end of a pipe.
    #[cfg(windows)]
    PipeClient(tokio::net::windows::named_pipe::NamedPipeClient),
}

impl std::fmt::Debug for IpcStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IpcStream")
    }
}

macro_rules! each {
    ($self:ident, $s:ident => $e:expr) => {
        match $self.get_mut() {
            #[cfg(unix)]
            IpcStream::Unix($s) => $e,
            #[cfg(windows)]
            IpcStream::PipeServer($s) => $e,
            #[cfg(windows)]
            IpcStream::PipeClient($s) => $e,
        }
    };
}

impl AsyncRead for IpcStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        each!(self, s => Pin::new(s).poll_read(cx, buf))
    }
}

impl AsyncWrite for IpcStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        each!(self, s => Pin::new(s).poll_write(cx, buf))
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        each!(self, s => Pin::new(s).poll_flush(cx))
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        each!(self, s => Pin::new(s).poll_shutdown(cx))
    }
}

/// The agent's listening side.
pub struct IpcListener {
    #[cfg(unix)]
    inner: tokio::net::UnixListener,
    #[cfg(windows)]
    name: String,
    #[cfg(windows)]
    next: Option<tokio::net::windows::named_pipe::NamedPipeServer>,
}

impl std::fmt::Debug for IpcListener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IpcListener")
    }
}

fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "endpoint kind not supported on this OS",
    )
}

impl IpcListener {
    /// Listens on `ep`. Unix: the directory is created `0711`, a stale socket
    /// file is removed, the socket is `0666` (the worker runs as the user).
    /// Windows: the first pipe instance is created with `first_pipe_instance`, so
    /// another process cannot have squatted the name, and every instance gets an
    /// explicit security descriptor (`windows/ffi_pipe.rs`).
    pub fn bind(ep: &IpcEndpoint) -> io::Result<IpcListener> {
        match ep {
            #[cfg(unix)]
            IpcEndpoint::Unix(path) => {
                use std::os::unix::fs::PermissionsExt;
                if let Some(dir) = path.parent()
                    && !dir.exists()
                {
                    std::fs::create_dir_all(dir)?;
                    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o711))?;
                }
                match std::fs::remove_file(path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e),
                }
                let inner = tokio::net::UnixListener::bind(path)?;
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666))?;
                Ok(IpcListener { inner })
            }
            #[cfg(windows)]
            IpcEndpoint::Pipe(name) => {
                let first = crate::windows::create_pipe_server(name, true)?;
                Ok(IpcListener {
                    name: name.clone(),
                    next: Some(first),
                })
            }
            #[allow(unreachable_patterns)]
            _ => Err(unsupported()),
        }
    }

    /// The next connection and the PID of the process on the other end.
    pub async fn accept(&mut self) -> io::Result<(IpcStream, u32)> {
        #[cfg(unix)]
        {
            let (s, _) = self.inner.accept().await?;
            let pid = s
                .peer_cred()?
                .pid()
                .and_then(|p| u32::try_from(p).ok())
                .ok_or_else(|| io::Error::other("peer pid unavailable"))?;
            Ok((IpcStream::Unix(s), pid))
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            let server = match self.next.take() {
                Some(s) => s,
                None => crate::windows::create_pipe_server(&self.name, false)?,
            };
            server.connect().await?;
            self.next = crate::windows::create_pipe_server(&self.name, false).ok();
            let pid = crate::windows::pipe_client_pid(server.as_raw_handle())?;
            Ok((IpcStream::PipeServer(server), pid))
        }
    }
}

/// Connects to the agent.
pub async fn connect(ep: &IpcEndpoint) -> io::Result<IpcStream> {
    match ep {
        #[cfg(unix)]
        IpcEndpoint::Unix(path) => Ok(IpcStream::Unix(
            tokio::net::UnixStream::connect(path).await?,
        )),
        #[cfg(windows)]
        IpcEndpoint::Pipe(name) => {
            const ERROR_PIPE_BUSY: i32 = 231;
            for _ in 0..50 {
                match tokio::net::windows::named_pipe::ClientOptions::new().open(name) {
                    Ok(c) => return Ok(IpcStream::PipeClient(c)),
                    Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                    Err(e) => return Err(e),
                }
            }
            Err(io::Error::new(io::ErrorKind::TimedOut, "pipe busy"))
        }
        #[allow(unreachable_patterns)]
        _ => Err(unsupported()),
    }
}
