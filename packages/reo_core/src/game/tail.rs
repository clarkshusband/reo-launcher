use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncSeekExt, BufReader};

use crate::game::diagnosis::CrashWatch;
use crate::game::session::SessionRecorder;
use reo_events::EventBus;

const POLL_INTERVAL: Duration = Duration::from_millis(200);

pub(crate) struct LogTail {
    stop: Arc<AtomicBool>,
    handle: tokio::task::JoinHandle<()>,
}

impl LogTail {
    /// Stops only after one last pass the lines explaining why the game
    /// exited are written moments before it does
    pub(crate) async fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.handle.await;
    }
}

/// Lines starting before `record_from` still reach the console but not the
/// recorder a re-adopted game's log was already booked up to there
pub(crate) fn spawn_log_tail(
    cluster_id: i64,
    path: PathBuf,
    events: EventBus,
    recorder: Option<SessionRecorder>,
    record_from: u64,
    crash_watch: CrashWatch,
) -> LogTail {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);

    let handle = tokio::spawn(async move {
        if let Err(err) = tail(
            cluster_id,
            &path,
            &events,
            recorder.as_ref(),
            record_from,
            &crash_watch,
            &flag,
        )
        .await
        {
            tracing::warn!(cluster_id, path = %path.display(), error = %err, "game log tail stopped");
        }
    });

    LogTail { stop, handle }
}

async fn tail(
    cluster_id: i64,
    path: &PathBuf,
    events: &EventBus,
    recorder: Option<&SessionRecorder>,
    mut record_from: u64,
    crash_watch: &CrashWatch,
    stop: &AtomicBool,
) -> std::io::Result<()> {
    // The game may not have created the file yet on a re-adopt so wait for it
    // rather than giving up
    let file = loop {
        match tokio::fs::File::open(path).await {
            Ok(file) => break file,
            Err(_) if !stop.load(Ordering::Relaxed) => {
                tokio::time::sleep(POLL_INTERVAL).await;
            }
            Err(err) => return Err(err),
        }
    };

    let mut reader = BufReader::new(file);
    // Bytes rather than a `String` a JVM writing in the native codepage would
    // otherwise end the tail at its first non-UTF-8 byte
    let mut line = Vec::new();
    let mut offset = 0u64;

    loop {
        line.clear();
        let read = reader.read_until(b'\n', &mut line).await?;

        if read == 0 {
            // The read above returned EOF on a drained file so the final pass
            // has already run by the time the flag is seen here
            if stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            // A relaunch truncates the log if the file shrank our offset is
            // past the end and we would never read again
            let len = polyio::stat(path).await.map(|m| m.len()).unwrap_or(offset);
            if len < offset {
                reader.seek(std::io::SeekFrom::Start(0)).await?;
                offset = 0;
                // Nothing in a fresh log has been recorded yet
                record_from = 0;
            }
            tokio::time::sleep(POLL_INTERVAL).await;
            continue;
        }

        let start = offset;
        offset += read as u64;
        let line_recorder = recorder.filter(|_| start >= record_from);

        // A partial line means the game is mid-write wait for the newline
        // rather than emitting half a line and re-emitting the rest later
        if line.last() != Some(&b'\n') {
            // Unless the game is gone a process killed mid-write leaves a
            // newline that never arrives hanging the caller of `stop()`
            if stop.load(Ordering::Relaxed) {
                let text = String::from_utf8_lossy(&line);
                emit(cluster_id, &text, events, line_recorder, crash_watch).await;
                return Ok(());
            }
            reader.seek(std::io::SeekFrom::Start(start)).await?;
            offset = start;
            tokio::time::sleep(POLL_INTERVAL).await;
            continue;
        }

        let text = String::from_utf8_lossy(&line);
        emit(cluster_id, &text, events, line_recorder, crash_watch).await;
    }
}

