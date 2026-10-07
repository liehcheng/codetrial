use super::*;
use crate::tasks::test_support::admitted;

async fn session() -> TaskSession {
    let mut session = TaskSession::new(admitted().await, 100);
    session.start(100);
    session.observe_turn(
        "turn-1",
        "I traced the loop and checked what was still open.",
        110,
    );
    session
        .record_check("trace", "covered", "initial", "turn-1")
        .unwrap();
    session
}

fn review() -> Value {
    let dimension = json!({"rating": null, "reason": "No reliable opportunity was captured.", "insufficientReason": "missing_turns", "support": "none", "evidence": []});
    json!({"dimensions": DIMENSIONS.into_iter().map(|name| (name.to_owned(), dimension.clone())).collect::<serde_json::Map<_,_>>(), "gaps": [], "nextActions": ["Trace a boundary input and explain the state."]})
}

#[tokio::test]
async fn learning_review_distinguishes_independent_and_supported_evidence() {
    let mut session = session().await;
    let mut value = review();
    let dimension = &mut value["dimensions"]["reasoningParticipation"];
    dimension["rating"] = json!(3);
    dimension["reason"] = json!("The learner independently traced the input.");
    dimension["insufficientReason"] = Value::Null;
    dimension["evidence"] =
        json!([{"kind": "turn", "id": "turn-1"}, {"kind": "revision", "id": "initial"}]);
    validate(&session, &value, "").unwrap();
    session
        .checks
        .get_mut("trace")
        .unwrap()
        .turn_support
        .insert("turn-1".to_owned(), Support::ConceptualHint);
    assert!(validate(&session, &value, "").is_err());
    value["dimensions"]["reasoningParticipation"]["rating"] = json!(2);
    value["dimensions"]["reasoningParticipation"]["support"] = json!("conceptual_hint");
    validate(&session, &value, "").unwrap();

    value["dimensions"]["reasoningParticipation"]["support"] = json!("targeted_followup");
    assert!(validate(&session, &value, "").is_err());

    // A later hint does not retroactively penalize a different independent
    // turn.
    session.observe_turn(
        "turn-2",
        "I corrected the boundary case independently.",
        120,
    );
    value["dimensions"]["reasoningParticipation"]["rating"] = json!(3);
    value["dimensions"]["reasoningParticipation"]["support"] = json!("none");
    value["dimensions"]["reasoningParticipation"]["evidence"] =
        json!([{"kind": "turn", "id": "turn-2"}]);
    validate(&session, &value, "").unwrap();
}

#[tokio::test]
async fn reviews_refuse_hiring_unsupported_judgments_code_and_dangling_references() {
    let session = session().await;
    for text in [
        "```python\nreturn solution\n```",
        "Hire this learner.",
        "They cheated.",
        "Their accent needs work.",
    ] {
        let mut value = review();
        value["dimensions"]["implementationOwnership"]["reason"] = json!(text);
        assert!(validate(&session, &value, "").is_err(), "accepted {text}");
    }
    let mut value = review();
    value["hireRecommendation"] = json!("yes");
    assert!(validate(&session, &value, "").is_err());
    let mut value = review();
    value["dimensions"]["implementationOwnership"]["evidence"] =
        json!([{"kind": "turn", "id": "absent"}]);
    assert!(validate(&session, &value, "").is_err());
    let mut value = review();
    value["dimensions"]["implementationOwnership"]["insufficientReason"] = Value::Null;
    assert!(validate(&session, &value, "").is_err());
    let mut value = review();
    value["dimensions"]["implementationOwnership"]["rating"] = json!(0);
    value["dimensions"]["implementationOwnership"]["insufficientReason"] = Value::Null;
    value["dimensions"]["implementationOwnership"]["evidence"] =
        json!([{"kind":"turn", "id":"turn-1"}]);
    assert!(validate(&session, &value, "").is_err());
}

