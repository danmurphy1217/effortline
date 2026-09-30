//! Encrypted, local activity storage. The caller owns the library directory and secret source.

use crate::fit_import::{
    import_fit_activity, ActivityData, ActivitySample, ActivitySource, FitProvenance, ImportError,
    ImportedActivity, SourceIdentity, Sport, MAX_FIT_BYTES,
};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

const SCHEMA_VERSION: i64 = 1;
const OBJECT_MAGIC: &[u8; 6] = b"ELFIT1";
const NONCE_BYTES: usize = 24;
const OBJECT_OVERHEAD: usize = OBJECT_MAGIC.len() + NONCE_BYTES + 16;

/// Real storage boundaries. Observers must return promptly and must not access the library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryStage {
    SecretAccess,
    OpenRecovery,
    FitParsing,
    DuplicateCheck,
    Encryption,
    FileWriteSync,
    SampleInserts,
    DatabaseCommit,
}

/// Durations describe completed work, including failed stages. They contain no athlete data.
#[derive(Debug, Clone, Copy)]
pub enum LibraryProgress {
    Started(LibraryStage),
    Finished {
        stage: LibraryStage,
        elapsed: Duration,
        succeeded: bool,
    },
}

fn measure<T>(
    stage: LibraryStage,
    report: &mut impl FnMut(LibraryProgress),
    work: impl FnOnce() -> Result<T, LibraryError>,
) -> Result<T, LibraryError> {
    report(LibraryProgress::Started(stage));
    let start = Instant::now();
    let result = work();
    report(LibraryProgress::Finished {
        stage,
        elapsed: start.elapsed(),
        succeeded: result.is_ok(),
    });
    result
}

/// A 256-bit random secret supplied by the host. The core never selects a secret store.
pub struct LibrarySecret(Zeroizing<[u8; 32]>);

impl LibrarySecret {
    /// The host must fill `bytes` from a cryptographically secure random source.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(Zeroizing::new(bytes))
    }
}

#[derive(Debug)]
pub struct SecretUnavailable;

pub trait LibrarySecretProvider {
    fn load_secret(&self) -> Result<LibrarySecret, SecretUnavailable>;
}

#[derive(Debug)]
pub enum LibraryError {
    Import(ImportError),
    SecretUnavailable,
    Busy,
    CannotUnlock,
    EncryptionUnavailable,
    UnsupportedSchema,
    Incomplete,
    CorruptOriginal,
    Randomness,
    Io(std::io::Error),
    Database(rusqlite::Error),
}

impl LibraryError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Import(error) => error.code(),
            Self::SecretUnavailable => "library_secret_unavailable",
            Self::Busy => "library_busy",
            Self::CannotUnlock => "library_cannot_unlock",
            Self::EncryptionUnavailable => "library_encryption_unavailable",
            Self::UnsupportedSchema => "library_unsupported_schema",
            Self::Incomplete => "library_incomplete",
            Self::CorruptOriginal => "library_corrupt_original",
            Self::Randomness => "library_randomness_unavailable",
            Self::Io(_) => "library_io",
            Self::Database(_) => "library_database",
        }
    }
}

impl From<std::io::Error> for LibraryError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<rusqlite::Error> for LibraryError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

