//! All FIT bytes in this file are synthetic. They are not device exports.

use effortline_core::fit_import::{
    import_fit_activity, ImportError, Sport, MAX_FIT_BYTES, MAX_FIT_DEFINITIONS, MAX_FIT_RECORDS,
};

mod support;
use support::*;

#[test]
fn imports_synthetic_running_activity_with_units_and_source_identity() {
    let bytes = synthetic_fit(4, true, 2);
    let activity = import_fit_activity(&bytes).unwrap();
    assert_eq!(activity.data.sport, Sport::Running);
    assert_eq!(activity.data.start_unix_ms, 1_699_999_990_000);
    assert_eq!(activity.data.end_unix_ms, 1_700_000_002_000);
    assert_eq!(activity.data.total_distance_m, Some(1000.0));
    assert_eq!(activity.data.samples.len(), 2);
    assert_eq!(activity.data.samples[0].speed_m_s, Some(3.0));
    assert_eq!(activity.data.samples[0].heart_rate_bpm, Some(140));
    assert_eq!(activity.data.samples[1].distance_m, Some(1.0));
    assert_eq!(activity.source.provenance.manufacturer_id, Some(1));
    assert_eq!(activity.source.provenance.product_id, Some(42));
    assert_eq!(
        activity.source.provenance.created_at_unix_ms,
        Some(1_700_000_000_000)
    );
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
fn extends_synthetic_session_end_to_include_later_samples() {
    let bytes = synthetic_fit_with_session_end(4, true, 2, FIT_TIME);
    let activity = import_fit_activity(&bytes).unwrap();
    assert_eq!(activity.data.start_unix_ms, 1_699_999_990_000);
    assert_eq!(activity.data.end_unix_ms, 1_700_000_001_000);
    assert_eq!(activity.data.samples.len(), 2);

    let reversed_session = synthetic_fit_with_session_end(4, true, 2, FIT_TIME - 11);
    assert_eq!(
        import_fit_activity(&reversed_session),
        Err(ImportError::Corrupt)
    );
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

#[test]
fn imports_valid_synthetic_developer_field_without_using_it_as_a_measure() {
    let mut data = activity_data();
    append_developer_description(&mut data, 7, 0x84, "vendor_metric", "mph");
    data.extend(developer_definition(2, 20, &[(253, 4, 0x86)], &[(7, 2, 0)]));
    data.push(2);
    data.extend((FIT_TIME + 1).to_le_bytes());
    data.extend(500_u16.to_le_bytes());
    let activity = import_fit_activity(&fit_file(&data, 12)).unwrap();
    assert_eq!(activity.data.samples.len(), 2);
    assert_eq!(activity.data.samples[1].speed_m_s, None);
}

#[test]
fn ignores_synthetic_developer_fields_that_reuse_standard_names() {
    let mut data = activity_data();
    append_developer_description(&mut data, 1, 0x86, "distance", "yards");
    append_developer_description(&mut data, 2, 0x84, "speed", "mph");
    append_developer_description(&mut data, 3, 0x02, "heart_rate", "%");
    append_developer_description(&mut data, 4, 0x86, "timestamp", "local");
    data.extend(developer_definition(
        2,
        20,
        &[(253, 4, 0x86)],
        &[(1, 4, 0), (2, 2, 0), (3, 1, 0), (4, 4, 0)],
    ));
    data.push(2);
    data.extend((FIT_TIME + 1).to_le_bytes());
    data.extend(500_u32.to_le_bytes());
    data.extend(15_u16.to_le_bytes());
    data.push(99);
    data.extend(123_u32.to_le_bytes());
    let activity = import_fit_activity(&fit_file(&data, 12)).unwrap();
    let sample = &activity.data.samples[1];
    assert_eq!(sample.timestamp_unix_ms, 1_700_000_001_000);
    assert_eq!(sample.distance_m, None);
    assert_eq!(sample.speed_m_s, None);
    assert_eq!(sample.heart_rate_bpm, None);

    data.extend(developer_definition(2, 20, &[], &[(4, 4, 0)]));
    data.push(2);
    data.extend((FIT_TIME + 2).to_le_bytes());
    assert_eq!(
        import_fit_activity(&fit_file(&data, 12)),
        Err(ImportError::Corrupt)
    );
}

#[test]
fn accepts_synthetic_14_byte_header_and_checks_both_crcs() {
    let bytes = fit_file(&activity_data(), 14);
    assert!(import_fit_activity(&bytes).is_ok());
    let mut bad_header = bytes.clone();
    bad_header[12] ^= 1;
    assert_eq!(import_fit_activity(&bad_header), Err(ImportError::Corrupt));
    let mut bad_data = bytes;
    let last = bad_data.len() - 1;
    bad_data[last] ^= 1;
    assert_eq!(import_fit_activity(&bad_data), Err(ImportError::Corrupt));
}

#[test]
fn prefers_synthetic_enhanced_speed_and_falls_back_when_it_is_missing() {
    for (enhanced, expected) in [(4_000_u32, 4.0), (u32::MAX, 2.5)] {
        let mut data = activity_data();
        data.extend(definition(
            2,
            20,
            &[(253, 4, 0x86), (6, 2, 0x84), (73, 4, 0x86)],
        ));
        data.push(2);
        data.extend((FIT_TIME + 1).to_le_bytes());
        data.extend(2_500_u16.to_le_bytes());
        data.extend(enhanced.to_le_bytes());
        let activity = import_fit_activity(&fit_file(&data, 12)).unwrap();
        assert_eq!(activity.data.samples[1].speed_m_s, Some(expected));
    }
}

#[test]
fn reads_synthetic_compressed_standard_timestamp() {
    let mut data = activity_data();
    data.extend(definition(2, 20, &[(5, 4, 0x86)]));
    data.push(0xC1); // compressed timestamp for local message 2, one second later
    data.extend(100_u32.to_le_bytes());
    let activity = import_fit_activity(&fit_file(&data, 12)).unwrap();
    assert_eq!(
        activity.data.samples[1].timestamp_unix_ms,
        1_700_000_001_000
    );
    assert_eq!(activity.data.samples[1].distance_m, Some(1.0));
}

#[test]
fn reads_synthetic_compressed_speed_distance_fallback() {
    let mut data = activity_data();
    data.extend(definition(2, 20, &[(253, 4, 0x86), (8, 3, 0x0d)]));
    data.push(2);
    data.extend((FIT_TIME + 1).to_le_bytes());
    data.extend([0xfa, 0x00, 0x08]); // 12-bit speed 250, then 12-bit distance 128.

    let activity = import_fit_activity(&fit_file(&data, 12)).unwrap();
    assert_eq!(activity.data.samples[1].speed_m_s, Some(2.5));
    assert_eq!(activity.data.samples[1].distance_m, Some(8.0));
}

#[test]
fn rejects_synthetic_definition_flood() {
    let mut data = activity_data();
    for _ in 0..MAX_FIT_DEFINITIONS {
        data.extend(definition(0, 0, &[]));
    }
    assert_eq!(
        import_fit_activity(&fit_file(&data, 12)),
        Err(ImportError::TooManyDefinitions)
    );
}

#[test]
fn rejects_synthetic_malformed_standard_definition_before_decode() {
    let mut data = activity_data();
    data.extend(definition(2, 20, &[(6, 3, 0x84)]));
    assert_eq!(
        import_fit_activity(&fit_file(&data, 12)),
        Err(ImportError::Corrupt)
    );
}

#[test]
fn accepts_synthetic_short_event_data_field_without_changing_activity_measures() {
    let mut data = activity_data();
    data.extend(definition(3, 21, &[(3, 1, 0x86)]));
    data.extend([3, 1]);

    let bytes = fit_file(&data, 12);
    let activity = import_fit_activity(&bytes).unwrap();
    assert_eq!(activity.data.sport, Sport::Running);
    assert_eq!(activity.data.total_distance_m, Some(1000.0));
    assert_eq!(activity.data.samples.len(), 1);
    assert_eq!(activity.data.samples[0].speed_m_s, Some(3.0));
    assert_eq!(activity.data.samples[0].distance_m, Some(0.0));

    let mut bad_crc = bytes;
    *bad_crc.last_mut().unwrap() ^= 1;
    assert_eq!(import_fit_activity(&bad_crc), Err(ImportError::Corrupt));
}

#[test]
fn rejects_synthetic_malformed_developer_size_without_panicking() {
    let mut data = activity_data();
    append_developer_description(&mut data, 7, 0x84, "vendor_metric", "mph");
    data.extend(developer_definition(
        2,
        20,
        &[(253, 4, 0x86)],
        &[(7, 255, 0)],
    ));
    data.push(2);
    data.extend((FIT_TIME + 1).to_le_bytes());
    data.extend([0_u8; 255]);
    assert_eq!(
        import_fit_activity(&fit_file(&data, 12)),
        Err(ImportError::Corrupt)
    );
}