#[tokio::test]
async fn sparse_and_overflowed_sessions_have_explicit_nulls_and_server_metadata() {
    let mut session = TaskSession::new(admitted().await, 100);
    let value = review();
    validate(&session, &value, "").unwrap();
    session.capture_overflow = true;
    let stamped = stamp(&session, &value, "session-1");
    assert_eq!(stamped["assessmentMode"], "task");
    assert_eq!(stamped["taskAssessment"]["captureOverflow"], true);
    assert_eq!(stamped["taskAssessment"]["taskId"], "delimiter-closer");
    let reference = "if not stack or stack.pop() != pairs[char]: return False";
    assert!(reference_overlap(reference, reference));
    assert!(!reference_overlap(
        "Explain the state before each step.",
        reference
    ));
    let mut value = value;
    value["nextActions"] = json!([reference]);
    assert!(validate(&session, &value, reference).is_err());
}

#[tokio::test]
async fn server_unavailable_review_matches_the_browser_wire_fixture() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../fixtures/task-review.json")).unwrap();
    assert_eq!(fixture["topic"], crate::runtime::TOPIC_TASK_REVIEW);
    let mut session = session().await;
    session.final_revision_id = Some("initial".to_owned());
    assert_eq!(
        unavailable(&session, "room-1"),
        fixture["cases"][0]["payload"]
    );
}

#[tokio::test]
async fn support_follows_the_cited_turn_not_the_rest_of_its_check() {
    let mut session = session().await;
    session
        .action(
            crate::tasks::session::Action {
                version: 1,
                request_id: "clue".to_owned(),
                action: crate::tasks::session::ActionKind::Hint,
                revision_id: None,
                target_id: None,
            },
            111,
        )
        .unwrap();
    session.observe_turn(
        "turn-2",
        "After the clue I traced the nested case again.",
        112,
    );
    session
        .record_check_with_support("trace", "covered", "initial", "turn-2", Some("hint-1"))
        .unwrap();
    assert_eq!(session.checks["trace"].support, Support::ConceptualHint);
    let mut value = review();
    let dimension = &mut value["dimensions"]["reasoningParticipation"];
    dimension["rating"] = json!(3);
    dimension["reason"] = json!("The learner traced the input before any hint.");
    dimension["insufficientReason"] = Value::Null;
    dimension["evidence"] = json!([{"kind": "turn", "id": "turn-1"}]);
    validate(&session, &value, "").unwrap();
    let dimension = &mut value["dimensions"]["reasoningParticipation"];
    dimension["rating"] = json!(2);
    dimension["support"] = json!("conceptual_hint");
    assert!(validate(&session, &value, "").is_err());
    value["dimensions"]["reasoningParticipation"]["evidence"] =
        json!([{"kind": "turn", "id": "turn-2"}]);
    validate(&session, &value, "").unwrap();
}

#[tokio::test]
async fn unrecognized_turns_support_no_rating_and_the_model_never_reads_them() {
    let mut session = session().await;

    // Greek letters stand for a turn the recognizer returned in another script;
    // the script, not any language, is what the rule tests.
    let garbled =
        "\u{3b1}\u{3b2}\u{3b3}\u{3b4} \u{3b5}\u{3b6}\u{3b7}\u{3b8} \u{3b9}\u{3ba}\u{3bb}\u{3bc}";
    session.observe_turn("turn-2", garbled, 112);
    let mut value = review();
    let dimension = &mut value["dimensions"]["reasoningParticipation"];
    dimension["rating"] = json!(1);
    dimension["reason"] = json!("The learner began to explain.");
    dimension["insufficientReason"] = Value::Null;
    dimension["evidence"] = json!([{"kind": "turn", "id": "turn-2"}]);
    assert!(validate(&session, &value, "").is_err());
    let dimension = &mut value["dimensions"]["reasoningParticipation"];
    dimension["rating"] = Value::Null;
    dimension["insufficientReason"] = json!("garbled_turns");
    validate(&session, &value, "").unwrap();
    let prompt = prompt(&session);
    assert!(!prompt.contains(garbled));
    assert!(prompt.contains(crate::agent::UNRECOGNIZED_TURN));
    assert!(prompt.contains("I traced the loop"));
}

#[tokio::test]
async fn validated_review_matches_the_browser_wire_fixture() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../fixtures/task-review.json")).unwrap();
    let expected = &fixture["cases"][1]["payload"];
    let session = session().await;
    let mut value = review();
    value["dimensions"]["reasoningParticipation"] = json!({"rating": 3,
        "reason": "The learner traced the loop state before any hint.",
        "insufficientReason": null, "support": "none",
        "evidence": [{"kind": "turn", "id": "turn-1"}]});
    value["gaps"] = json!(["No practice run was captured."]);
    value["nextActions"] = json!(["Run the public cases and predict each result first."]);
    validate(&session, &value, "").unwrap();
    assert_eq!(&stamp(&session, &value, "room-1"), expected);
}

