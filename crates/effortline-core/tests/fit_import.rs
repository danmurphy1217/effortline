//! All FIT bytes in this file are synthetic. They are not device exports.

use effortline_core::fit_import::{
    import_fit_activity, ImportError, Sport, MAX_FIT_BYTES, MAX_FIT_RECORDS,
};

const FIT_TIME: u32 = 1_068_934_400; // 2023-11-14 22:13:20 UTC, relative to FIT epoch.

fn definition(local: u8, global: u16, fields: &[(u8, u8, u8)]) -> Vec<u8> {
    let mut bytes = vec![0x40 | local, 0, 0];
    bytes.extend(global.to_le_bytes());
    bytes.push(fields.len() as u8);
    for &(number, size, base_type) in fields {
        bytes.extend([number, size, base_type]);
    }
    bytes
}

fn crc16(bytes: &[u8]) -> u16 {
    let mut crc = 0u16;
    for byte in bytes {
        crc ^= u16::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xA001
            } else {
                crc >> 1
            };
        }
    }
    crc
}

fn synthetic_fit(file_type: u8, optional_fields: bool, sample_count: usize) -> Vec<u8> {
    let mut data = definition(0, 0, &[(0, 1, 0), (1, 2, 0x84), (2, 2, 0x84), (4, 4, 0x86)]);
    data.push(0);
    data.push(file_type);
    data.extend(1u16.to_le_bytes());
    data.extend(42u16.to_le_bytes());
    data.extend(FIT_TIME.to_le_bytes());

    data.extend(definition(
        1,
        18,
        &[(2, 4, 0x86), (5, 1, 0), (9, 4, 0x86), (253, 4, 0x86)],
    ));
    data.push(1);
    data.extend(FIT_TIME.to_le_bytes());
    data.push(1); // running
    data.extend(100_000_u32.to_le_bytes()); // 1 km in 1/100 m
    data.extend((FIT_TIME + sample_count as u32).to_le_bytes());

    let fields = if optional_fields {
        vec![(253, 4, 0x86), (5, 4, 0x86), (6, 2, 0x84), (3, 1, 0x02)]
    } else {
        vec![(253, 4, 0x86)]
    };
    data.extend(definition(2, 20, &fields));
    for index in 0..sample_count {
        data.push(2);
        data.extend((FIT_TIME + index as u32).to_le_bytes());
        if optional_fields {
            data.extend((index as u32 * 100).to_le_bytes());
            data.extend(3_000u16.to_le_bytes());
            data.push(140);
        }
    }

    let mut bytes = vec![12, 0x20, 0, 0];
    bytes.extend((data.len() as u32).to_le_bytes());
    bytes.extend(b".FIT");
    bytes.extend(data);
    bytes.extend(crc16(&bytes).to_le_bytes());
    bytes
}

#[test]
fn imports_synthetic_running_activity_with_units_and_source_identity() {
    let bytes = synthetic_fit(4, true, 2);
    let activity = import_fit_activity(&bytes).unwrap();
    assert_eq!(activity.data.sport, Sport::Running);
    assert_eq!(activity.data.start_unix_ms, 1_700_000_000_000);
    assert_eq!(activity.data.end_unix_ms, 1_700_000_002_000);
    assert_eq!(activity.data.total_distance_m, Some(1000.0));
    assert_eq!(activity.data.samples.len(), 2);
    assert_eq!(activity.data.samples[0].speed_m_s, Some(3.0));
    assert_eq!(activity.data.samples[0].heart_rate_bpm, Some(140));
    assert_eq!(activity.data.samples[1].distance_m, Some(1.0));
    assert_eq!(activity.source.provenance.manufacturer_id, Some(1));
    assert_eq!(activity.source.provenance.product_id, Some(42));
    assert_eq!(
        activity.source.identity,
        import_fit_activity(&bytes).unwrap().source.identity
    );
    let mut valid_change = bytes.clone();
    valid_change[34] = 43; // Synthetic file_id product field.
    valid_change.truncate(valid_change.len() - 2);
    valid_change.extend(crc16(&valid_change).to_le_bytes());
    assert_ne!(
        activity.source.identity,
        import_fit_activity(&valid_change).unwrap().source.identity
    );
    let mut changed = bytes.clone();
    let last = changed.len() - 1;
    changed[last] ^= 1;
    assert_eq!(import_fit_activity(&changed), Err(ImportError::Corrupt));
}

#[test]
fn keeps_missing_optional_sample_values_absent() {
    let activity = import_fit_activity(&synthetic_fit(4, false, 1)).unwrap();
    let sample = &activity.data.samples[0];
    assert_eq!(sample.distance_m, None);
    assert_eq!(sample.speed_m_s, None);
    assert_eq!(sample.heart_rate_bpm, None);
}

#[test]
fn rejects_non_activity_and_bad_inputs_with_typed_errors() {
    assert_eq!(import_fit_activity(&[]), Err(ImportError::Truncated));
    let valid = synthetic_fit(4, true, 1);
    assert_eq!(
        import_fit_activity(&valid[..valid.len() - 1]),
        Err(ImportError::Truncated)
    );
    let mut trailing_junk = valid.clone();
    trailing_junk.push(0);
    assert_eq!(
        import_fit_activity(&trailing_junk),
        Err(ImportError::Corrupt)
    );
    let mut wrong_signature = valid.clone();
    wrong_signature[8] = b'X';
    assert_eq!(
        import_fit_activity(&wrong_signature),
        Err(ImportError::Corrupt)
    );
    let mut new_protocol = valid.clone();
    new_protocol[1] = 0x30;
    assert_eq!(
        import_fit_activity(&new_protocol),
        Err(ImportError::Unsupported)
    );
    assert_eq!(
        import_fit_activity(&synthetic_fit(5, true, 1)),
        Err(ImportError::NotActivity)
    );
    assert_eq!(
        import_fit_activity(&vec![0; MAX_FIT_BYTES + 1]),
        Err(ImportError::TooLarge)
    );
    assert_eq!(ImportError::Corrupt.code(), "fit_corrupt");
}

#[test]
fn rejects_invalid_synthetic_sample_timestamp() {
    let mut bytes = synthetic_fit(4, false, 1);
    let timestamp_start = bytes.len() - 2 - 4;
    bytes[timestamp_start..timestamp_start + 4].fill(0xff);
    bytes.truncate(bytes.len() - 2);
    bytes.extend(crc16(&bytes).to_le_bytes());
    assert_eq!(import_fit_activity(&bytes), Err(ImportError::Corrupt));
}

#[test]
fn rejects_too_many_synthetic_data_messages() {
    let bytes = synthetic_fit(4, false, MAX_FIT_RECORDS);
    assert_eq!(
        import_fit_activity(&bytes),
        Err(ImportError::TooManyRecords)
    );
}
