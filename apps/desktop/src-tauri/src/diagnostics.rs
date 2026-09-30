//! Local diagnostics accept only reviewed, typed fields. Never pass raw errors or activity data.
use crate::fit_preview::{PreviewError, PreviewResponse};
use crate::library_save::{LibraryCheckResponse, SaveProgress, SaveResponse};
use serde::Serialize;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const MAX_FILE_BYTES: u64 = 1024 * 1024;
const QUEUE_CAPACITY: usize = 256;

#[derive(Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub(super) enum Event {
    AppStarted {
        app_version: &'static str,
        os: &'static str,
        arch: &'static str,
    },
    PreviewStarted,
    PreviewFinished {
        elapsed_ms: f64,
        outcome: PreviewOutcome,
    },
    SaveStarted,
    LibraryCheckStarted,
    LibraryCheckProgress {
        progress: SaveProgress,
    },
    LibraryCheckFinished {
        elapsed_ms: f64,
        result: LibraryCheckResponse,
    },
    LibraryReused,
    SaveProgress {
        progress: SaveProgress,
    },
    SaveFinished {
        elapsed_ms: f64,
        result: SaveResponse,
    },
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum PreviewOutcome {
    Ready { sample_count: usize },
    Cancelled,
    Error { code: PreviewError },
}

impl From<&PreviewResponse> for PreviewOutcome {
    fn from(response: &PreviewResponse) -> Self {
        match response {
            PreviewResponse::Ready { preview, .. } => Self::Ready {
                sample_count: preview.sample_count,
            },
            PreviewResponse::Cancelled { .. } => Self::Cancelled,
            PreviewResponse::Error { code, .. } => Self::Error { code: *code },
        }
    }
}

#[derive(Serialize)]
struct Record {
    version: u8,
    unix_ms: u128,
    session: String,
    operation: u64,
    dropped_events: u64,
    #[serde(flatten)]
    event: Event,
}

pub(super) struct Diagnostics {
    sender: SyncSender<Record>,
    session: String,
    next_operation: AtomicU64,
    dropped: AtomicU64,
}

impl Diagnostics {
    pub fn new(directory: Option<PathBuf>) -> Self {
        let (sender, receiver) = sync_channel::<Record>(QUEUE_CAPACITY);
        // Disk work never runs on the UI or save worker. A slow/full disk drops diagnostics
        // once the bounded queue fills; it must not hold up an activity commit.
        let _ = std::thread::Builder::new().name("local-diagnostics".into()).spawn(move || {
            let Some(directory) = directory else { return; };
            let mut writer = LogWriter { directory, limit: MAX_FILE_BYTES };
            for record in receiver {
                if writer.append(&record).is_err() {
                    eprintln!("Effortline local diagnostics unavailable; activity storage is unaffected.");
                    break;
                }
            }
        });
        Self {
            sender,
            session: format!("{}-{}", unix_ms(), std::process::id()),
            next_operation: AtomicU64::new(1),
            dropped: AtomicU64::new(0),
        }
    }

    pub fn begin(&self, event: Event) -> (u64, Instant) {
        let operation = self.next_operation.fetch_add(1, Ordering::Relaxed);
        let started = Instant::now();
        self.record(operation, event);
        (operation, started)
    }

    pub fn record(&self, operation: u64, event: Event) {
        let dropped = self.dropped.swap(0, Ordering::Relaxed);
        let record = Record {
            version: 1,
            unix_ms: unix_ms(),
            session: self.session.clone(),
            operation,
            dropped_events: dropped,
            event,
        };
        if self.sender.try_send(record).is_err() {
            self.dropped
                .fetch_add(dropped.saturating_add(1), Ordering::Relaxed);
        }
    }
}

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

struct LogWriter {
    directory: PathBuf,
    limit: u64,
}

