//! Provider-neutral import of one FIT activity from untrusted bytes.

use fitparser::de::{DecodeOption, FitObject, FitStreamProcessor};
use fitparser::profile::MesgNum;
use fitparser::{ErrorKind, FitDataField, FitDataRecord, Value};
use sha2::{Digest, Sha256};

/// Maximum FIT file size accepted by this importer (16 MiB).
pub const MAX_FIT_BYTES: usize = 16 * 1024 * 1024;
/// Maximum number of FIT data messages accepted, including non-sample messages.
pub const MAX_FIT_RECORDS: usize = 100_000;

/// Stable error code for import failures. No raw file contents enter an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportError {
    TooLarge,
    TooManyRecords,
    Truncated,
    Corrupt,
    Unsupported,
    NotActivity,
}

impl ImportError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::TooLarge => "fit_too_large",
            Self::TooManyRecords => "fit_too_many_records",
            Self::Truncated => "fit_truncated",
            Self::Corrupt => "fit_corrupt",
            Self::Unsupported => "fit_unsupported",
            Self::NotActivity => "fit_not_activity",
        }
    }
}

/// Identity of the exact source bytes. Equal hashes indicate a repeat import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceIdentity {
    pub sha256: [u8; 32],
}

/// Metadata reported by the FIT file. These values do not define source identity.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FitProvenance {
    pub manufacturer_id: Option<u16>,
    pub product_id: Option<u16>,
    pub created_at_unix_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivitySource {
    pub identity: SourceIdentity,
    pub provenance: FitProvenance,
}

/// Sport from the session message. Only `Running` is suitable for the first comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sport {
    Running,
    Other,
    Unknown,
}

/// An observed FIT sample. Optional values are absent, never imputed.
#[derive(Debug, Clone, PartialEq)]
pub struct ActivitySample {
    pub timestamp_unix_ms: i64,
    pub distance_m: Option<f64>,
    pub speed_m_s: Option<f64>,
    pub heart_rate_bpm: Option<u8>,
}

