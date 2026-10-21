//! Decision Conversion (descriptor: specs/descriptors/proto_decision_conversion.json).

use std::error::Error;

use polaris_proto::{ProtoError, pb};
use polaris_sdk::error::ParseError;
use polaris_sdk::system_one::{
    Answer, Decision, DecisionInput, Label, LabelScore, Probability, Question, QuestionAnswer,
    QuestionId, QuestionKind,
};
use proptest::collection::vec;
use proptest::prelude::*;
use prost::Message;

type TestResult = Result<(), Box<dyn Error>>;

fn p(value: f32) -> Result<Probability, ParseError> {
    Probability::new(value)
}

fn sample_decision() -> Result<Decision, Box<dyn Error>> {
    Ok(Decision {
        answers: vec![
            QuestionAnswer {
                question: QuestionId::new("intent")?,
                answer: Answer::Choice {
                    selected: Label::new("refactor")?,
                    probs: vec![
                        LabelScore {
                            label: Label::new("refactor")?,
                            probability: p(0.75)?,
                        },
                        LabelScore {
                            label: Label::new("explain")?,
                            probability: p(0.25)?,
                        },
                    ],
                },
            },
            QuestionAnswer {
                question: QuestionId::new("complexity")?,
                answer: Answer::Score {
                    value: -3,
                    confidence: p(1.0)?,
                },
            },
            QuestionAnswer {
                question: QuestionId::new("needs_sandbox")?,
                answer: Answer::Noul { yes: p(0.0)? },
            },
        ],
        latency_us: u64::MAX,
    })
}

fn decision_roundtrip(decision: &Decision) -> Result<Decision, Box<dyn Error>> {
    let wire = pb::Decision::from(decision).encode_to_vec();
    let decoded = pb::Decision::decode(wire.as_slice())?;
    Ok(Decision::try_from(decoded)?)
}

#[test]
fn decision_with_all_answer_kinds_roundtrips() -> TestResult {
    let decision = sample_decision()?;
    assert_eq!(decision_roundtrip(&decision)?, decision);
    let empty = Decision {
        answers: vec![],
        latency_us: 0,
    };
    assert_eq!(decision_roundtrip(&empty)?, empty);
    Ok(())
}

#[test]
fn questions_and_input_roundtrip() -> TestResult {
    let questions = [
        Question {
            id: QuestionId::new("intent")?,
            kind: QuestionKind::Choice {
                options: vec![Label::new("a")?, Label::new("b")?],
            },
        },
        Question {
            id: QuestionId::new("risk")?,
            kind: QuestionKind::Score { min: -5, max: 10 },
        },
        Question {
            id: QuestionId::new("is_code")?,
            kind: QuestionKind::Noul {
                statement: "the text contains source code".to_owned(),
            },
        },
    ];
    for question in questions {
        let wire = pb::Question::from(&question).encode_to_vec();
        let back = Question::try_from(pb::Question::decode(wire.as_slice())?)?;
        assert_eq!(back, question);
    }
    for context in [None, Some(String::new()), Some("surrounding".to_owned())] {
        let input = DecisionInput {
            text: "hello".to_owned(),
            context,
        };
        let wire = pb::DecisionInput::from(&input).encode_to_vec();
        let back = DecisionInput::from(pb::DecisionInput::decode(wire.as_slice())?);
        assert_eq!(back, input);
    }
    Ok(())
}

fn noul(yes: f32) -> pb::Decision {
    pb::Decision {
        answers: vec![pb::QuestionAnswer {
            question: "q".to_owned(),
            answer: Some(pb::Answer {
                kind: Some(pb::answer::Kind::Noul(pb::answer::Noul { yes })),
            }),
        }],
        latency_us: 1,
    }
}

#[test]
fn invalid_probability_is_error() {
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.1, 1.5, 1e30] {
        assert_eq!(
            Decision::try_from(noul(bad)),
            Err(ProtoError::Invalid(ParseError::InvalidProbability)),
            "{bad}"
        );
    }
    assert!(Decision::try_from(noul(0.5)).is_ok());
    let choice = pb::Decision {
        answers: vec![pb::QuestionAnswer {
            question: "q".to_owned(),
            answer: Some(pb::Answer {
                kind: Some(pb::answer::Kind::Choice(pb::answer::Choice {
                    selected: "a".to_owned(),
                    probs: vec![pb::LabelScore {
                        label: "a".to_owned(),
                        probability: f32::NAN,
                    }],
                })),
            }),
        }],
        latency_us: 0,
    };
    assert_eq!(
        Decision::try_from(choice),
        Err(ProtoError::Invalid(ParseError::InvalidProbability))
    );
}

#[test]
fn invalid_names_are_errors() {
    let long = "x".repeat(QuestionId::MAX_LEN + 1);
    for bad in ["", "   ", "tab\there", long.as_str()] {
        let mut wire = noul(0.5);
        if let Some(first) = wire.answers.first_mut() {
            first.question = bad.to_owned();
        }
        assert!(
            matches!(
                Decision::try_from(wire),
                Err(ProtoError::Invalid(ParseError::InvalidValue { .. }))
            ),
            "{bad:?}"
        );
    }
    let bad_label = pb::Decision {
        answers: vec![pb::QuestionAnswer {
            question: "q".to_owned(),
            answer: Some(pb::Answer {
                kind: Some(pb::answer::Kind::Choice(pb::answer::Choice {
                    selected: String::new(),
                    probs: vec![],
                })),
            }),
        }],
        latency_us: 0,
    };
    assert!(matches!(
        Decision::try_from(bad_label),
        Err(ProtoError::Invalid(ParseError::InvalidValue { .. }))
    ));
}

