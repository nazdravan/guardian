//! One background job at a time; the window polls it every frame and never blocks (spec R26).

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

use guardian_core::edit::plan::WritePlan;
use guardian_core::snapshot::Snapshot;
use guardian_winhost::CollectOptions;
use guardian_winhost::apply::{Applied, ApplyError};

pub enum Job {
    Collect(CollectOptions),
    Apply { plan: WritePlan, overwrite: bool },
    Restart,
    SaveReport(Box<Snapshot>),
}

pub enum Done {
    Collected(Box<Snapshot>),
    Applied(Result<Applied, ApplyError>),
    Restarted(Result<(), String>),
    ReportSaved(Result<PathBuf, String>),
}

pub struct Worker {
    rx: Receiver<Done>,
    progress: Arc<Mutex<Option<String>>>,
}

impl Worker {
    pub fn spawn(job: Job, ctx: egui::Context) -> Self {
        let (tx, rx) = channel();
        let progress = Arc::new(Mutex::new(Some(match &job {
            Job::Collect(_) => "Reading service, files, log…".to_string(),
            Job::Apply { .. } => "Saving…".into(),
            Job::Restart => "Restarting ProcSentinelAgent…".into(),
            Job::SaveReport(_) => "Saving report…".into(),
        })));
        let p = Arc::clone(&progress);
        std::thread::spawn(move || {
            let set = |m: &str| {
                if let Ok(mut g) = p.lock() {
                    *g = Some(m.to_string());
                }
                ctx.request_repaint();
            };
            let done = match job {
                Job::Collect(o) => Done::Collected(Box::new(guardian_winhost::collect(&o))),
                Job::Apply { plan, overwrite } => {
                    Done::Applied(guardian_winhost::apply::apply(&plan, &guardian_winhost::io_path, overwrite, chrono::Local::now()))
                }
                Job::Restart => Done::Restarted(guardian_winhost::restart_service(&set)),
                // Task 12 replaces this with guardian_winhost::save_report.
                Job::SaveReport(_) => Done::ReportSaved(Err("Saving reports arrives in a later task".into())),
            };
            let _ = tx.send(done);
            ctx.request_repaint();
        });
        Worker { rx, progress }
    }

    pub fn poll(&self) -> Option<Done> {
        self.rx.try_recv().ok()
    }

    pub fn progress(&self) -> Option<String> {
        self.progress.lock().ok().and_then(|g| g.clone())
    }
}