fn reject_symlink(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(io::Error::other("diagnostic symlink"))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

impl LogWriter {
    fn append(&mut self, record: &Record) -> io::Result<()> {
        reject_symlink(&self.directory)?;
        fs::create_dir_all(&self.directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.directory, fs::Permissions::from_mode(0o700))?;
        }
        // Serialize rotation across app processes. Only the log worker can wait here;
        // producers keep using try_send on their bounded queue.
        let lock_path = self.directory.join("diagnostics.lock");
        reject_symlink(&lock_path)?;
        let mut lock_options = OpenOptions::new();
        lock_options.create(true).truncate(false).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            lock_options.mode(0o600);
        }
        let lock = lock_options.open(lock_path)?;
        lock.lock()?;
        let path = self.directory.join("diagnostics.jsonl");
        let previous = self.directory.join("diagnostics.previous.jsonl");
        reject_symlink(&path)?;
        reject_symlink(&previous)?;
        let mut line = serde_json::to_vec(record)?;
        line.push(b'\n');
        if line.len() as u64 > self.limit {
            return Err(io::Error::other("diagnostic record too large"));
        }
        let size = match fs::metadata(&path) {
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
            Err(error) => return Err(error),
        };
        if size + line.len() as u64 > self.limit {
            match fs::remove_file(&previous) {
                Ok(()) => (),
                Err(error) if error.kind() == io::ErrorKind::NotFound => (),
                Err(error) => return Err(error),
            }
            fs::rename(&path, &previous)?;
        }
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(&line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> Record {
        Record {
            version: 1,
            unix_ms: 0,
            session: "synthetic".into(),
            operation: 1,
            dropped_events: 0,
            event: Event::SaveFinished {
                elapsed_ms: 12.0,
                result: SaveResponse::Saved { version: 1 },
            },
        }
    }

    #[test]
    fn rotates_bounded_logs_and_keeps_records_parseable() {
        let directory = tempfile::tempdir().unwrap();
        let mut writer = LogWriter {
            directory: directory.path().join("logs"),
            limit: 1024,
        };
        for _ in 0..100 {
            writer.append(&record()).unwrap();
        }
        let entries: Vec<_> = fs::read_dir(&writer.directory)
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|ext| ext == "jsonl")
            })
            .collect();
        assert_eq!(entries.len(), 2);
        for entry in entries {
            let entry = entry.unwrap();
            assert!(entry.metadata().unwrap().len() <= 1024);
            for line in fs::read_to_string(entry.path()).unwrap().lines() {
                let value: serde_json::Value = serde_json::from_str(line).unwrap();
                assert_eq!(value["event"], "save_finished");
                assert_eq!(value["result"]["status"], "saved");
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    entry.metadata().unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
        }
    }

    #[test]
    fn concurrent_writers_keep_rotation_bounded() {
        let directory = tempfile::tempdir().unwrap();
        std::thread::scope(|scope| {
            for _ in 0..4 {
                let directory = directory.path().to_path_buf();
                scope.spawn(move || {
                    let mut writer = LogWriter {
                        directory,
                        limit: 1024,
                    };
                    for _ in 0..100 {
                        writer.append(&record()).unwrap();
                    }
                });
            }
        });
        for name in ["diagnostics.jsonl", "diagnostics.previous.jsonl"] {
            let path = directory.path().join(name);
            assert!(fs::metadata(&path).unwrap().len() <= 1024);
            for line in fs::read_to_string(path).unwrap().lines() {
                serde_json::from_str::<serde_json::Value>(line).unwrap();
            }
        }
    }

    #[test]
    fn full_or_closed_queue_drops_diagnostics_without_blocking() {
        let (sender, receiver) = sync_channel(1);
        let diagnostics = Diagnostics {
            sender,
            session: "synthetic".into(),
            next_operation: AtomicU64::new(1),
            dropped: AtomicU64::new(0),
        };
        diagnostics.record(1, Event::SaveStarted);
        diagnostics.record(1, Event::LibraryReused);
        assert_eq!(diagnostics.dropped.load(Ordering::Relaxed), 1);
        receiver.recv().unwrap();
        diagnostics.record(1, Event::LibraryReused);
        assert_eq!(receiver.recv().unwrap().dropped_events, 1);
        drop(receiver);
        diagnostics.record(1, Event::SaveStarted);
        assert_eq!(diagnostics.dropped.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn disk_failure_does_not_panic_or_overwrite_an_unrelated_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("occupied");
        fs::write(&path, b"synthetic sentinel").unwrap();
        let mut writer = LogWriter {
            directory: path.clone(),
            limit: 1024,
        };
        assert!(writer.append(&record()).is_err());
        assert_eq!(fs::read(path).unwrap(), b"synthetic sentinel");
    }
}