#[tokio::test]
async fn ended_attempt_matches_the_browser_wire_fixture() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../fixtures/task-review.json")).unwrap();
    let mut session = session().await;
    session
        .acknowledge_edit("draft", "print('so far')", 150)
        .unwrap();
    let end = serde_json::from_value(json!({"version": 1, "requestId": "hide",
        "outcome": "invalid", "cause": "visibility", "durationMs": 0}))
    .unwrap();
    session.end(end, 220).unwrap();
    assert_eq!(ended(&session, "room-1"), fixture["cases"][2]["payload"]);
}

#[tokio::test]
async fn advice_never_coaches_a_learner_to_be_easier_to_transcribe() {
    let session = session().await;
    for advice in [
        "Speak more clearly to avoid transcription ambiguity.",
        "Explain your answer in English next time.",
        "Make your explanation audible.",
    ] {
        for field in ["gaps", "nextActions"] {
            let mut value = review();
            value[field] = json!([advice]);
            assert!(validate(&session, &value, "").is_err(), "{field}: {advice}");
        }
    }
    // A neutral mention of transcription in a dimension reason stays allowed.
    let mut value = review();
    value["dimensions"]["reasoningParticipation"]["reason"] =
        json!("Evidence was limited by transcription.");
    validate(&session, &value, "").unwrap();
}

#[tokio::test]
async fn an_attempt_ended_under_a_rule_is_a_record_without_ratings() {
    let mut session = session().await;
    session
        .acknowledge_edit("draft", "print('so far')", 150)
        .unwrap();
    let end = serde_json::from_value(json!({"version": 1, "requestId": "e",
        "outcome": "invalid", "cause": "visibility", "durationMs": 0}))
    .unwrap();
    session.end(end, 220).unwrap();
    let result = ended(&session, "room-1");
    let assessment = &result["taskAssessment"];
    assert_eq!(result["assessmentMode"], "task");
    assert_eq!(assessment["outcome"]["outcome"], "invalid");
    assert_eq!(assessment["outcome"]["cause"], "visibility");
    assert_eq!(assessment["outcome"]["at"], 120);
    assert_eq!(assessment["finalRevisionId"], "draft");
    assert_eq!(assessment["finalCode"], "print('so far')");
    assert_eq!(assessment["githubLogin"], "learner");
    assert_eq!(assessment["rulesAcknowledgedAt"], "2026-10-07T00:00:00Z");
    assert_eq!(assessment["calibration"]["metric"], "keypoints");
    for rated in ["dimensions", "gaps", "nextActions", "feedbackUnavailable"] {
        assert!(
            assessment.get(rated).is_none() && result.get(rated).is_none(),
            "{rated}"
        );
    }
}

#[tokio::test]
async fn review_prompt_contains_public_assignment_requirements_only() {
    let session = session().await;
    let text = prompt(&session);
    let line = text
        .lines()
        .find(|line| line.starts_with("PUBLIC TASK REQUIREMENTS"))
        .unwrap();
    let projection: Value = serde_json::from_str(line.split_once(": ").unwrap().1).unwrap();
    let public = session.task.exercise.live_projection();
    for field in [
        "brief",
        "contract",
        "completionTargets",
        "understandingChecks",
    ] {
        assert!(!projection[field].is_null());
        assert_eq!(projection[field], public[field]);
    }
    for field in [
        "reference",
        "referenceCode",
        "interactionPrompt",
        "privateNotes",
        "hints",
        "clarifications",
    ] {
        assert!(projection.get(field).is_none());
    }
}

#[tokio::test]
async fn the_review_reads_turns_in_the_order_they_were_spoken() {
    let mut session = TaskSession::new(admitted().await, 100);
    session.start(100);
    for index in 1..=11 {
        session.observe_turn(&format!("turn-{index}"), &format!("point {index}"), 101);
    }
    let prompt = prompt(&session);
    let second = prompt.find("\"turn-2\"").unwrap();
    let tenth = prompt.find("\"turn-10\"").unwrap();
    assert!(second < tenth, "turn-10 was read before turn-2");
}
