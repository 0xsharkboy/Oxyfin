//! A task that sends playback reports to the server, in order, one at a time.

use oxyfin_api::{PlaySessionId, PlaybackReport, Session};
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

/// A request the reporter task sends.
enum Job {
    Report(ReportKind, PlaybackReport),
    StopTranscoding(PlaySessionId),
}

/// Sending side of the reporter task.
pub(super) struct Reporter {
    jobs: UnboundedSender<Job>,
    task: JoinHandle<()>,
}

impl Reporter {
    /// Starts the task. It sends `()` on `expired` once, at the first 401, and
    /// drops every request after it. Other failures are ignored.
    pub(super) fn spawn(session: Session, expired: UnboundedSender<()>) -> Self {
        let (jobs, receiver) = unbounded_channel();
        let task = tokio::spawn(run(session, receiver, expired));
        Self { jobs, task }
    }

    /// Queues a report.
    pub(super) fn send(&self, kind: ReportKind, report: PlaybackReport) {
        let _ = self.jobs.send(Job::Report(kind, report));
    }

    /// Queues the end of the server's transcode for a playback session.
    pub(super) fn stop_transcoding(&self, play_session_id: PlaySessionId) {
        let _ = self.jobs.send(Job::StopTranscoding(play_session_id));
    }

    /// Stops accepting requests; the returned task ends once the queue is sent.
    pub(super) fn close(self) -> JoinHandle<()> {
        self.task
    }
}

async fn run(session: Session, mut jobs: UnboundedReceiver<Job>, expired: UnboundedSender<()>) {
    while let Some(job) = jobs.recv().await {
        let result = match job {
            Job::Report(ReportKind::Start, report) => session.report_playback_start(&report).await,
            Job::Report(ReportKind::Progress, report) => {
                session.report_playback_progress(&report).await
            }
            Job::Report(ReportKind::Stopped, report) => {
                session.report_playback_stopped(&report).await
            }
            Job::StopTranscoding(id) => session.stop_transcoding(&id).await,
        };
        if let Err(error) = result
            && CoreError::from(error) == CoreError::SessionExpired
        {
            let _ = expired.send(());
            break;
        }
    }
}