impl From<ImportError> for LibraryError {
    fn from(error: ImportError) -> Self {
        Self::Import(error)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportStatus {
    Saved,
    AlreadyPresent,
}

pub struct ImportResult {
    pub identity: SourceIdentity,
    pub status: ImportStatus,
}

/// One open library. The lock prevents a second process from racing orphan recovery.
pub struct ActivityLibrary {
    database: Connection,
    objects: PathBuf,
    object_key: Zeroizing<[u8; 32]>,
    _lock: File,
}

impl ActivityLibrary {
    /// Open or create a version-1 library. A pre-existing partial library is an error.
    pub fn open(
        directory: impl AsRef<Path>,
        provider: &impl LibrarySecretProvider,
    ) -> Result<Self, LibraryError> {
        Self::open_with_progress(directory, provider, &mut |_| {})
    }

    pub fn open_with_progress(
        directory: impl AsRef<Path>,
        provider: &impl LibrarySecretProvider,
        report: &mut impl FnMut(LibraryProgress),
    ) -> Result<Self, LibraryError> {
        let secret = measure(LibraryStage::SecretAccess, report, || {
            provider
                .load_secret()
                .map_err(|_| LibraryError::SecretUnavailable)
        })?;
        measure(LibraryStage::OpenRecovery, report, || {
            let database_key = derive_key(&secret.0, b"effortline/sqlcipher/v1")?;
            let object_key = derive_key(&secret.0, b"effortline/objects/v1")?;
            let directory = directory.as_ref();
            fs::create_dir_all(directory)?;
            let lock = OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(directory.join("library.lock"))?;
            lock.try_lock().map_err(|error| match error {
                std::fs::TryLockError::WouldBlock => LibraryError::Busy,
                std::fs::TryLockError::Error(error) => LibraryError::Io(error),
            })?;

            let objects = directory.join("objects");
            let database_path = directory.join("library.sqlite3");
            let existing = database_path.exists();
            if !existing && objects.exists() && fs::read_dir(&objects)?.next().is_some() {
                return Err(LibraryError::Incomplete);
            }
            if existing && !objects.is_dir() {
                return Err(LibraryError::Incomplete);
            }
            fs::create_dir_all(&objects)?;

            let mut database = Connection::open(database_path)?;
            set_database_key(&database, &database_key)?;
            let _: String = database
                .query_row("PRAGMA cipher_version", [], |row| row.get(0))
                .map_err(|_| LibraryError::EncryptionUnavailable)?;
            let version: i64 = database
                .query_row("PRAGMA user_version", [], |row| row.get(0))
                .map_err(|_| LibraryError::CannotUnlock)?;
            if existing {
                if version != SCHEMA_VERSION {
                    return Err(LibraryError::UnsupportedSchema);
                }
            } else {
                create_schema(&mut database)?;
            }
            database.execute_batch(
            "PRAGMA foreign_keys = ON; PRAGMA journal_mode = DELETE; PRAGMA synchronous = FULL;",
        )?;
            let library = Self {
                database,
                objects,
                object_key,
                _lock: lock,
            };
            library.recover_orphans()?;
            Ok(library)
        })
    }

    /// Import FIT bytes through the canonical importer. The same bytes return `AlreadyPresent`.
    pub fn import_fit_bytes(&mut self, bytes: &[u8]) -> Result<ImportResult, LibraryError> {
        self.import_fit_bytes_with_progress(bytes, &mut |_| {})
    }

    pub fn import_fit_bytes_with_progress(
        &mut self,
        bytes: &[u8],
        report: &mut impl FnMut(LibraryProgress),
    ) -> Result<ImportResult, LibraryError> {
        let activity = measure(LibraryStage::FitParsing, report, || {
            Ok(import_fit_activity(bytes)?)
        })?;
        let identity = activity.source.identity.clone();
        let present = measure(LibraryStage::DuplicateCheck, report, || {
            if self.find_activity(&identity)?.is_some() {
                self.read_original_bytes(&identity)?;
                Ok(true)
            } else {
                Ok(false)
            }
        })?;
        if present {
            return Ok(ImportResult {
                identity,
                status: ImportStatus::AlreadyPresent,
            });
        }

        let object_name = random_object_name()?;
        let temporary_path = self.objects.join(format!(".tmp-{object_name}"));
        let final_path = self.objects.join(&object_name);
        let encrypted = measure(LibraryStage::Encryption, report, || {
            encrypt_original(&self.object_key, &identity, bytes)
        })?;
        measure(LibraryStage::FileWriteSync, report, || {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary_path)?;
            file.write_all(&encrypted)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary_path, &final_path)?;
            sync_directory(&self.objects)?;
            Ok(())
        })?;

        // The object is durable before the row is committed. A failed or interrupted commit
        // leaves an orphan; open() removes it after checking committed references.
        let transaction = measure(LibraryStage::SampleInserts, report, || {
            let transaction = self.database.transaction()?;
            save_activity(&transaction, &activity, &object_name)?;
            Ok(transaction)
        })?;
        measure(LibraryStage::DatabaseCommit, report, || {
            Ok(transaction.commit()?)
        })?;
        Ok(ImportResult {
            identity,
            status: ImportStatus::Saved,
        })
    }

