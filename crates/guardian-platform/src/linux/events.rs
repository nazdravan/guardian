//! Process-start events from the kernel's proc connector (netlink).
//!
//! Needs `CAP_NET_ADMIN` (root). The socket is read in a dedicated thread and
//! events go to the agent through a channel.

use netlink_sys::{Socket, SocketAddr, protocols::NETLINK_CONNECTOR};
use nix::errno::Errno;
use tokio::sync::mpsc;

use crate::{PlatformError, ProcessEventSource, ProcessStart};

const NLMSG_HDRLEN: usize = 16;
const CN_MSG_LEN: usize = 20;
const NLMSG_DONE: u16 = 3;
const CN_IDX_PROC: u32 = 1;
const CN_VAL_PROC: u32 = 1;
const PROC_CN_MCAST_LISTEN: u32 = 1;
const PROC_EVENT_EXEC: u32 = 0x0000_0002;

/// The subscription request: `nlmsghdr` + `cn_msg` + `PROC_CN_MCAST_LISTEN`.
pub fn subscribe_msg(seq: u32) -> Vec<u8> {
    let total = NLMSG_HDRLEN + CN_MSG_LEN + 4;
    let mut m = Vec::with_capacity(total);
    // nlmsghdr
    m.extend_from_slice(&u32::try_from(total).unwrap_or(40).to_ne_bytes());
    m.extend_from_slice(&NLMSG_DONE.to_ne_bytes());
    m.extend_from_slice(&0u16.to_ne_bytes()); // flags
    m.extend_from_slice(&seq.to_ne_bytes());
    m.extend_from_slice(&0u32.to_ne_bytes()); // port id: kernel assigns
    // cn_msg
    m.extend_from_slice(&CN_IDX_PROC.to_ne_bytes());
    m.extend_from_slice(&CN_VAL_PROC.to_ne_bytes());
    m.extend_from_slice(&seq.to_ne_bytes());
    m.extend_from_slice(&0u32.to_ne_bytes()); // ack
    m.extend_from_slice(&4u16.to_ne_bytes()); // payload length
    m.extend_from_slice(&0u16.to_ne_bytes()); // flags
    // payload
    m.extend_from_slice(&PROC_CN_MCAST_LISTEN.to_ne_bytes());
    m
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    let s: [u8; 4] = b.get(at..at + 4)?.try_into().ok()?;
    Some(u32::from_ne_bytes(s))
}

/// The thread-group id of an EXEC event in one netlink message, `None` otherwise.
///
/// Layout after the 16-byte `nlmsghdr` and 20-byte `cn_msg`: `what: u32`,
/// `cpu: u32`, `timestamp: u64`, then for EXEC `pid: u32`, `tgid: u32`.
pub fn parse_proc_event(msg: &[u8]) -> Option<u32> {
    let ev = NLMSG_HDRLEN + CN_MSG_LEN;
    if u32_at(msg, ev)? != PROC_EVENT_EXEC {
        return None;
    }
    u32_at(msg, ev + 16 + 4)
}

/// Every EXEC tgid in a datagram that may hold several netlink messages.
pub fn parse_datagram(buf: &[u8]) -> Vec<u32> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while let Some(len) = u32_at(buf, at) {
        let len = usize::try_from(len).unwrap_or(0);
        if len < NLMSG_HDRLEN || at + len > buf.len() {
            break;
        }
        if let Some(tgid) = parse_proc_event(&buf[at..at + len]) {
            out.push(tgid);
        }
        at += (len + 3) & !3;
    }
    out
}

/// The running source: a reader thread feeding a channel.
pub struct ProcConnector {
    rx: mpsc::UnboundedReceiver<Result<ProcessStart, PlatformError>>,
}

