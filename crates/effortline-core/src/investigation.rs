//! Deterministic, cited answers for the first chat-led investigation.

use crate::fit_import::{ImportedActivity, Sport};
use crate::library::{ActivityLibrary, LibraryError};

pub const RUNS_PER_PERIOD: usize = 3;
const HEART_RATE_MIN_SAMPLES: usize = 10;

#[derive(Debug, Clone, PartialEq)]
pub struct ActivityEvidence {
    /// Stable reference to the exact imported source bytes.
    pub source_id: String,
    pub started_at_unix_ms: i64,
    pub duration_seconds: u64,
    pub distance_m: f64,
    pub pace_seconds_per_km: f64,
    pub sample_count: usize,
    pub heart_rate_sample_count: usize,
    pub median_heart_rate_bpm: Option<f64>,
    /// Numeric manufacturer/product pair reported by the FIT file, when present.
    pub device: Option<(u16, u16)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PaceComparison {
    pub previous_median_seconds_per_km: f64,
    pub recent_median_seconds_per_km: f64,
    /// Negative means the recent median pace is faster.
    pub change_percent: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HeartRateComparison {
    Available {
        previous_median_bpm: f64,
        recent_median_bpm: f64,
    },
    InsufficientCoverage {
        previous_qualified_runs: usize,
        recent_qualified_runs: usize,
        required_runs_per_period: usize,
        minimum_samples_per_run: usize,
        minimum_coverage_percent: u8,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum RunningInvestigation {
    Compared {
        previous_runs: Vec<ActivityEvidence>,
        recent_runs: Vec<ActivityEvidence>,
        pace: PaceComparison,
        heart_rate: HeartRateComparison,
        device_history: DeviceHistory,
    },
    InsufficientData {
        eligible_runs: usize,
        required_runs: usize,
        reason: InsufficientDataReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceHistory {
    Consistent,
    Mixed,
    Missing,
    MixedOrMissing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsufficientDataReason {
    TooFewRuns,
    AmbiguousPeriodBoundary,
}

/// Compare the three newest eligible runs with the three runs before them.
/// Eligibility requires a running sport, positive duration, and recorded distance.
pub fn investigate_recent_running(
    library: &ActivityLibrary,
) -> Result<RunningInvestigation, LibraryError> {
    let activities = library.recent_running_activities()?;
    Ok(compare_activities(&activities))
}

fn compare_activities(activities: &[ImportedActivity]) -> RunningInvestigation {
    let eligible: Vec<_> = activities
        .iter()
        .filter(|activity| {
            activity.data.sport == Sport::Running
                && activity
                    .data
                    .total_distance_m
                    .is_some_and(|distance| distance > 0.0)
                && activity.data.end_unix_ms > activity.data.start_unix_ms
        })
        .take(RUNS_PER_PERIOD * 2)
        .collect();
    let required_runs = RUNS_PER_PERIOD * 2;
    if eligible.len() < required_runs {
        return RunningInvestigation::InsufficientData {
            eligible_runs: eligible.len(),
            required_runs,
            reason: InsufficientDataReason::TooFewRuns,
        };
    }

    // Do not use a hash tie-break to invent which side of the comparison a run belongs to.
    if eligible[RUNS_PER_PERIOD - 1].data.start_unix_ms
        == eligible[RUNS_PER_PERIOD].data.start_unix_ms
    {
        return RunningInvestigation::InsufficientData {
            eligible_runs: eligible.len(),
            required_runs,
            reason: InsufficientDataReason::AmbiguousPeriodBoundary,
        };
    }

    // The library query returns newest first. Keep the two periods distinct and present
    // evidence oldest first so it reads naturally in the chat transcript.
    let recent = &eligible[..RUNS_PER_PERIOD];
    let previous = &eligible[RUNS_PER_PERIOD..required_runs];
    let recent_evidence: Vec<_> = recent
        .iter()
        .rev()
        .map(|activity| evidence(activity))
        .collect();
    let previous_evidence: Vec<_> = previous
        .iter()
        .rev()
        .map(|activity| evidence(activity))
        .collect();
    let previous_pace = median(
        previous_evidence
            .iter()
            .map(|run| run.pace_seconds_per_km)
            .collect(),
    );
    let recent_pace = median(
        recent_evidence
            .iter()
            .map(|run| run.pace_seconds_per_km)
            .collect(),
    );
    let qualified_previous: Vec<_> = previous_evidence
        .iter()
        .filter(|run| qualifies_for_heart_rate(run))
        .collect();
    let qualified_recent: Vec<_> = recent_evidence
        .iter()
        .filter(|run| qualifies_for_heart_rate(run))
        .collect();
    let heart_rate = if qualified_previous.len() == RUNS_PER_PERIOD
        && qualified_recent.len() == RUNS_PER_PERIOD
    {
        HeartRateComparison::Available {
            previous_median_bpm: median(
                qualified_previous
                    .iter()
                    .filter_map(|run| run.median_heart_rate_bpm)
                    .collect(),
            ),
            recent_median_bpm: median(
                qualified_recent
                    .iter()
                    .filter_map(|run| run.median_heart_rate_bpm)
                    .collect(),
            ),
        }
    } else {
        HeartRateComparison::InsufficientCoverage {
            previous_qualified_runs: qualified_previous.len(),
            recent_qualified_runs: qualified_recent.len(),
            required_runs_per_period: RUNS_PER_PERIOD,
            minimum_samples_per_run: HEART_RATE_MIN_SAMPLES,
            minimum_coverage_percent: 50,
        }
    };
    let devices: Vec<_> = previous_evidence
        .iter()
        .chain(&recent_evidence)
        .filter_map(|run| run.device)
        .collect();
    let has_missing_device = devices.len() != required_runs;
    let has_mixed_devices = devices
        .first()
        .is_some_and(|first| devices.iter().any(|device| device != first));
    let device_history = match (has_mixed_devices, has_missing_device) {
        (false, false) => DeviceHistory::Consistent,
        (true, false) => DeviceHistory::Mixed,
        (false, true) => DeviceHistory::Missing,
        (true, true) => DeviceHistory::MixedOrMissing,
    };

    RunningInvestigation::Compared {
        previous_runs: previous_evidence,
        recent_runs: recent_evidence,
        pace: PaceComparison {
            previous_median_seconds_per_km: previous_pace,
            recent_median_seconds_per_km: recent_pace,
            change_percent: (recent_pace - previous_pace) / previous_pace * 100.0,
        },
        heart_rate,
        device_history,
    }
}

fn evidence(activity: &ImportedActivity) -> ActivityEvidence {
    let distance_m = activity.data.total_distance_m.unwrap_or_default();
    let duration_ms = activity
        .data
        .end_unix_ms
        .saturating_sub(activity.data.start_unix_ms);
    let duration_seconds = (duration_ms / 1000) as u64;
    let heart_rates: Vec<_> = activity
        .data
        .samples
        .iter()
        .filter_map(|sample| sample.heart_rate_bpm.map(f64::from))
        .collect();
    ActivityEvidence {
        source_id: activity
            .source
            .identity
            .sha256
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        started_at_unix_ms: activity.data.start_unix_ms,
        duration_seconds,
        distance_m,
        pace_seconds_per_km: duration_ms as f64 / 1000.0 / (distance_m / 1000.0),
        sample_count: activity.data.samples.len(),
        heart_rate_sample_count: heart_rates.len(),
        median_heart_rate_bpm: (!heart_rates.is_empty()).then(|| median(heart_rates)),
        device: activity
            .source
            .provenance
            .manufacturer_id
            .zip(activity.source.provenance.product_id),
    }
}

fn qualifies_for_heart_rate(run: &ActivityEvidence) -> bool {
    run.heart_rate_sample_count >= HEART_RATE_MIN_SAMPLES
        && run.heart_rate_sample_count.saturating_mul(2) >= run.sample_count
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[middle - 1] + values[middle]) / 2.0
    } else {
        values[middle]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fit_import::{
        ActivityData, ActivitySample, ActivitySource, FitProvenance, SourceIdentity,
    };

    fn run(
        index: u8,
        start: i64,
        pace_seconds_per_km: f64,
        with_heart_rate: bool,
    ) -> ImportedActivity {
        let distance_m = 5_000.0;
        let duration_ms = (pace_seconds_per_km * 5.0 * 1000.0) as i64;
        let samples = (0..20)
            .map(|sample| ActivitySample {
                timestamp_unix_ms: start + sample * 1000,
                distance_m: None,
                speed_m_s: None,
                heart_rate_bpm: (with_heart_rate && sample < 12).then_some(140 + index),
            })
            .collect();
        ImportedActivity {
            source: ActivitySource {
                identity: SourceIdentity {
                    sha256: [index; 32],
                },
                provenance: FitProvenance::default(),
            },
            data: ActivityData {
                sport: Sport::Running,
                start_unix_ms: start,
                end_unix_ms: start + duration_ms,
                total_distance_m: Some(distance_m),
                samples,
            },
        }
    }

    #[test]
    fn requires_six_eligible_runs_and_reports_the_count() {
        let mut runs = (0..6)
            .map(|index| run(index, i64::from(index) * 1000, 300.0, true))
            .collect::<Vec<_>>();
        runs[2].data.sport = Sport::Other;
        assert_eq!(
            compare_activities(&runs),
            RunningInvestigation::InsufficientData {
                eligible_runs: 5,
                required_runs: 6,
                reason: InsufficientDataReason::TooFewRuns,
            }
        );
    }

    #[test]
    fn refuses_to_split_activities_with_the_same_start_time_across_periods() {
        let runs = (0..6)
            .map(|index| run(index, 1_700_000_000_000, 300.0, true))
            .collect::<Vec<_>>();
        assert_eq!(
            compare_activities(&runs),
            RunningInvestigation::InsufficientData {
                eligible_runs: 6,
                required_runs: 6,
                reason: InsufficientDataReason::AmbiguousPeriodBoundary,
            }
        );
    }

    #[test]
    fn splits_recent_and_previous_periods_and_returns_resolving_citations() {
        let runs = (0..6)
            .map(|index| {
                // Input mirrors the library's newest-first query order.
                let pace = if index < 3 { 280.0 } else { 300.0 };
                run(
                    index,
                    1_700_000_000_000 + (5 - i64::from(index)) * 86_400_000,
                    pace,
                    true,
                )
            })
            .collect::<Vec<_>>();
        let result = compare_activities(&runs);
        let RunningInvestigation::Compared {
            previous_runs,
            recent_runs,
            pace,
            heart_rate,
            device_history,
        } = result
        else {
            panic!("six eligible runs should be compared");
        };
        assert_eq!(previous_runs.len(), 3);
        assert_eq!(recent_runs.len(), 3);
        assert_eq!(previous_runs[0].source_id, format!("{:02x}", 5).repeat(32));
        assert_eq!(recent_runs[0].source_id, format!("{:02x}", 2).repeat(32));
        assert_eq!(device_history, DeviceHistory::Missing);
        assert_eq!(pace.previous_median_seconds_per_km, 300.0);
        assert_eq!(pace.recent_median_seconds_per_km, 280.0);
        assert!((pace.change_percent - (-6.6666666667)).abs() < 0.0001);
        assert_eq!(
            heart_rate,
            HeartRateComparison::Available {
                previous_median_bpm: 144.0,
                recent_median_bpm: 141.0,
            }
        );
    }

    #[test]
    fn missing_heart_rate_is_reported_without_blocking_pace_comparison() {
        let runs = (0..6)
            .map(|index| {
                run(
                    index,
                    1_700_000_000_000 + (5 - i64::from(index)) * 86_400_000,
                    300.0,
                    index != 5,
                )
            })
            .collect::<Vec<_>>();
        let RunningInvestigation::Compared {
            heart_rate,
            pace,
            previous_runs,
            ..
        } = compare_activities(&runs)
        else {
            panic!("six eligible runs should be compared");
        };
        assert_eq!(pace.previous_median_seconds_per_km, 300.0);
        assert_eq!(previous_runs[0].heart_rate_sample_count, 0);
        assert_eq!(
            heart_rate,
            HeartRateComparison::InsufficientCoverage {
                previous_qualified_runs: 2,
                recent_qualified_runs: 3,
                required_runs_per_period: 3,
                minimum_samples_per_run: 10,
                minimum_coverage_percent: 50,
            }
        );
    }

    #[test]
    fn mixed_device_history_is_reported_as_a_limit() {
        let mut runs = (0..6)
            .map(|index| {
                run(
                    index,
                    1_700_000_000_000 + (5 - i64::from(index)) * 86_400_000,
                    300.0,
                    true,
                )
            })
            .collect::<Vec<_>>();
        for (index, activity) in runs.iter_mut().enumerate() {
            activity.source.provenance.manufacturer_id = Some(1);
            activity.source.provenance.product_id = Some(if index < 3 { 10 } else { 11 });
        }
        let RunningInvestigation::Compared { device_history, .. } = compare_activities(&runs)
        else {
            panic!("six eligible runs should be compared");
        };
        assert_eq!(device_history, DeviceHistory::Mixed);
    }
}