    /// List source identities without loading sample rows or original bytes.
    pub fn list_activity_ids(&self) -> Result<Vec<SourceIdentity>, LibraryError> {
        let mut statement = self
            .database
            .prepare("SELECT source_sha256 FROM activities ORDER BY start_unix_ms DESC")?;
        let hashes = statement.query_map([], |row| row.get::<_, Vec<u8>>(0))?;
        hashes
            .map(|hash| source_identity(hash.map_err(LibraryError::Database)?))
            .collect()
    }

    /// Read canonical activity facts saved in SQLCipher. Missing identity is normal.
    pub fn find_activity(
        &self,
        identity: &SourceIdentity,
    ) -> Result<Option<ImportedActivity>, LibraryError> {
        let row = self
            .database
            .query_row(
                "SELECT object_name, manufacturer_id, product_id, created_at_unix_ms, sport, \
                 start_unix_ms, end_unix_ms, total_distance_m FROM activities WHERE source_sha256 = ?1",
                [identity.sha256.as_slice()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, Option<f64>>(7)?,
                    ))
                },
            )
            .optional()?;
        let Some((object_name, manufacturer, product, created, sport, start, end, distance)) = row
        else {
            return Ok(None);
        };
        if !valid_object_name(&object_name) || !self.objects.join(&object_name).is_file() {
            return Err(LibraryError::Incomplete);
        }
        let sport = match sport {
            1 => Sport::Running,
            2 => Sport::Other,
            0 => Sport::Unknown,
            _ => return Err(LibraryError::Incomplete),
        };
        let manufacturer_id = manufacturer
            .map(u16::try_from)
            .transpose()
            .map_err(|_| LibraryError::Incomplete)?;
        let product_id = product
            .map(u16::try_from)
            .transpose()
            .map_err(|_| LibraryError::Incomplete)?;
        let mut statement = self.database.prepare(
            "SELECT timestamp_unix_ms, distance_m, speed_m_s, heart_rate_bpm \
             FROM samples WHERE source_sha256 = ?1 ORDER BY sample_index",
        )?;
        let rows = statement.query_map([identity.sha256.as_slice()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<f64>>(1)?,
                row.get::<_, Option<f64>>(2)?,
                row.get::<_, Option<i64>>(3)?,
            ))
        })?;
        let mut samples = Vec::new();
        for row in rows {
            let (timestamp_unix_ms, distance_m, speed_m_s, heart_rate_bpm) = row?;
            samples.push(ActivitySample {
                timestamp_unix_ms,
                distance_m,
                speed_m_s,
                heart_rate_bpm: heart_rate_bpm
                    .map(u8::try_from)
                    .transpose()
                    .map_err(|_| LibraryError::Incomplete)?,
            });
        }
        Ok(Some(ImportedActivity {
            source: ActivitySource {
                identity: identity.clone(),
                provenance: FitProvenance {
                    manufacturer_id,
                    product_id,
                    created_at_unix_ms: created,
                },
            },
            data: ActivityData {
                sport,
                start_unix_ms: start,
                end_unix_ms: end,
                total_distance_m: distance,
                samples,
            },
        }))
    }

    /// Decrypt the immutable source object and verify its content hash.
    pub fn read_original_bytes(
        &self,
        identity: &SourceIdentity,
    ) -> Result<Option<Vec<u8>>, LibraryError> {
        let object_name: Option<String> = self
            .database
            .query_row(
                "SELECT object_name FROM activities WHERE source_sha256 = ?1",
                [identity.sha256.as_slice()],
                |row| row.get(0),
            )
            .optional()?;
        let Some(object_name) = object_name else {
            return Ok(None);
        };
        if !valid_object_name(&object_name) {
            return Err(LibraryError::Incomplete);
        }
        let mut encrypted = Vec::new();
        File::open(self.objects.join(object_name))?
            .take((MAX_FIT_BYTES + OBJECT_OVERHEAD + 1) as u64)
            .read_to_end(&mut encrypted)?;
        if encrypted.len() > MAX_FIT_BYTES + OBJECT_OVERHEAD {
            return Err(LibraryError::CorruptOriginal);
        }
        decrypt_original(&self.object_key, identity, &encrypted).map(Some)
    }

    fn recover_orphans(&self) -> Result<(), LibraryError> {
        let mut statement = self
            .database
            .prepare("SELECT object_name FROM activities")?;
        let names = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut referenced = HashSet::new();
        for name in names {
            let name = name?;
            if !valid_object_name(&name) || !self.objects.join(&name).is_file() {
                return Err(LibraryError::Incomplete);
            }
            referenced.insert(name);
        }
        for entry in fs::read_dir(&self.objects)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if (valid_object_name(&name) && !referenced.contains(name.as_ref()))
                || name.strip_prefix(".tmp-").is_some_and(valid_object_name)
            {
                fs::remove_file(entry.path())?;
            }
        }
        sync_directory(&self.objects)?;
        Ok(())
    }
}

