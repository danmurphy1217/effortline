//! Storage tests use only synthetic FIT bytes and disposable secrets.
mod support;

use effortline_core::fit_import::import_fit_activity;
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
    assert_eq!(
        library.import_fit_bytes(&bytes).unwrap().status,
        ImportStatus::AlreadyPresent
    );
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
    database.execute_batch("CREATE TRIGGER fail_sample BEFORE INSERT ON samples BEGIN SELECT RAISE(ABORT, 'synthetic write failure'); END;").unwrap();
    drop(database);
    let bytes = support::synthetic_fit(4, true, 2);
    let mut library = ActivityLibrary::open(directory.path(), &TestSecret(1)).unwrap();
    assert!(matches!(
        library.import_fit_bytes(&bytes),
        Err(LibraryError::Database(_))
    ));
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