/// Imported source facts. Speed is observed FIT speed, not a derived pace.
#[derive(Debug, Clone, PartialEq)]
pub struct ActivityData {
    pub sport: Sport,
    pub start_unix_ms: i64,
    pub end_unix_ms: i64,
    pub total_distance_m: Option<f64>,
    pub samples: Vec<ActivitySample>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportedActivity {
    pub source: ActivitySource,
    pub data: ActivityData,
}

/// Import one FIT activity. The caller keeps the original bytes for later encrypted storage.
/// The same bytes always produce the same source identity; this function writes nothing.
pub fn import_fit_activity(bytes: &[u8]) -> Result<ImportedActivity, ImportError> {
    validate_envelope(bytes)?;

    let mut processor = FitStreamProcessor::new();
    processor.add_option(DecodeOption::ReturnNumericEnumValues);
    let mut remaining = bytes;
    let mut data_messages = 0;
    let mut saw_crc = false;
    let mut file_type = None;
    let mut provenance = FitProvenance::default();
    let mut sport = Sport::Unknown;
    let mut session_start = None;
    let mut session_end = None;
    let mut session_count = 0;
    let mut total_distance_m = None;
    let mut samples = Vec::new();

    while !remaining.is_empty() {
        let (next, object) = processor
            .deserialize_next(remaining)
            .map_err(|error| map_parser_error(&error))?;
        if next.len() >= remaining.len() {
            return Err(ImportError::Corrupt);
        }
        remaining = next;
        match object {
            FitObject::Crc(_) => saw_crc = true,
            FitObject::Header(_) | FitObject::DefinitionMessage(_) => {}
            FitObject::DataMessage(message) => {
                data_messages += 1;
                if data_messages > MAX_FIT_RECORDS {
                    return Err(ImportError::TooManyRecords);
                }
                let invalid_timestamp = message
                    .fields()
                    .get(&253)
                    .is_some_and(|value| *value == Value::Invalid);
                let invalid_session_start = message
                    .fields()
                    .get(&2)
                    .is_some_and(|value| *value == Value::Invalid);
                let invalid_created_at = message
                    .fields()
                    .get(&4)
                    .is_some_and(|value| *value == Value::Invalid);
                let record = processor
                    .decode_message(message)
                    .map_err(|error| map_parser_error(&error))?;
                match record.kind() {
                    MesgNum::FileId => {
                        file_type = field(&record, "type").and_then(numeric_enum);
                        provenance.manufacturer_id =
                            field(&record, "manufacturer").and_then(u16_value);
                        provenance.product_id = record
                            .fields()
                            .iter()
                            .find(|field| field.number() == 2)
                            .and_then(u16_value);
                        provenance.created_at_unix_ms = (!invalid_created_at)
                            .then(|| field(&record, "time_created").and_then(timestamp_ms))
                            .flatten();
                    }
                    MesgNum::Session => {
                        session_count += 1;
                        if session_count > 1 {
                            return Err(ImportError::Unsupported);
                        }
                        sport = match field(&record, "sport").and_then(numeric_enum) {
                            Some(1) => Sport::Running,
                            Some(_) => Sport::Other,
                            None => Sport::Unknown,
                        };
                        session_start = (!invalid_session_start)
                            .then(|| field(&record, "start_time").and_then(timestamp_ms))
                            .flatten();
                        session_end = (!invalid_timestamp)
                            .then(|| field(&record, "timestamp").and_then(timestamp_ms))
                            .flatten();
                        total_distance_m = field(&record, "total_distance").and_then(nonnegative);
                    }
                    MesgNum::Record => {
                        if invalid_timestamp {
                            return Err(ImportError::Corrupt);
                        }
                        let timestamp_unix_ms = field(&record, "timestamp")
                            .and_then(timestamp_ms)
                            .ok_or(ImportError::Corrupt)?;
                        let speed_m_s = field(&record, "enhanced_speed")
                            .and_then(nonnegative)
                            .or_else(|| field(&record, "speed").and_then(nonnegative));
                        samples.push(ActivitySample {
                            timestamp_unix_ms,
                            distance_m: field(&record, "distance").and_then(nonnegative),
                            speed_m_s,
                            heart_rate_bpm: field(&record, "heart_rate").and_then(u8_value),
                        });
                    }
                    _ => {}
                }
            }
        }
    }

    if !saw_crc {
        return Err(ImportError::Truncated);
    }
    if file_type != Some(4) || samples.is_empty() {
        return Err(ImportError::NotActivity);
    }
    let first = samples
        .first()
        .ok_or(ImportError::NotActivity)?
        .timestamp_unix_ms;
    let last = samples
        .last()
        .ok_or(ImportError::NotActivity)?
        .timestamp_unix_ms;
    let start_unix_ms = session_start.unwrap_or(first);
    let end_unix_ms = session_end.unwrap_or(last);
    if end_unix_ms < start_unix_ms
        || samples
            .windows(2)
            .any(|pair| pair[1].timestamp_unix_ms < pair[0].timestamp_unix_ms)
    {
        return Err(ImportError::Corrupt);
    }

    Ok(ImportedActivity {
        source: ActivitySource {
            identity: SourceIdentity {
                sha256: Sha256::digest(bytes).into(),
            },
            provenance,
        },
        data: ActivityData {
            sport,
            start_unix_ms,
            end_unix_ms,
            total_distance_m,
            samples,
        },
    })
}

fn validate_envelope(bytes: &[u8]) -> Result<(), ImportError> {
    if bytes.len() > MAX_FIT_BYTES {
        return Err(ImportError::TooLarge);
    }
    if bytes.len() < 12 {
        return Err(ImportError::Truncated);
    }
    let header_size = bytes[0] as usize;
    if header_size != 12 && header_size != 14 {
        return Err(ImportError::Unsupported);
    }
    if &bytes[8..12] != b".FIT" {
        return Err(ImportError::Corrupt);
    }
    if bytes[1] >> 4 > 2 {
        return Err(ImportError::Unsupported);
    }
    let data_size =
        u32::from_le_bytes(bytes[4..8].try_into().map_err(|_| ImportError::Corrupt)?) as usize;
    let expected = header_size
        .checked_add(data_size)
        .and_then(|size| size.checked_add(2))
        .ok_or(ImportError::TooLarge)?;
    if expected > bytes.len() {
        return Err(ImportError::Truncated);
    }
    if expected < bytes.len() {
        let trailing = &bytes[expected..];
        if trailing.len() >= 12
            && (trailing[0] == 12 || trailing[0] == 14)
            && trailing.get(8..12) == Some(b".FIT")
        {
            return Err(ImportError::Unsupported);
        }
        return Err(ImportError::Corrupt);
    }
    Ok(())
}

fn map_parser_error(error: &ErrorKind) -> ImportError {
    match error {
        ErrorKind::UnexpectedEof(_) => ImportError::Truncated,
        _ => ImportError::Corrupt,
    }
}

fn field<'a>(record: &'a FitDataRecord, name: &str) -> Option<&'a FitDataField> {
    record.fields().iter().find(|value| value.name() == name)
}

fn numeric_enum(field: &FitDataField) -> Option<i64> {
    match field.value() {
        Value::SInt64(value) => Some(*value),
        _ => None,
    }
}

fn timestamp_ms(field: &FitDataField) -> Option<i64> {
    match field.value() {
        Value::Timestamp(value) => Some(value.timestamp_millis()),
        _ => None,
    }
}

fn nonnegative(field: &FitDataField) -> Option<f64> {
    let value = match field.value() {
        Value::Float64(value) => *value,
        Value::Float32(value) => f64::from(*value),
        Value::UInt16(value) => f64::from(*value),
        Value::UInt32(value) => f64::from(*value),
        _ => return None,
    };
    value
        .is_finite()
        .then_some(value)
        .filter(|value| *value >= 0.0)
}

fn u8_value(field: &FitDataField) -> Option<u8> {
    match field.value() {
        Value::UInt8(value) => Some(*value),
        _ => None,
    }
}

fn u16_value(field: &FitDataField) -> Option<u16> {
    match field.value() {
        Value::UInt16(value) => Some(*value),
        Value::SInt64(value) => u16::try_from(*value).ok(),
        _ => None,
    }
}