fn derive_key(secret: &[u8; 32], purpose: &[u8]) -> Result<Zeroizing<[u8; 32]>, LibraryError> {
    let mut key = Zeroizing::new([0; 32]);
    Hkdf::<Sha256>::new(None, secret)
        .expand(purpose, &mut *key)
        .map_err(|_| LibraryError::EncryptionUnavailable)?;
    Ok(key)
}

fn set_database_key(database: &Connection, key: &[u8; 32]) -> Result<(), LibraryError> {
    let mut sql = Zeroizing::new(String::from("PRAGMA key = \"x'"));
    for byte in key {
        write!(&mut *sql, "{byte:02x}").map_err(|_| LibraryError::EncryptionUnavailable)?;
    }
    sql.push_str("'\";");
    database.execute_batch(&sql)?;
    Ok(())
}

fn create_schema(database: &mut Connection) -> Result<(), LibraryError> {
    let transaction = database.transaction()?;
    transaction.execute_batch(
        "CREATE TABLE activities (
            source_sha256 BLOB PRIMARY KEY CHECK(length(source_sha256) = 32),
            object_name TEXT NOT NULL UNIQUE,
            manufacturer_id INTEGER,
            product_id INTEGER,
            created_at_unix_ms INTEGER,
            sport INTEGER NOT NULL,
            start_unix_ms INTEGER NOT NULL,
            end_unix_ms INTEGER NOT NULL,
            total_distance_m REAL
        );
        CREATE TABLE samples (
            source_sha256 BLOB NOT NULL REFERENCES activities(source_sha256),
            sample_index INTEGER NOT NULL,
            timestamp_unix_ms INTEGER NOT NULL,
            distance_m REAL,
            speed_m_s REAL,
            heart_rate_bpm INTEGER,
            PRIMARY KEY(source_sha256, sample_index)
        );
        PRAGMA user_version = 1;",
    )?;
    transaction.commit()?;
    Ok(())
}

