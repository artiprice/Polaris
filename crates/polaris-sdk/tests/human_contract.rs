//! Human Request Contract (descriptor: specs/descriptors/human_request_backend.json).

use std::error::Error;

use polaris_sdk::{
    human::{
        HumanAnswerPayload, HumanOption, HumanOptionId, HumanQuestion, HumanQuestionAnswer,
        HumanQuestionId, HumanQuestionKind, HumanRequestId, HumanRequestPayload, HumanResponse,
        HumanSelection,
    },
    ids::{Cid, FrameId, SessionId},
};

type TestResult = Result<(), Box<dyn Error>>;

fn option(id: &str, label: &str) -> Result<HumanOption, polaris_sdk::error::ParseError> {
    Ok(HumanOption {
        id: HumanOptionId::new(id)?,
        label: label.to_owned(),
    })
}

fn request() -> Result<HumanRequestPayload, Box<dyn Error>> {
    Ok(HumanRequestPayload::new(
        HumanRequestId::from_u128(1),
        SessionId::from_u128(2),
        FrameId::from_u128(3),
        4,
        vec![
            HumanQuestion {
                id: HumanQuestionId::new("choice")?,
                prompt: "Choose a route".to_owned(),
                required: true,
                kind: HumanQuestionKind::MultiChoice {
                    options: vec![option("safe", "Safe")?, option("fast", "Fast")?],
                },
            },
            HumanQuestion {
                id: HumanQuestionId::new("context")?,
                prompt: "Add context".to_owned(),
                required: false,
                kind: HumanQuestionKind::FreeText,
            },
        ],
    )?)
}

#[test]
fn optional_questions_require_an_explicit_unanswered_record() -> TestResult {
    let request = request()?;
    let answer = HumanAnswerPayload::new(
        &request,
        Cid::from_bytes([7; 32]),
        vec![
            HumanQuestionAnswer {
                question_id: HumanQuestionId::new("choice")?,
                response: HumanResponse::Multiple {
                    selections: vec![HumanSelection::Option(HumanOptionId::new("safe")?)],
                },
                notes: None,
            },
            HumanQuestionAnswer {
                question_id: HumanQuestionId::new("context")?,
                response: HumanResponse::Unanswered,
                notes: None,
            },
        ],
    )?;
    assert_eq!(answer.answers.len(), request.questions.len());
    Ok(())
}

#[test]
fn required_questions_have_no_default_and_reject_explicit_unanswered() -> TestResult {
    let request = request()?;
    let result = HumanAnswerPayload::new(
        &request,
        Cid::from_bytes([7; 32]),
        vec![
            HumanQuestionAnswer {
                question_id: HumanQuestionId::new("choice")?,
                response: HumanResponse::Unanswered,
                notes: None,
            },
            HumanQuestionAnswer {
                question_id: HumanQuestionId::new("context")?,
                response: HumanResponse::Unanswered,
                notes: None,
            },
        ],
    );
    assert!(result.is_err());
    Ok(())
}

#[test]
fn none_of_the_above_is_exclusive_in_multichoice() -> TestResult {
    let request = request()?;
    let result = HumanAnswerPayload::new(
        &request,
        Cid::from_bytes([7; 32]),
        vec![
            HumanQuestionAnswer {
                question_id: HumanQuestionId::new("choice")?,
                response: HumanResponse::Multiple {
                    selections: vec![
                        HumanSelection::NoneOfTheAbove,
                        HumanSelection::Option(HumanOptionId::new("safe")?),
                    ],
                },
                notes: None,
            },
            HumanQuestionAnswer {
                question_id: HumanQuestionId::new("context")?,
                response: HumanResponse::Unanswered,
                notes: None,
            },
        ],
    );
    assert!(result.is_err());
    Ok(())
}

#[test]
fn rejects_stale_revision_unknown_ids_and_wrong_answer_kinds() -> TestResult {
    let request = request()?;
    let correct = Cid::from_bytes([7; 32]);
    let stale = HumanAnswerPayload::new(&request, Cid::from_bytes([8; 32]), vec![]);
    assert!(stale.is_err());
    let unknown = HumanAnswerPayload::new(
        &request,
        correct,
        vec![HumanQuestionAnswer {
            question_id: HumanQuestionId::new("missing")?,
            response: HumanResponse::Unanswered,
            notes: None,
        }],
    );
    assert!(unknown.is_err());
    let wrong_kind = HumanAnswerPayload::new(
        &request,
        correct,
        vec![
            HumanQuestionAnswer {
                question_id: HumanQuestionId::new("choice")?,
                response: HumanResponse::FreeText {
                    text: "safe".to_owned(),
                },
                notes: None,
            },
            HumanQuestionAnswer {
                question_id: HumanQuestionId::new("context")?,
                response: HumanResponse::Unanswered,
                notes: None,
            },
        ],
    );
    assert!(wrong_kind.is_err());
    Ok(())
}

#[test]
fn validates_choice_count_and_aggregate_request_bytes() -> TestResult {
    let mut questions = Vec::new();
    for index in 0..9 {
        questions.push(HumanQuestion {
            id: HumanQuestionId::new(&format!("q{index}"))?,
            prompt: "Bounded prompt".to_owned(),
            required: false,
            kind: HumanQuestionKind::SingleChoice {
                options: vec![option("yes", "Yes")?],
            },
        });
    }
    let result = HumanRequestPayload::new(
        HumanRequestId::from_u128(9),
        SessionId::from_u128(2),
        FrameId::from_u128(3),
        1,
        questions,
    );
    assert!(result.is_err());
    Ok(())
}
