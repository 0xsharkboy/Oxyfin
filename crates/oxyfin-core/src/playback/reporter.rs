//! A task that sends playback reports to the server, in order, one at a time.

use oxyfin_api::{PlaybackReport, Session};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::task::JoinHandle;

use crate::error::CoreError;

/// Kind of playback report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReportKind {
    Start,
    Progress,
    Stopped,
}

/// Sending side of the reporter task.
pub(super) struct Reporter {
    reports: UnboundedSender<(ReportKind, PlaybackReport)>,
    task: JoinHandle<()>,
}

impl Reporter {
    /// Starts the task. It sends `()` on `expired` once, at the first 401, and
    /// drops every report after it. Other failures are ignored.
    pub(super) fn spawn(session: Session, expired: UnboundedSender<()>) -> Self {
        let (reports, receiver) = unbounded_channel();
        let task = tokio::spawn(run(session, receiver, expired));
        Self { reports, task }
    }

    /// Queues a report.
    pub(super) fn send(&self, kind: ReportKind, report: PlaybackReport) {
        let _ = self.reports.send((kind, report));
    }

    /// Stops accepting reports; the returned task ends once the queue is sent.
    pub(super) fn close(self) -> JoinHandle<()> {
        self.task
    }
}

async fn run(
    session: Session,
    mut reports: UnboundedReceiver<(ReportKind, PlaybackReport)>,
    expired: UnboundedSender<()>,
) {
    while let Some((kind, report)) = reports.recv().await {
        let result = match kind {
            ReportKind::Start => session.report_playback_start(&report).await,
            ReportKind::Progress => session.report_playback_progress(&report).await,
            ReportKind::Stopped => session.report_playback_stopped(&report).await,
        };
        if let Err(error) = result
            && CoreError::from(error) == CoreError::SessionExpired
        {
            let _ = expired.send(());
            break;
        }
    }
}