async fn emit(
    cluster_id: i64,
    line: &str,
    events: &EventBus,
    recorder: Option<&SessionRecorder>,
    crash_watch: &CrashWatch,
) {
    // Blank lines are kept so the console reads the way the log does
    let text = reo_cluster::logs::censor(line.trim_end_matches(['\n', '\r'])).into_owned();
    if let Some(recorder) = recorder {
        recorder.observe(&text).await;
    }
    // Read on the way past rather than re-opening the log afterwards a crash
    // can take the log with it
    crash_watch.observe(&text);
    events.game_log(cluster_id, text);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    use tokio::sync::mpsc;

    struct Harness {
        dir: async_tempfile::TempDir,
        rx: reo_events::EventReceiver,
        events: EventBus,
    }

    impl Harness {
        async fn new() -> Self {
            let (tx, rx) = mpsc::unbounded_channel();
            let dir = polyio::tempdir().await.expect("temp dir");
            Self {
                dir,
                rx,
                events: EventBus::new(tx),
            }
        }

        fn path(&self) -> PathBuf {
            self.dir.dir_path().join("cluster-output.log")
        }

        fn drain(&mut self) -> Vec<String> {
            let mut out = Vec::new();
            while let Ok(reo_events::Event::Game(reo_events::GameEvent::Log {
                line,
                ..
            })) = self.rx.try_recv()
            {
                out.push(line);
            }
            out
        }
    }

    async fn settle() {
        tokio::time::sleep(POLL_INTERVAL * 4).await;
    }

    #[tokio::test]
    async fn follows_lines_appended_after_start() {
        let mut h = Harness::new().await;
        let path = h.path();
        polyio::write(&path, "[12:00:00] first\n").await.unwrap();

        let tail = spawn_log_tail(7, path.clone(), h.events.clone(), None, 0, CrashWatch::new());
        settle().await;

        let mut file = tokio::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .await
            .unwrap();
        file.write_all(b"[12:00:01] second\n").await.unwrap();
        file.flush().await.unwrap();
        settle().await;
        tail.stop().await;

        assert_eq!(h.drain(), vec!["[12:00:00] first", "[12:00:01] second"]);
    }

    #[tokio::test]
    async fn keeps_going_past_a_line_that_is_not_utf8() {
        let mut h = Harness::new().await;
        let path = h.path();
        // A JVM on Windows can write the log in the native codepage
        polyio::write(&path, b"[12:00:00] caf\xe9\n[12:00:01] after\n".as_slice())
            .await
            .unwrap();

        let tail = spawn_log_tail(7, path.clone(), h.events.clone(), None, 0, CrashWatch::new());
        settle().await;
        tail.stop().await;

        assert_eq!(
            h.drain(),
            vec!["[12:00:00] caf\u{fffd}", "[12:00:01] after"]
        );
    }

    #[tokio::test]
    async fn waits_for_a_file_that_does_not_exist_yet() {
        let mut h = Harness::new().await;
        let path = h.path();

        let tail = spawn_log_tail(7, path.clone(), h.events.clone(), None, 0, CrashWatch::new());
        settle().await;
        polyio::write(&path, "[12:00:00] late\n").await.unwrap();
        settle().await;
        tail.stop().await;

        assert_eq!(h.drain(), vec!["[12:00:00] late"]);
    }

    #[tokio::test]
    async fn holds_back_a_partial_line_until_its_newline_lands() {
        let mut h = Harness::new().await;
        let path = h.path();
        polyio::write(&path, "[12:00:00] half").await.unwrap();

        let tail = spawn_log_tail(7, path.clone(), h.events.clone(), None, 0, CrashWatch::new());
        settle().await;
        // A line mid-write must not be emitted or it would arrive twice
        assert!(h.drain().is_empty());

        let mut file = tokio::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .await
            .unwrap();
        file.write_all(b" and half\n").await.unwrap();
        file.flush().await.unwrap();
        settle().await;
        tail.stop().await;

        assert_eq!(h.drain(), vec!["[12:00:00] half and half"]);
    }

    #[tokio::test]
    async fn recovers_when_the_log_is_truncated() {
        let mut h = Harness::new().await;
        let path = h.path();
        polyio::write(&path, "[12:00:00] old session line\n")
            .await
            .unwrap();

        let tail = spawn_log_tail(7, path.clone(), h.events.clone(), None, 0, CrashWatch::new());
        settle().await;
        assert_eq!(h.drain(), vec!["[12:00:00] old session line"]);

        // A relaunch truncates the log the tail would otherwise sit forever at
        // an offset past the new end
        polyio::write(&path, "[13:00:00] new\n").await.unwrap();
        settle().await;
        tail.stop().await;

        assert_eq!(h.drain(), vec!["[13:00:00] new"]);
    }

    #[tokio::test]
    async fn stops_even_when_the_last_line_has_no_newline() {
        let mut h = Harness::new().await;
        let path = h.path();
        // A process killed mid-write leaves a newline that never arrives
        polyio::write(&path, "[12:00:00] cut off mid-writ")
            .await
            .unwrap();

        let tail = spawn_log_tail(7, path.clone(), h.events.clone(), None, 0, CrashWatch::new());
        settle().await;

        // Waiting for that newline would hang the exit path forever
        let stopped = tokio::time::timeout(Duration::from_secs(5), tail.stop()).await;
        assert!(
            stopped.is_ok(),
            "tail.stop() hung on an unterminated final line"
        );
        assert_eq!(h.drain(), vec!["[12:00:00] cut off mid-writ"]);
    }

    #[tokio::test]
    async fn emits_the_final_lines_written_just_before_exit() {
        let mut h = Harness::new().await;
        let path = h.path();
        polyio::write(&path, "[12:00:00] Stopping!\n")
            .await
            .unwrap();

        // Stopping immediately as the exit path does must still flush the
        // reason the game exited
        let tail = spawn_log_tail(7, path.clone(), h.events.clone(), None, 0, CrashWatch::new());
        tail.stop().await;

        assert_eq!(h.drain(), vec!["[12:00:00] Stopping!"]);
    }
}