fn open() -> Result<Socket, PlatformError> {
    let map = |e: std::io::Error| match Errno::from_raw(e.raw_os_error().unwrap_or(0)) {
        Errno::EPERM | Errno::EACCES => PlatformError::AccessDenied,
        _ => PlatformError::Io(e),
    };
    let mut s = Socket::new(NETLINK_CONNECTOR).map_err(map)?;
    s.bind(&SocketAddr::new(std::process::id(), CN_IDX_PROC))
        .map_err(map)?;
    let _ = s.set_rx_buf_sz(4 * 1024 * 1024);
    s.send(&subscribe_msg(1), 0).map_err(map)?;
    Ok(s)
}

impl ProcConnector {
    /// Subscribes to exec events; `AccessDenied` without `CAP_NET_ADMIN`.
    pub fn start() -> Result<ProcConnector, PlatformError> {
        let sock = open()?;
        let (tx, rx) = mpsc::unbounded_channel();
        std::thread::Builder::new()
            .name("guardian-proc-connector".into())
            .spawn(move || {
                let mut buf: Vec<u8> = Vec::with_capacity(64 * 1024);
                loop {
                    buf.clear();
                    match sock.recv(&mut buf, 0) {
                        Ok(_) => {
                            for pid in parse_datagram(&buf) {
                                if tx.send(Ok(ProcessStart { pid })).is_err() {
                                    return;
                                }
                            }
                        }
                        Err(e) if e.raw_os_error() == Some(Errno::ENOBUFS as i32) => {
                            // The kernel dropped events: ask for a full scan, keep going.
                            if tx.send(Err(PlatformError::Changed)).is_err() {
                                return;
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                        Err(e) => {
                            let _ = tx.send(Err(PlatformError::Io(e)));
                            return;
                        }
                    }
                }
            })
            .map_err(PlatformError::Io)?;
        Ok(ProcConnector { rx })
    }
}

#[async_trait::async_trait]
impl ProcessEventSource for ProcConnector {
    async fn next(&mut self) -> Result<ProcessStart, PlatformError> {
        self.rx
            .recv()
            .await
            .unwrap_or_else(|| Err(PlatformError::Os("proc connector reader stopped".into())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(what: u32, tgid: u32) -> Vec<u8> {
        let mut m = vec![0u8; NLMSG_HDRLEN + CN_MSG_LEN];
        m.extend_from_slice(&what.to_ne_bytes());
        m.extend_from_slice(&0u32.to_ne_bytes()); // cpu
        m.extend_from_slice(&0u64.to_ne_bytes()); // timestamp
        m.extend_from_slice(&(tgid + 1).to_ne_bytes()); // pid (a thread)
        m.extend_from_slice(&tgid.to_ne_bytes());
        let len = u32::try_from(m.len()).unwrap();
        m[0..4].copy_from_slice(&len.to_ne_bytes());
        m
    }

    #[test]
    fn parses_exec_event() {
        assert_eq!(parse_proc_event(&event(PROC_EVENT_EXEC, 4242)), Some(4242));
        assert_eq!(parse_proc_event(&event(1, 4242)), None, "FORK is ignored");
        assert_eq!(parse_proc_event(&[0u8; 10]), None);
    }

    #[test]
    fn parses_several_messages_in_one_datagram() {
        let mut d = event(PROC_EVENT_EXEC, 1);
        d.extend(event(1, 2));
        d.extend(event(PROC_EVENT_EXEC, 3));
        assert_eq!(parse_datagram(&d), vec![1, 3]);
        assert_eq!(
            parse_datagram(&d[..d.len() - 3]),
            vec![1],
            "truncated tail is dropped"
        );
    }

    #[test]
    fn subscribe_message_layout() {
        let m = subscribe_msg(9);
        assert_eq!(m.len(), 40);
        assert_eq!(u32_at(&m, 0), Some(40));
        assert_eq!((u32_at(&m, 16), u32_at(&m, 20)), (Some(1), Some(1)));
        assert_eq!(u32_at(&m, 36), Some(PROC_CN_MCAST_LISTEN));
    }
}
