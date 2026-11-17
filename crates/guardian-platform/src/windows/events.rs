//! Process-start events from ETW: the user-mode provider
//! Microsoft-Windows-Kernel-Process, event 1 (ProcessStart). Needs admin.

use ferrisetw::EventRecord;
use ferrisetw::parser::Parser;
use ferrisetw::provider::Provider;
use ferrisetw::schema_locator::SchemaLocator;
use ferrisetw::trace::{TraceTrait, UserTrace, stop_trace_by_name};
use tokio::sync::mpsc;

use crate::{PlatformError, ProcessEventSource, ProcessStart};

/// The provider's GUID.
pub const KERNEL_PROCESS_PROVIDER: &str = "22FB2CD6-0E7B-422B-A0C7-2FAD1FD0E716";
/// `WINEVENT_KEYWORD_PROCESS`.
const KEYWORD_PROCESS: u64 = 0x10;
/// ProcessStart.
const EVENT_PROCESS_START: u16 = 1;
/// Session name; a leftover session with this name is stopped first.
pub const SESSION_NAME: &str = "GuardianAgentProcessTrace";

type Tx = mpsc::UnboundedSender<Result<ProcessStart, PlatformError>>;

/// A running ETW session.
pub struct EtwEvents {
    rx: mpsc::UnboundedReceiver<Result<ProcessStart, PlatformError>>,
    // Dropping the trace stops the session.
    _trace: UserTrace,
}

fn on_event(tx: &Tx, record: &EventRecord, schemas: &SchemaLocator) {
    if record.event_id() != EVENT_PROCESS_START {
        return;
    }
    let Ok(schema) = schemas.event_schema(record) else {
        return;
    };
    let parser = Parser::create(record, &schema);
    if let Ok(pid) = parser.try_parse::<u32>("ProcessID") {
        let _ = tx.send(Ok(ProcessStart { pid }));
    }
}

impl EtwEvents {
    /// Starts the session; fails without administrator rights.
    pub fn start() -> Result<EtwEvents, PlatformError> {
        let _ = stop_trace_by_name(SESSION_NAME);
        let (tx, rx) = mpsc::unbounded_channel();
        let cb_tx = tx.clone();
        let provider = Provider::by_guid(KERNEL_PROCESS_PROVIDER)
            .any(KEYWORD_PROCESS)
            .add_callback(move |record: &EventRecord, schemas: &SchemaLocator| {
                on_event(&cb_tx, record, schemas);
            })
            .build();
        let (trace, handle) = UserTrace::new()
            .named(SESSION_NAME.to_string())
            .enable(provider)
            .start()
            .map_err(|e| PlatformError::Os(format!("ETW session failed to start: {e:?}")))?;
        std::thread::Builder::new()
            .name("guardian-etw".into())
            .spawn(move || {
                let r = UserTrace::process_from_handle(handle);
                let _ = tx.send(Err(PlatformError::Os(format!("ETW session ended: {r:?}"))));
            })
            .map_err(PlatformError::Io)?;
        Ok(EtwEvents { rx, _trace: trace })
    }
}

#[async_trait::async_trait]
impl ProcessEventSource for EtwEvents {
    async fn next(&mut self) -> Result<ProcessStart, PlatformError> {
        self.rx
            .recv()
            .await
            .unwrap_or_else(|| Err(PlatformError::Os("ETW reader stopped".into())))
    }
}