fn save_activity(
    transaction: &rusqlite::Transaction<'_>,
    activity: &ImportedActivity,
    object_name: &str,
) -> Result<(), LibraryError> {
    let sport = match activity.data.sport {
        Sport::Unknown => 0,
        Sport::Running => 1,
        Sport::Other => 2,
    };
    transaction.execute(
        "INSERT INTO activities VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            activity.source.identity.sha256.as_slice(),
            object_name,
            activity.source.provenance.manufacturer_id,
            activity.source.provenance.product_id,
            activity.source.provenance.created_at_unix_ms,
            sport,
            activity.data.start_unix_ms,
            activity.data.end_unix_ms,
            activity.data.total_distance_m,
        ],
    )?;
    let mut insert = transaction.prepare("INSERT INTO samples VALUES (?1, ?2, ?3, ?4, ?5, ?6)")?;
    for (index, sample) in activity.data.samples.iter().enumerate() {
        insert.execute(params![
            activity.source.identity.sha256.as_slice(),
            index as i64,
            sample.timestamp_unix_ms,
            sample.distance_m,
            sample.speed_m_s,
            sample.heart_rate_bpm,
        ])?;
    }
    Ok(())
}

fn random_object_name() -> Result<String, LibraryError> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(|_| LibraryError::Randomness)?;
    let mut name = String::with_capacity(32 + ".fitenc".len());
    for byte in random {
        write!(&mut name, "{byte:02x}").map_err(|_| LibraryError::Randomness)?;
    }
    name.push_str(".fitenc");
    Ok(name)
}

fn valid_object_name(name: &str) -> bool {
    name.len() == 39
        && name.ends_with(".fitenc")
        && name[..32].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn encrypt_original(
    key: &[u8; 32],
    identity: &SourceIdentity,
    bytes: &[u8],
) -> Result<Vec<u8>, LibraryError> {
    let mut nonce = [0_u8; NONCE_BYTES];
    getrandom::fill(&mut nonce).map_err(|_| LibraryError::Randomness)?;
    let cipher = XChaCha20Poly1305::new(key.into());
    let ciphertext = cipher
        .encrypt(
            &XNonce::from(nonce),
            Payload {
                msg: bytes,
                aad: &identity.sha256,
            },
        )
        .map_err(|_| LibraryError::CorruptOriginal)?;
    let mut result = Vec::with_capacity(OBJECT_MAGIC.len() + NONCE_BYTES + ciphertext.len());
    result.extend_from_slice(OBJECT_MAGIC);
    result.extend_from_slice(&nonce);
    result.extend_from_slice(&ciphertext);
    Ok(result)
}

fn decrypt_original(
    key: &[u8; 32],
    identity: &SourceIdentity,
    encrypted: &[u8],
) -> Result<Vec<u8>, LibraryError> {
    if encrypted.len() < OBJECT_OVERHEAD || !encrypted.starts_with(OBJECT_MAGIC) {
        return Err(LibraryError::CorruptOriginal);
    }
    let (nonce, ciphertext) = encrypted[OBJECT_MAGIC.len()..].split_at(NONCE_BYTES);
    let plaintext = XChaCha20Poly1305::new(key.into())
        .decrypt(
            &XNonce::try_from(nonce).map_err(|_| LibraryError::CorruptOriginal)?,
            Payload {
                msg: ciphertext,
                aad: &identity.sha256,
            },
        )
        .map_err(|_| LibraryError::CorruptOriginal)?;
    if Sha256::digest(&plaintext).as_slice() != identity.sha256 {
        return Err(LibraryError::CorruptOriginal);
    }
    Ok(plaintext)
}

fn source_identity(hash: Vec<u8>) -> Result<SourceIdentity, LibraryError> {
    let sha256 = hash.try_into().map_err(|_| LibraryError::Incomplete)?;
    Ok(SourceIdentity { sha256 })
}

#[cfg(unix)]
fn sync_directory(directory: &Path) -> Result<(), LibraryError> {
    File::open(directory)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_directory(_: &Path) -> Result<(), LibraryError> {
    Ok(())
}
