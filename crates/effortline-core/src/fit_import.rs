//! Provider-neutral import of one FIT activity from untrusted bytes.

use fitparser::de::{DecodeOption, FitObject, FitStreamProcessor};
use fitparser::profile::MesgNum;
use fitparser::{ErrorKind, FitDataField, FitDataRecord, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// Maximum FIT file size accepted by this importer (16 MiB).
pub const MAX_FIT_BYTES: usize = 16 * 1024 * 1024;
/// Maximum number of FIT data messages accepted, including non-sample messages.
pub const MAX_FIT_RECORDS: usize = 100_000;
/// Maximum number of FIT definition messages accepted.
pub const MAX_FIT_DEFINITIONS: usize = 10_000;

/// Stable error code for import failures. No raw file contents enter an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportError {
    TooLarge,
    TooManyRecords,
    TooManyDefinitions,
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
            Self::TooManyDefinitions => "fit_too_many_definitions",
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

struct DefinitionLayout {
    data_size: usize,
    developer_fields: Vec<(u8, u8, u8)>, // developer index, field number, byte size
}

/// Import one FIT activity. The caller keeps the original bytes for later encrypted storage.
/// The same bytes always produce the same source identity; this function writes nothing.
pub fn import_fit_activity(bytes: &[u8]) -> Result<ImportedActivity, ImportError> {
    let data_end = validate_envelope(bytes)?;
    validate_data_crc(bytes, data_end)?;

    let mut processor = FitStreamProcessor::new();
    let mut definitions: [Option<DefinitionLayout>; 16] = std::array::from_fn(|_| None);
    let mut developer_widths: HashMap<(u8, u8), u8> = HashMap::new();
    let mut remaining = bytes;
    let mut data_messages = 0;
    let mut definition_messages = 0;
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
        let offset = bytes.len() - remaining.len();
        let mut normalized_definition = None;
        if offset >= usize::from(bytes[0]) && offset < data_end {
            let header = remaining[0];
            let data_remaining = data_end - offset;
            if header & 0x80 == 0 && header & 0x40 != 0 {
                definition_messages += 1;
                if definition_messages > MAX_FIT_DEFINITIONS {
                    return Err(ImportError::TooManyDefinitions);
                }
                let (local, layout, normalized) = check_definition(&remaining[..data_remaining])?;
                definitions[usize::from(local)] = Some(layout);
                normalized_definition = normalized;
            } else {
                let local = if header & 0x80 != 0 {
                    (header >> 5) & 0x03
                } else {
                    header & 0x0f
                };
                let layout = definitions[usize::from(local)]
                    .as_ref()
                    .ok_or(ImportError::Corrupt)?;
                if layout.data_size > data_remaining {
                    return Err(ImportError::Corrupt);
                }
                for &(developer_index, field_number, size) in &layout.developer_fields {
                    let width = developer_widths
                        .get(&(developer_index, field_number))
                        .ok_or(ImportError::Corrupt)?;
                    if size % width != 0 {
                        return Err(ImportError::Corrupt);
                    }
                }
            }
        }
        let (consumed, object) = if let Some(normalized) = normalized_definition.as_deref() {
            // The original CRC was checked above. The parser sees the same short event field as
            // bytes, but its running CRC now differs from the original stream.
            processor.add_option(DecodeOption::SkipDataCrcValidation);
            let (next, object) = processor
                .deserialize_next(normalized)
                .map_err(|error| map_parser_error(&error))?;
            (normalized.len() - next.len(), object)
        } else {
            let (next, object) = processor
                .deserialize_next(remaining)
                .map_err(|error| map_parser_error(&error))?;
            (remaining.len() - next.len(), object)
        };
        if consumed == 0 {
            return Err(ImportError::Corrupt);
        }
        remaining = &remaining[consumed..];
        match object {
            FitObject::Crc(_) => saw_crc = true,
            FitObject::Header(_) | FitObject::DefinitionMessage(_) => {}
            FitObject::DataMessage(message) => {
                data_messages += 1;
                if data_messages > MAX_FIT_RECORDS {
                    return Err(ImportError::TooManyRecords);
                }
                let raw = message.fields();
                let raw_file_type = raw.get(&0).and_then(enum_value);
                let raw_manufacturer = raw.get(&1).and_then(uint16_value);
                let raw_product = raw.get(&2).and_then(uint16_value);
                let raw_sport = raw.get(&5).and_then(enum_value);
                let raw_distance = raw.get(&5).and_then(uint32_value);
                let raw_speed = raw
                    .get(&73)
                    .and_then(uint32_value)
                    .map(u64::from)
                    .or_else(|| raw.get(&6).and_then(uint16_value).map(u64::from));
                let raw_heart_rate = raw.get(&3).and_then(uint8_value);
                let compressed_timestamp = message.time_offset().is_some();
                let developer_count = message.developer_fields().len();
                let developer_description = if message.global_message_number() == 206 {
                    raw.get(&0)
                        .and_then(unsigned_value)
                        .zip(raw.get(&1).and_then(unsigned_value))
                        .zip(raw.get(&2).and_then(unsigned_value))
                        .and_then(|((index, number), base_type)| {
                            Some((
                                u8::try_from(index).ok()?,
                                u8::try_from(number).ok()?,
                                u8::try_from(base_type).ok()?,
                            ))
                        })
                } else {
                    None
                };
                let has_timestamp = raw.contains_key(&253) || compressed_timestamp;
                let has_start_time = raw.contains_key(&2);
                let has_created_at = raw.contains_key(&4);
                let has_total_distance = raw.contains_key(&9);
                let has_compressed_speed_distance = raw.contains_key(&8);
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
                // fitparser appends developer values after profile fields, then a compressed timestamp.
                let standard_count = record
                    .fields()
                    .len()
                    .checked_sub(developer_count + usize::from(compressed_timestamp))
                    .ok_or(ImportError::Corrupt)?;
                let standard = &record.fields()[..standard_count];
                match record.kind() {
                    MesgNum::FieldDescription => {
                        if let Some((index, number, base_type)) = developer_description {
                            developer_widths.insert((index, number), base_type_width(base_type));
                        }
                    }
                    MesgNum::FileId => {
                        file_type = raw_file_type;
                        provenance.manufacturer_id = raw_manufacturer;
                        provenance.product_id = raw_product;
                        provenance.created_at_unix_ms = (has_created_at && !invalid_created_at)
                            .then(|| standard_field(standard, 4, "").and_then(timestamp_ms))
                            .flatten();
                    }
                    MesgNum::Session => {
                        session_count += 1;
                        if session_count > 1 {
                            return Err(ImportError::Unsupported);
                        }
                        sport = match raw_sport {
                            Some(1) => Sport::Running,
                            Some(_) => Sport::Other,
                            None => Sport::Unknown,
                        };
                        session_start = (has_start_time && !invalid_session_start)
                            .then(|| standard_field(standard, 2, "").and_then(timestamp_ms))
                            .flatten();
                        session_end = (has_timestamp && !invalid_timestamp)
                            .then(|| standard_timestamp(standard, &record, compressed_timestamp))
                            .flatten();
                        total_distance_m = has_total_distance
                            .then(|| standard_field(standard, 9, "m").and_then(nonnegative))
                            .flatten();
                    }
                    MesgNum::Record => {
                        if invalid_timestamp {
                            return Err(ImportError::Corrupt);
                        }
                        let timestamp_unix_ms = has_timestamp
                            .then(|| standard_timestamp(standard, &record, compressed_timestamp))
                            .flatten()
                            .ok_or(ImportError::Corrupt)?;
                        let speed_m_s =
                            raw_speed.map(|value| value as f64 / 1000.0).or_else(|| {
                                has_compressed_speed_distance
                                    .then(|| {
                                        standard_field(standard, 73, "m/s")
                                            .or_else(|| standard_field(standard, 6, "m/s"))
                                            .and_then(nonnegative)
                                    })
                                    .flatten()
                            });
                        let distance_m =
                            raw_distance.map(|value| value as f64 / 100.0).or_else(|| {
                                has_compressed_speed_distance
                                    .then(|| standard_field(standard, 5, "m").and_then(nonnegative))
                                    .flatten()
                            });
                        samples.push(ActivitySample {
                            timestamp_unix_ms,
                            distance_m,
                            speed_m_s,
                            heart_rate_bpm: raw_heart_rate,
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

fn validate_envelope(bytes: &[u8]) -> Result<usize, ImportError> {
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
    Ok(expected - 2)
}

fn check_definition(data: &[u8]) -> Result<(u8, DefinitionLayout, Option<Vec<u8>>), ImportError> {
    // fitparser 0.11.0 prints malformed standard widths and can overflow on malformed developer widths.
    let header = *data.first().ok_or(ImportError::Corrupt)?;
    let field_count = *data.get(5).ok_or(ImportError::Corrupt)? as usize;
    let global_message = match data[2] {
        0 => u16::from_le_bytes([data[3], data[4]]),
        1 => u16::from_be_bytes([data[3], data[4]]),
        _ => return Err(ImportError::Corrupt),
    };
    let fields_end = 6 + field_count * 3;
    let fields = data.get(6..fields_end).ok_or(ImportError::Corrupt)?;
    let mut data_size = 1_usize;
    let mut short_event_type_offsets = Vec::new();
    for (index, field) in fields.as_chunks::<3>().0.iter().enumerate() {
        let size = field[1];
        // Some event messages encode field 3 in one byte despite declaring uint32.
        let short_event_data =
            global_message == 21 && field[0] == 3 && size == 1 && field[2] == 0x86;
        if size % base_type_width(field[2]) != 0 && !short_event_data {
            return Err(ImportError::Corrupt);
        }
        if short_event_data {
            short_event_type_offsets.push(6 + index * 3 + 2);
        }
        data_size += usize::from(size);
    }
    let mut developer_fields = Vec::new();
    let mut definition_end = fields_end;
    if header & 0x20 != 0 {
        let count = *data.get(fields_end).ok_or(ImportError::Corrupt)? as usize;
        let start = fields_end + 1;
        let end = start + count * 3;
        definition_end = end;
        let fields = data.get(start..end).ok_or(ImportError::Corrupt)?;
        for field in fields.as_chunks::<3>().0 {
            developer_fields.push((field[2], field[0], field[1]));
            data_size += usize::from(field[1]);
        }
    }
    let normalized = if short_event_type_offsets.is_empty() {
        None
    } else {
        let mut definition = data[..definition_end].to_vec();
        for offset in short_event_type_offsets {
            definition[offset] = 0x0d; // fitparser's own fallback type for this short field.
        }
        Some(definition)
    };
    Ok((
        header & 0x0f,
        DefinitionLayout {
            data_size,
            developer_fields,
        },
        normalized,
    ))
}

fn validate_data_crc(bytes: &[u8], data_end: usize) -> Result<(), ImportError> {
    let data = if bytes[0] == 14 {
        &bytes[14..data_end]
    } else {
        &bytes[..data_end]
    };
    let expected = u16::from_le_bytes([bytes[data_end], bytes[data_end + 1]]);
    let mut crc = 0_u16;
    for byte in data {
        crc ^= u16::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xA001
            } else {
                crc >> 1
            };
        }
    }
    (crc == expected).then_some(()).ok_or(ImportError::Corrupt)
}

fn base_type_width(base_type: u8) -> u8 {
    match base_type & 0x9f {
        0x83 | 0x84 | 0x8b => 2,
        0x85 | 0x86 | 0x88 | 0x8c => 4,
        0x89 | 0x8e | 0x8f | 0x90 => 8,
        _ => 1,
    }
}

fn map_parser_error(error: &ErrorKind) -> ImportError {
    match error {
        ErrorKind::UnexpectedEof(_) => ImportError::Truncated,
        _ => ImportError::Corrupt,
    }
}

fn standard_field<'a>(
    fields: &'a [FitDataField],
    number: u8,
    units: &str,
) -> Option<&'a FitDataField> {
    fields
        .iter()
        .find(|field| field.number() == number && field.units() == units)
}

fn standard_timestamp(
    standard: &[FitDataField],
    record: &FitDataRecord,
    compressed: bool,
) -> Option<i64> {
    standard_field(standard, 253, "s")
        .and_then(timestamp_ms)
        .or_else(|| {
            if compressed {
                record
                    .fields()
                    .last()
                    .filter(|field| field.number() == 253)
                    .and_then(timestamp_ms)
            } else {
                None
            }
        })
}

fn unsigned_value(value: &Value) -> Option<u64> {
    match value {
        Value::Enum(value) | Value::UInt8(value) | Value::UInt8z(value) => Some(u64::from(*value)),
        Value::UInt16(value) | Value::UInt16z(value) => Some(u64::from(*value)),
        Value::UInt32(value) | Value::UInt32z(value) => Some(u64::from(*value)),
        Value::UInt64(value) | Value::UInt64z(value) => Some(*value),
        _ => None,
    }
}

fn enum_value(value: &Value) -> Option<u8> {
    match value {
        Value::Enum(value) => Some(*value),
        _ => None,
    }
}

fn uint8_value(value: &Value) -> Option<u8> {
    match value {
        Value::UInt8(value) => Some(*value),
        _ => None,
    }
}

fn uint16_value(value: &Value) -> Option<u16> {
    match value {
        Value::UInt16(value) => Some(*value),
        _ => None,
    }
}

fn uint32_value(value: &Value) -> Option<u32> {
    match value {
        Value::UInt32(value) => Some(*value),
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
