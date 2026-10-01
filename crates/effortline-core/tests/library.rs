//! Storage tests use only synthetic FIT bytes and disposable secrets.
mod support;

use effortline_core::fit_import::import_fit_activity;
use effortline_core::fit_import::ActivitySummary;
use effortline_core::library::{
    ActivityLibrary, ImportStatus, LibraryError, LibrarySecret, LibrarySecretProvider,
    SecretUnavailable,
};
use std::fs;
use tempfile::tempdir;

struct TestSecret(u8);
impl LibrarySecretProvider for TestSecret {
    fn load_secret(&self) -> Result<LibrarySecret, SecretUnavailable> {
        Ok(LibrarySecret::from_bytes([self.0; 32]))
    }
}

#[test]
fn saves_reopens_and_deduplicates_encrypted_activity() {
    let directory = tempdir().unwrap();
    let bytes = support::synthetic_fit(4, true, 2);
    let expected = import_fit_activity(&bytes).unwrap();
    let mut library = ActivityLibrary::open(directory.path(), &TestSecret(1)).unwrap();
    let saved = library.import_fit_bytes(&bytes).unwrap();
    assert_eq!(saved.status, ImportStatus::Saved);
    assert_eq!(saved.summary, ActivitySummary::from(&expected.data));
    let duplicate = library.import_fit_bytes(&bytes).unwrap();
    assert_eq!(duplicate.status, ImportStatus::AlreadyPresent);
    assert_eq!(duplicate.summary, saved.summary);
    assert_eq!(
        library.find_activity(&saved.identity).unwrap(),
        Some(expected.clone())
    );
    drop(library);
    let database_bytes = fs::read(directory.path().join("library.sqlite3")).unwrap();
    assert!(!database_bytes.starts_with(b"SQLite format 3"));
    let object = fs::read_dir(directory.path().join("objects"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_ne!(fs::read(object).unwrap(), bytes);
    assert!(matches!(
        ActivityLibrary::open(directory.path(), &TestSecret(2)),
        Err(LibraryError::CannotUnlock)
    ));
    let library = ActivityLibrary::open(directory.path(), &TestSecret(1)).unwrap();
    assert_eq!(
        library.list_activity_ids().unwrap(),
        vec![saved.identity.clone()]
    );
    assert_eq!(
        library.find_activity(&saved.identity).unwrap(),
        Some(expected)
    );
    assert_eq!(
        library.read_original_bytes(&saved.identity).unwrap(),
        Some(bytes)
    );
}

#[test]
fn rejects_concurrent_open_and_invalid_import_without_saving() {
    let directory = tempdir().unwrap();
    let mut library = ActivityLibrary::open(directory.path(), &TestSecret(1)).unwrap();
    assert!(matches!(
        ActivityLibrary::open(directory.path(), &TestSecret(1)),
        Err(LibraryError::Busy)
    ));
    assert!(matches!(
        library.import_fit_bytes(b"invalid synthetic FIT"),
        Err(LibraryError::Import(_))
    ));
    assert!(library.list_activity_ids().unwrap().is_empty());
    assert_eq!(
        fs::read_dir(directory.path().join("objects"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn recovers_uncommitted_objects_but_preserves_committed_original() {
    let directory = tempdir().unwrap();
    let bytes = support::synthetic_fit(4, false, 1);
    let mut library = ActivityLibrary::open(directory.path(), &TestSecret(1)).unwrap();
    let saved = library.import_fit_bytes(&bytes).unwrap();
    drop(library);
    let objects = directory.path().join("objects");
    let orphan = objects.join("00000000000000000000000000000000.fitenc");
    let temporary = objects.join(".tmp-11111111111111111111111111111111.fitenc");
    fs::write(&orphan, b"synthetic interrupted write").unwrap();
    fs::write(&temporary, b"synthetic partial write").unwrap();
    let library = ActivityLibrary::open(directory.path(), &TestSecret(1)).unwrap();
    assert!(!orphan.exists());
    assert!(!temporary.exists());
    assert_eq!(
        library.read_original_bytes(&saved.identity).unwrap(),
        Some(bytes)
    );
    assert_eq!(
        library
            .find_activity(&saved.identity)
            .unwrap()
            .unwrap()
            .data
            .samples[0]
            .heart_rate_bpm,
        None
    );
}

#[test]
fn detects_damaged_and_missing_original_without_overwriting_it() {
    let directory = tempdir().unwrap();
    let bytes = support::synthetic_fit(4, true, 1);
    let mut library = ActivityLibrary::open(directory.path(), &TestSecret(1)).unwrap();
    let saved = library.import_fit_bytes(&bytes).unwrap();
    let object = fs::read_dir(directory.path().join("objects"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut damaged = fs::read(&object).unwrap();
    *damaged.last_mut().unwrap() ^= 1;
    fs::write(&object, &damaged).unwrap();
    assert!(matches!(
        library.read_original_bytes(&saved.identity),
        Err(LibraryError::CorruptOriginal)
    ));
    assert!(matches!(
        library.import_fit_bytes(&bytes),
        Err(LibraryError::CorruptOriginal)
    ));
    assert_eq!(fs::read(&object).unwrap(), damaged);
    fs::remove_file(object).unwrap();
    assert!(matches!(
        library.find_activity(&saved.identity),
        Err(LibraryError::Incomplete)
    ));
    drop(library);
    assert!(matches!(
        ActivityLibrary::open(directory.path(), &TestSecret(1)),
        Err(LibraryError::Incomplete)
    ));
}

fn open_test_database(directory: &std::path::Path) -> rusqlite::Connection {
    let mut key = [0; 32];
    hkdf::Hkdf::<sha2::Sha256>::new(None, &[1; 32])
        .expand(b"effortline/sqlcipher/v1", &mut key)
        .unwrap();
    let hex: String = key.iter().map(|byte| format!("{byte:02x}")).collect();
    let database = rusqlite::Connection::open(directory.join("library.sqlite3")).unwrap();
    database
        .execute_batch(&format!("PRAGMA key = \"x'{hex}'\";"))
        .unwrap();
    database
}

#[test]
fn failed_sample_write_rolls_back_activity_and_allows_retry_after_recovery() {
    let directory = tempdir().unwrap();
    drop(ActivityLibrary::open(directory.path(), &TestSecret(1)).unwrap());
    let database = open_test_database(directory.path());
    database.execute_batch("CREATE TRIGGER fail_sample BEFORE INSERT ON samples WHEN NEW.sample_index = 1024 BEGIN SELECT RAISE(ABORT, 'synthetic write failure'); END;").unwrap();
    drop(database);
    let bytes = support::synthetic_fit(4, true, 16_705);
    let mut library = ActivityLibrary::open(directory.path(), &TestSecret(1)).unwrap();
    use effortline_core::library::{LibraryProgress, LibraryStage};
    let mut events = Vec::new();
    assert!(matches!(
        library.import_fit_bytes_with_progress(&bytes, &mut |event| events.push(event)),
        Err(LibraryError::Database(_))
    ));
    assert!(events.iter().any(|event| matches!(
        event,
        LibraryProgress::SamplesWritten {
            completed: 1024,
            total: 16_705,
            ..
        }
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        LibraryProgress::Finished {
            stage: LibraryStage::SampleInserts,
            succeeded: false,
            ..
        }
    )));
    assert!(!events.iter().any(|event| matches!(
        event,
        LibraryProgress::Started(LibraryStage::DatabaseCommit)
    )));
    assert!(library.list_activity_ids().unwrap().is_empty());
    drop(library);
    let database = open_test_database(directory.path());
    database.execute_batch("DROP TRIGGER fail_sample;").unwrap();
    drop(database);
    let mut library = ActivityLibrary::open(directory.path(), &TestSecret(1)).unwrap();
    assert_eq!(
        fs::read_dir(directory.path().join("objects"))
            .unwrap()
            .count(),
        0
    );
    assert_eq!(
        library.import_fit_bytes(&bytes).unwrap().status,
        ImportStatus::Saved
    );
}

#[test]
fn rejects_future_schema_without_changing_version() {
    let directory = tempdir().unwrap();
    drop(ActivityLibrary::open(directory.path(), &TestSecret(1)).unwrap());
    let database = open_test_database(directory.path());
    database.execute_batch("PRAGMA user_version = 2;").unwrap();
    drop(database);
    assert!(matches!(
        ActivityLibrary::open(directory.path(), &TestSecret(1)),
        Err(LibraryError::UnsupportedSchema)
    ));
    let database = open_test_database(directory.path());
    assert_eq!(
        database
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
#[ignore = "synthetic stage timing benchmark; run explicitly with --nocapture"]
fn synthetic_save_stage_timings() {
    use effortline_core::library::LibraryProgress;
    let bytes = support::synthetic_fit_sized(1121, 120875);
    let parsed = import_fit_activity(&bytes).unwrap();
    assert_eq!(parsed.data.samples.len(), 1121);
    let directory = tempdir().unwrap();
    let mut report = |event| {
        if let LibraryProgress::Finished {
            stage,
            elapsed,
            succeeded,
        } = event
        {
            println!(
                "synthetic stage={stage:?} elapsed_ms={:.3} succeeded={succeeded}",
                elapsed.as_secs_f64() * 1000.0
            );
        }
    };
    let mut library =
        ActivityLibrary::open_with_progress(directory.path(), &TestSecret(1), &mut report).unwrap();
    assert_eq!(
        library
            .import_fit_bytes_with_progress(&bytes, &mut report)
            .unwrap()
            .status,
        ImportStatus::Saved
    );
}

#[test]
fn progress_tracks_real_stages_and_never_reports_commit_after_failure() {
    use effortline_core::library::{LibraryProgress, LibraryStage};
    let directory = tempdir().unwrap();
    let mut events = Vec::new();
    let mut library =
        ActivityLibrary::open_with_progress(directory.path(), &TestSecret(1), &mut |event| {
            events.push(event)
        })
        .unwrap();
    let bytes = support::synthetic_fit(4, true, 1121);
    library
        .import_fit_bytes_with_progress(&bytes, &mut |event| events.push(event))
        .unwrap();
    let stages: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            LibraryProgress::Started(stage) => Some(*stage),
            _ => None,
        })
        .collect();
    assert_eq!(
        stages,
        [
            LibraryStage::SecretAccess,
            LibraryStage::OpenRecovery,
            LibraryStage::FitParsing,
            LibraryStage::DuplicateCheck,
            LibraryStage::Encryption,
            LibraryStage::FileWriteSync,
            LibraryStage::SampleInserts,
            LibraryStage::DatabaseCommit
        ]
    );
    let boundaries: Vec<_> = events
        .iter()
        .copied()
        .filter(|event| !matches!(event, LibraryProgress::SamplesWritten { .. }))
        .collect();
    for pair in boundaries.as_chunks::<2>().0 {
        assert!(
            matches!(pair, [LibraryProgress::Started(a), LibraryProgress::Finished { stage: b, succeeded: true, .. }] if a == b)
        );
    }
    events.clear();
    assert!(library
        .import_fit_bytes_with_progress(b"synthetic invalid FIT", &mut |event| events.push(event))
        .is_err());
    assert!(matches!(
        events.as_slice(),
        [
            LibraryProgress::Started(LibraryStage::FitParsing),
            LibraryProgress::Finished {
                stage: LibraryStage::FitParsing,
                succeeded: false,
                ..
            }
        ]
    ));
}

#[test]
fn synthetic_large_save_stage_timings() {
    use effortline_core::library::LibraryProgress;
    let directory = tempdir().unwrap();
    let mut initial = ActivityLibrary::open(directory.path(), &TestSecret(1)).unwrap();
    for count in 1..=64 {
        initial
            .import_fit_bytes(&support::synthetic_fit(4, true, count))
            .unwrap();
    }
    drop(initial);
    let mut report = |event| {
        if let LibraryProgress::Finished {
            stage,
            elapsed,
            succeeded,
        } = event
        {
            println!(
                "synthetic stage={stage:?} elapsed_ms={:.3} succeeded={succeeded}",
                elapsed.as_secs_f64() * 1000.0
            );
        }
    };
    println!("synthetic mode=cold_open prepopulated_activities=64");
    let mut library =
        ActivityLibrary::open_with_progress(directory.path(), &TestSecret(1), &mut report).unwrap();
    for count in [16_705, 16_706] {
        let bytes = support::synthetic_fit(4, true, count);
        println!(
            "synthetic samples={count} bytes={} reuse={}",
            bytes.len(),
            count == 16_706
        );
        let mut written = Vec::new();
        let saved = library
            .import_fit_bytes_with_progress(&bytes, &mut |event| {
                report(event);
                if let LibraryProgress::SamplesWritten {
                    completed, total, ..
                } = event
                {
                    assert_eq!(total, count);
                    written.push(completed);
                }
            })
            .unwrap();
        assert_eq!(saved.status, ImportStatus::Saved);
        assert_eq!(written.last(), Some(&count));
        assert!(written.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(
            library
                .find_activity(&saved.identity)
                .unwrap()
                .unwrap()
                .data
                .samples
                .len(),
            count
        );
    }
    assert_eq!(library.list_activity_ids().unwrap().len(), 66);
    drop(library);
    let library = ActivityLibrary::open(directory.path(), &TestSecret(1)).unwrap();
    assert_eq!(library.list_activity_ids().unwrap().len(), 66);
}