#[test]
fn missing_answer_is_error() {
    let no_answer = pb::Decision {
        answers: vec![pb::QuestionAnswer {
            question: "q".to_owned(),
            answer: None,
        }],
        latency_us: 0,
    };
    assert_eq!(
        Decision::try_from(no_answer),
        Err(ProtoError::MissingField("question_answer.answer"))
    );
    let no_kind = pb::Decision {
        answers: vec![pb::QuestionAnswer {
            question: "q".to_owned(),
            answer: Some(pb::Answer { kind: None }),
        }],
        latency_us: 0,
    };
    assert_eq!(
        Decision::try_from(no_kind),
        Err(ProtoError::MissingField("answer.kind"))
    );
}

#[test]
fn conversion_never_panics_on_default_message() {
    assert_eq!(
        Decision::try_from(pb::Decision::default()),
        Ok(Decision {
            answers: vec![],
            latency_us: 0
        })
    );
    assert!(Question::try_from(pb::Question::default()).is_err());
    let answer = pb::QuestionAnswer::default();
    assert!(
        Decision::try_from(pb::Decision {
            answers: vec![answer],
            latency_us: 0
        })
        .is_err()
    );
}

fn arb_name() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_]{0,15}"
}

fn arb_label() -> impl Strategy<Value = Label> {
    arb_name().prop_filter_map("valid label", |s| Label::new(&s).ok())
}

fn arb_question_id() -> impl Strategy<Value = QuestionId> {
    arb_name().prop_filter_map("valid id", |s| QuestionId::new(&s).ok())
}

fn arb_prob() -> impl Strategy<Value = Probability> {
    (0u16..=1000).prop_filter_map("in range", |n| Probability::new(f32::from(n) / 1000.0).ok())
}

fn arb_answer() -> impl Strategy<Value = Answer> {
    prop_oneof![
        (
            arb_label(),
            vec(
                (arb_label(), arb_prob())
                    .prop_map(|(label, probability)| LabelScore { label, probability }),
                0..5
            )
        )
            .prop_map(|(selected, probs)| Answer::Choice { selected, probs }),
        (any::<i32>(), arb_prob())
            .prop_map(|(value, confidence)| Answer::Score { value, confidence }),
        arb_prob().prop_map(|yes| Answer::Noul { yes }),
    ]
}

fn arb_decision() -> impl Strategy<Value = Decision> {
    (
        vec(
            (arb_question_id(), arb_answer())
                .prop_map(|(question, answer)| QuestionAnswer { question, answer }),
            0..6,
        ),
        any::<u64>(),
    )
        .prop_map(|(answers, latency_us)| Decision {
            answers,
            latency_us,
        })
}

fn arb_question() -> impl Strategy<Value = Question> {
    (
        arb_question_id(),
        prop_oneof![
            vec(arb_label(), 0..5).prop_map(|options| QuestionKind::Choice { options }),
            (any::<i32>(), any::<i32>()).prop_map(|(min, max)| QuestionKind::Score { min, max }),
            ".{0,40}".prop_map(|statement| QuestionKind::Noul { statement }),
        ],
    )
        .prop_map(|(id, kind)| Question { id, kind })
}

proptest! {
    #[test]
    fn prop_decision_roundtrip(decision in arb_decision()) {
        let back = decision_roundtrip(&decision).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(back, decision);
    }

    #[test]
    fn prop_question_roundtrip(question in arb_question()) {
        let wire = pb::Question::from(&question).encode_to_vec();
        let decoded = pb::Question::decode(wire.as_slice())
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
        let back = Question::try_from(decoded).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(back, question);
    }

    #[test]
    fn prop_garbage_bytes_never_panic(
        raw in vec(any::<u8>(), 0..512),
        decision in arb_decision(),
        flips in vec((any::<usize>(), any::<u8>()), 0..4),
    ) {
        if let Ok(msg) = pb::Decision::decode(raw.as_slice()) {
            if let Ok(parsed) = Decision::try_from(msg) {
                let back = decision_roundtrip(&parsed)
                    .map_err(|e| TestCaseError::fail(e.to_string()))?;
                prop_assert_eq!(back, parsed);
            }
        }
        if let Ok(msg) = pb::Question::decode(raw.as_slice()) {
            let _ = Question::try_from(msg);
        }
        let mut bytes = pb::Decision::from(&decision).encode_to_vec();
        for (pos, value) in flips {
            let len = bytes.len();
            if let Some(slot) = bytes.get_mut(pos % len.max(1)) {
                *slot = value;
            }
        }
        if let Ok(msg) = pb::Decision::decode(bytes.as_slice()) {
            if let Ok(parsed) = Decision::try_from(msg) {
                let back = decision_roundtrip(&parsed)
                    .map_err(|e| TestCaseError::fail(e.to_string()))?;
                prop_assert_eq!(back, parsed);
            }
        }
    }
}
