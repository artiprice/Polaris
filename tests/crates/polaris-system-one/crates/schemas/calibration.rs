use polaris_sdk::ids::Cid;
use polaris_system_one_schemas::calibration::*;
type TestResult = Result<(), Box<dyn std::error::Error>>;
fn dataset() -> Result<CalibrationDataset, CalibrationError> {
    CalibrationDataset::join(
        "safety.command",
        1,
        Cid::from_bytes([1; 32]),
        Cid::from_bytes([2; 32]),
        &[
            CalibrationLabel {
                id: "a".into(),
                positive: true,
            },
            CalibrationLabel {
                id: "b".into(),
                positive: false,
            },
            CalibrationLabel {
                id: "c".into(),
                positive: true,
            },
            CalibrationLabel {
                id: "d".into(),
                positive: false,
            },
        ],
        &[
            CalibrationPrediction {
                id: "d".into(),
                positive_ppm: 100_000,
            },
            CalibrationPrediction {
                id: "c".into(),
                positive_ppm: 700_000,
            },
            CalibrationPrediction {
                id: "a".into(),
                positive_ppm: 900_000,
            },
            CalibrationPrediction {
                id: "b".into(),
                positive_ppm: 800_000,
            },
        ],
    )
}
#[test]
fn calibration_joins_exact_ids_and_computes_confusion() -> TestResult {
    let data = dataset()?;
    let report = data.evaluate(750_000)?;
    assert_eq!(report.true_positive, 1);
    assert_eq!(report.false_positive, 1);
    assert_eq!(report.true_negative, 1);
    assert_eq!(report.false_negative, 1);
    assert_eq!(
        report.precision,
        Some(ExactRatio {
            numerator: 1,
            denominator: 2
        })
    );
    assert_eq!(
        report.recall,
        Some(ExactRatio {
            numerator: 1,
            denominator: 2
        })
    );
    assert_eq!(
        report.brier,
        ExactRatio {
            numerator: 750_000_000_000,
            denominator: 4_000_000_000_000
        }
    );
    assert_eq!(
        report.ece,
        ExactRatio {
            numerator: 1_300_000,
            denominator: 4_000_000
        }
    );
    assert_eq!(report.fixture, Cid::from_bytes([1; 32]));
    assert_eq!(report.checkpoint, Cid::from_bytes([2; 32]));
    Ok(())
}
#[test]
fn calibration_rejects_missing_duplicate_and_foreign_outputs() -> TestResult {
    let labels = [CalibrationLabel {
        id: "a".into(),
        positive: true,
    }];
    for outputs in [
        vec![],
        vec![CalibrationPrediction {
            id: "b".into(),
            positive_ppm: 10,
        }],
        vec![
            CalibrationPrediction {
                id: "a".into(),
                positive_ppm: 1,
            },
            CalibrationPrediction {
                id: "a".into(),
                positive_ppm: 2,
            },
        ],
    ] {
        assert!(
            CalibrationDataset::join(
                "safety.command",
                1,
                Cid::from_bytes([1; 32]),
                Cid::from_bytes([2; 32]),
                &labels,
                &outputs
            )
            .is_err()
        );
    }
    Ok(())
}
#[test]
fn calibration_threshold_selection_respects_precision_and_support() -> TestResult {
    let data = dataset()?;
    let selected = data
        .select_threshold(PrecisionRequirement {
            minimum_ppm: 1_000_000,
            minimum_predicted_positive: 1,
        })?
        .ok_or("no threshold")?;
    assert_eq!(selected.threshold_ppm, 900_000);
    assert_eq!(selected.false_positive, 0);
    assert!(
        data.select_threshold(PrecisionRequirement {
            minimum_ppm: 1_000_000,
            minimum_predicted_positive: 2
        })?
        .is_none()
    );
    assert_eq!(
        data.evaluate(1_000_000)?.precision,
        None,
        "no predicted positive has undefined precision"
    );
    Ok(())
}
#[test]
fn calibration_is_permutation_invariant_and_bounds_inputs() -> TestResult {
    let labels = [
        CalibrationLabel {
            id: "a".into(),
            positive: true,
        },
        CalibrationLabel {
            id: "b".into(),
            positive: false,
        },
    ];
    let outputs = [
        CalibrationPrediction {
            id: "a".into(),
            positive_ppm: 1_000_000,
        },
        CalibrationPrediction {
            id: "b".into(),
            positive_ppm: 0,
        },
    ];
    let join = |l: &[CalibrationLabel], o: &[CalibrationPrediction]| {
        CalibrationDataset::join(
            "route.tool",
            1,
            Cid::from_bytes([1; 32]),
            Cid::from_bytes([2; 32]),
            l,
            o,
        )
    };
    let first = join(&labels, &outputs)?;
    let mut labels = labels;
    labels.reverse();
    let mut outputs = outputs;
    outputs.reverse();
    assert_eq!(
        first.evaluate(500_000)?,
        join(&labels, &outputs)?.evaluate(500_000)?
    );
    assert!(first.evaluate(1_000_001).is_err());
    assert!(
        join(
            &labels,
            &[
                CalibrationPrediction {
                    id: "a".into(),
                    positive_ppm: 1_000_001
                },
                outputs.first().ok_or("missing output")?.clone()
            ]
        )
        .is_err()
    );
    let large = vec![
        CalibrationLabel {
            id: "a".into(),
            positive: true
        };
        MAX_CALIBRATION_SAMPLES.saturating_add(1)
    ];
    assert!(join(&large, &[]).is_err());
    Ok(())
}
