//! Synthetic FIT builders shared by importer and storage tests.
#![allow(dead_code)]

pub const FIT_TIME: u32 = 1_068_934_400; // 2023-11-14 22:13:20 UTC, relative to FIT epoch.

pub fn definition(local: u8, global: u16, fields: &[(u8, u8, u8)]) -> Vec<u8> {
    let mut bytes = vec![0x40 | local, 0, 0];
    bytes.extend(global.to_le_bytes());
    bytes.push(fields.len() as u8);
    for &(number, size, base_type) in fields {
        bytes.extend([number, size, base_type]);
    }
    bytes
}

pub fn crc16(bytes: &[u8]) -> u16 {
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

pub fn fit_file(data: &[u8], header_size: u8) -> Vec<u8> {
    let mut bytes = vec![header_size, 0x20, 0, 0];
    bytes.extend((data.len() as u32).to_le_bytes());
    bytes.extend(b".FIT");
    if header_size == 14 {
        bytes.extend(crc16(&bytes).to_le_bytes());
    }
    bytes.extend(data);
    let crc_data = if header_size == 14 {
        data
    } else {
        bytes.as_slice()
    };
    bytes.extend(crc16(crc_data).to_le_bytes());
    bytes
}

pub fn activity_data() -> Vec<u8> {
    let bytes = synthetic_fit(4, true, 1);
    bytes[12..bytes.len() - 2].to_vec()
}

pub fn developer_definition(
    local: u8,
    global: u16,
    fields: &[(u8, u8, u8)],
    developer_fields: &[(u8, u8, u8)],
) -> Vec<u8> {
    let mut bytes = definition(local, global, fields);
    bytes[0] |= 0x20;
    bytes.push(developer_fields.len() as u8);
    for &(number, size, developer_index) in developer_fields {
        bytes.extend([number, size, developer_index]);
    }
    bytes
}

pub fn append_developer_description(
    data: &mut Vec<u8>,
    number: u8,
    base_type: u8,
    name: &str,
    units: &str,
) {
    data.extend(definition(
        3,
        206,
        &[
            (0, 1, 0x02),
            (1, 1, 0x02),
            (2, 1, 0),
            (3, 16, 0x07),
            (8, 8, 0x07),
        ],
    ));
    data.push(3);
    data.extend([0, number, base_type]);
    let mut name_bytes = [0; 16];
    name_bytes[..name.len()].copy_from_slice(name.as_bytes());
    data.extend(name_bytes);
    let mut unit_bytes = [0; 8];
    unit_bytes[..units.len()].copy_from_slice(units.as_bytes());
    data.extend(unit_bytes);
}

pub fn synthetic_fit(file_type: u8, optional_fields: bool, sample_count: usize) -> Vec<u8> {
    synthetic_fit_with_session_end(
        file_type,
        optional_fields,
        sample_count,
        FIT_TIME + sample_count as u32,
    )
}

pub fn synthetic_fit_with_session_end(
    file_type: u8,
    optional_fields: bool,
    sample_count: usize,
    session_end: u32,
) -> Vec<u8> {
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
    data.extend((FIT_TIME - 10).to_le_bytes());
    data.push(1); // running
    data.extend(100_000_u32.to_le_bytes()); // 1 km in 1/100 m
    data.extend(session_end.to_le_bytes());

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

    fit_file(&data, 12)
}

/// Build a running activity with consistent session and record times for investigation tests.
pub fn synthetic_running_activity(
    sample_count: usize,
    start_time: u32,
    duration_seconds: u32,
    distance_centimeters: u32,
) -> Vec<u8> {
    let mut data = definition(0, 0, &[(0, 1, 0), (1, 2, 0x84), (2, 2, 0x84), (4, 4, 0x86)]);
    data.push(0);
    data.push(4);
    data.extend(1u16.to_le_bytes());
    data.extend(42u16.to_le_bytes());
    data.extend(start_time.to_le_bytes());

    data.extend(definition(
        1,
        18,
        &[(2, 4, 0x86), (5, 1, 0), (9, 4, 0x86), (253, 4, 0x86)],
    ));
    data.push(1);
    data.extend(start_time.to_le_bytes());
    data.push(1);
    data.extend(distance_centimeters.to_le_bytes());
    data.extend((start_time + duration_seconds).to_le_bytes());

    data.extend(definition(
        2,
        20,
        &[(253, 4, 0x86), (5, 4, 0x86), (6, 2, 0x84), (3, 1, 0x02)],
    ));
    for index in 0..sample_count {
        let denominator = sample_count.saturating_sub(1).max(1) as u64;
        let offset = u64::from(duration_seconds) * index as u64 / denominator;
        let distance = u64::from(distance_centimeters) * index as u64 / denominator;
        data.push(2);
        data.extend((start_time + offset as u32).to_le_bytes());
        data.extend((distance as u32).to_le_bytes());
        data.extend(3_000u16.to_le_bytes());
        data.push(140);
    }
    fit_file(&data, 12)
}

/// Synthetic unknown messages pad the file without adding activity samples.
pub fn synthetic_fit_sized(sample_count: usize, total_bytes: usize) -> Vec<u8> {
    let bytes = synthetic_fit(4, true, sample_count);
    let mut data = bytes[12..bytes.len() - 2].to_vec();
    let mut remaining = total_bytes - bytes.len();
    while remaining > 0 {
        let mut size = 255.min(remaining - 10);
        let tail = remaining - size - 10;
        if tail > 0 && tail < 11 {
            size -= 11 - tail;
        }
        data.extend(definition(3, 65000, &[(0, size as u8, 0x0d)]));
        data.push(3);
        data.extend(vec![0; size]);
        remaining -= size + 10;
    }
    fit_file(&data, 12)
}
