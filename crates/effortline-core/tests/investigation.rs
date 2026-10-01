//! Investigation coverage uses generated FIT activities and encrypted temporary storage.
mod support;

use effortline_core::investigation::{
    investigate_recent_running, HeartRateComparison, RunningInvestigation,
};
use effortline_core::library::{
    ActivityLibrary, LibraryError, LibrarySecret, LibrarySecretProvider, SecretUnavailable,
};
use std::fs;
use tempfile::tempdir;

struct TestSecret;

impl LibrarySecretProvider for TestSecret {
    fn load_secret(&self) -> Result<LibrarySecret, SecretUnavailable> {
        Ok(LibrarySecret::from_bytes([3; 32]))
    }
}

#[test]
fn uses_saved_running_records_as_cited_evidence_and_reports_missing_hr() {
    let directory = tempdir().unwrap();
    let mut library = ActivityLibrary::open(directory.path(), &TestSecret).unwrap();
    let mut source_ids = Vec::new();
    for sample_count in 1..=6 {
        let bytes = support::synthetic_running_activity(
            sample_count,
            support::FIT_TIME + sample_count as u32 * 86_400,
            600 + sample_count as u32 * 30,
            100_000,
        );
        source_ids.push(
            library
                .import_fit_bytes(&bytes)
                .unwrap()
                .identity
                .sha256
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
        );
        if sample_count == 5 {
            assert!(matches!(
                investigate_recent_running(&library).unwrap(),
                RunningInvestigation::InsufficientData {
                    eligible_runs: 5,
                    required_runs: 6,
                    ..
                }
            ));
        }
    }

    let RunningInvestigation::Compared {
        previous_runs,
        recent_runs,
        heart_rate,
        ..
    } = investigate_recent_running(&library).unwrap()
    else {
        panic!("six saved running activities should be compared");
    };
    let cited_ids: Vec<_> = previous_runs
        .iter()
        .chain(&recent_runs)
        .map(|run| run.source_id.clone())
        .collect();
    assert_eq!(cited_ids.len(), 6);
    assert!(source_ids
        .iter()
        .all(|source_id| cited_ids.contains(source_id)));
    assert!(matches!(
        heart_rate,
        HeartRateComparison::InsufficientCoverage {
            minimum_samples_per_run: 10,
            minimum_coverage_percent: 50,
            ..
        }
    ));

    let object = fs::read_dir(directory.path().join("objects"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut encrypted = fs::read(&object).unwrap();
    *encrypted.last_mut().unwrap() ^= 1;
    fs::write(object, encrypted).unwrap();
    assert!(matches!(
        investigate_recent_running(&library),
        Err(LibraryError::CorruptOriginal)
    ));
}
