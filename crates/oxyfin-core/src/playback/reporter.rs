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

/// What the reporter tells the controller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Reported {
    /// The server rejected the session.
    Expired,
    /// A stop report was sent, whatever the server answered.
    Stop,
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
    /// Starts the task. It tells each stop report once the server answered it, and the first 401
    /// once, dropping every request after it. Other failures are ignored.
    pub(super) fn spawn(session: Session, reported: UnboundedSender<Reported>) -> Self {
        let (jobs, receiver) = unbounded_channel();
        let task = tokio::spawn(run(session, receiver, reported));
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

async fn run(
    session: Session,
    mut jobs: UnboundedReceiver<Job>,
    reported: UnboundedSender<Reported>,
) {
    while let Some(job) = jobs.recv().await {
        let stop = matches!(job, Job::Report(ReportKind::Stopped, _));
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
            let _ = reported.send(Reported::Expired);
            break;
        }
        if stop {
            let _ = reported.send(Reported::Stop);
        }
    }
}
