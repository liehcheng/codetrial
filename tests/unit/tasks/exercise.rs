use super::*;
use crate::tasks::session::Phase;

fn fixture(name: &str) -> Value {
    serde_json::from_str(
        &std::fs::read_to_string(format!("tests/fixtures/task-mode/{name}.json")).unwrap(),
    )
    .unwrap()
}

fn record(fixture: &Value) -> Result<TaskRecord, TaskError> {
    TaskRecord::from_bank(
        &fixture["problem"],
        &fixture["judge"],
        &fixture["variant"],
        fixture.get("sidecar"),
    )
}

#[test]
fn sessions_use_their_own_exercise_and_release_it() {
    let one = Arc::new(record(&fixture("customized")).unwrap());
    let two = Arc::new(record(&fixture("uncustomized")).unwrap());
    let weak_one = Arc::downgrade(&one);
    let weak_two = Arc::downgrade(&two);
    let first = Exercise(one);
    let second = Exercise(two);
    assert_eq!(first.id(), "delimiter-closer");
    assert_eq!(second.id(), "two-sum");
    assert!(
        first
            .starter("python")
            .unwrap()
            .contains("delimitersNestCleanly")
    );
    assert!(
        second
            .starter("python")
            .unwrap()
            .contains("matchDisputedCharge")
    );
    assert!(
        first
            .task_prompt(Phase::Work, Some("closer-branch"), "draft")
            .unwrap()
            .contains("complete the marked branch")
    );
    assert!(
        !second
            .task_prompt(Phase::Work, None, "draft")
            .unwrap()
            .contains("complete the marked branch")
    );
    assert_eq!(first.hints().len(), 3);
    assert_ne!(first.hints(), second.hints());
    assert!(
        first
            .starter("python")
            .unwrap()
            .contains("TASK_COMPLETE_CLOSER")
    );

    // Hint rungs are counted by the attempt's session (tests/unit/tasks/
    // session.rs); what matters here is that each exercise is its own and
    // nothing keeps it alive once its sessions end.
    drop(first);
    drop(second);
    assert!(weak_one.upgrade().is_none());
    assert!(weak_two.upgrade().is_none());
}

#[test]
fn rust_sidecars_refuse_python_contract_errors() {
    let base = fixture("customized");
    let mut mutations = Vec::new();
    let mut bad = base.clone();
    bad["sidecar"]["unknown"] = json!(true);
    mutations.push(bad);
    let mut bad = base.clone();
    bad["sidecar"]["promptVersion"] = json!(2);
    mutations.push(bad);
    let mut bad = base.clone();
    bad["sidecar"]["promptVersion"] = json!(true);
    mutations.push(bad);
    let mut bad = base.clone();
    bad["sidecar"]["promptVersion"] = json!(1.0);
    mutations.push(bad);
    let mut bad = base.clone();
    bad["sidecar"]["interactionPrompt"] = json!("{{optimal}}");
    mutations.push(bad);
    let mut bad = base.clone();
    bad["sidecar"]["interactionPrompt"] = json!("{{title");
    mutations.push(bad);
    let mut bad = base.clone();
    bad["sidecar"]["interactionPrompt"] = json!("x".repeat(4097));
    mutations.push(bad);
    let mut bad = base.clone();
    bad["sidecar"]["maxHintRungs"] = json!(4);
    mutations.push(bad);
    let mut bad = base.clone();
    bad["sidecar"]["requiredCases"] = json!(["nested"]);
    mutations.push(bad);
    let mut bad = base.clone();
    bad["sidecar"]["requiredCases"] = json!(["nested: s=\"{[]}\"", "nested: s=\"{[]}\""]);
    mutations.push(bad);
    let mut bad = base.clone();
    bad["problem"]["starterCode"]["python"] =
        json!("# TASK_COMPLETE_CLOSER # TASK_COMPLETE_CLOSER");
    mutations.push(bad);
    let mut bad = base.clone();
    bad["problem"]["starterCode"]["python"] = json!("# TASK_COMPLETE_CLOSER_SUFFIX");
    mutations.push(bad);
    let mut bad = base.clone();
    bad["sidecar"] = Value::Null;
    mutations.push(bad);
    let mut bad = base.clone();
    let case = bad["judge"]["cases"][4].clone();
    bad["judge"]["cases"].as_array_mut().unwrap().push(case);
    mutations.push(bad);
    for (index, bad) in mutations.iter().enumerate() {
        assert!(record(bad).is_err(), "mutation {index} accepted");
    }
    assert_eq!(record(&base).unwrap().required_case_indices(), &[4, 2, 5]);
}

#[test]
fn task_prompt_recovers_custom_instructions_and_substitutes_once() {
    let mut data = fixture("customized");
    data["sidecar"]["interactionPrompt"] = json!("{ {{title}} { {{language}} {{target}} {{phase}}");
    let exercise = Exercise(Arc::new(record(&data).unwrap()));
    let initial = exercise
        .task_prompt(Phase::Work, Some("closer-branch"), "draft")
        .unwrap();
    let recovery = exercise
        .task_prompt(Phase::WrapUp, Some("closer-branch"), "revision")
        .unwrap();
    assert!(initial.contains("Template Delimiter Audit"));
    assert!(recovery.contains("Complete the branch without changing the surrounding contract."));
    assert!(recovery.contains("wrap-up"));
    assert!(
        exercise
            .task_prompt(Phase::Work, Some("missing"), "")
            .is_err()
    );
    assert!(interpolate("{{{title}}{", &BTreeMap::from([("title", "sample")])).is_ok());
    assert!(interpolate("}} {{title}}", &BTreeMap::from([("title", "sample")])).is_err());
}

#[test]
fn task_bank_prompts_have_only_allowlisted_material_and_match_golden() {
    let problems: Value =
        serde_json::from_str(include_str!("../../../problem-bank/problems.json")).unwrap();
    let variants: Value =
        serde_json::from_str(include_str!("../../../problem-bank/variants.json")).unwrap();
    let judges: Value =
        serde_json::from_str(include_str!("../../../problem-bank/judges.json")).unwrap();
    let mut hashes = BTreeMap::new();
    for problem in problems.as_array().unwrap() {
        let id = problem["id"].as_str().unwrap();
        let exercise = Exercise(Arc::new(
            TaskRecord::from_bank(problem, &judges[id], &variants[id], None).unwrap(),
        ));
        let initial = exercise.task_prompt(Phase::Work, None, "").unwrap();
        let recovery = exercise.task_prompt(Phase::WrapUp, None, "").unwrap();
        for prompt in [&initial, &recovery] {
            assert!(
                !prompt.contains(problem["optimal"].as_str().unwrap()),
                "optimal leak for {id}"
            );
            assert!(
                !prompt.contains(problem["pitfalls"].as_str().unwrap()),
                "pitfalls leak for {id}"
            );
            assert!(!prompt.contains("\"optimal\":"));
            assert!(!prompt.contains("\"pitfalls\":"));
            assert!(!prompt.contains("\"guide\":"));
            if let Some(guide) = crate::agent::problems::guide_for(id) {
                assert!(!prompt.contains(guide), "guide leak for {id}");
            }
        }
        let digest = |text: &str| {
            ring::digest::digest(&ring::digest::SHA256, text.as_bytes())
                .as_ref()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        };
        hashes.insert(
            id.to_owned(),
            json!({"live": digest(&initial), "recovery": digest(&recovery)}),
        );
    }
    assert_eq!(hashes.len(), crate::agent::PROBLEMS.len());
    assert!(hashes.len() >= 150);
    let actual = format!("{}\n", serde_json::to_string_pretty(&hashes).unwrap());
    let path = "tests/golden/task-prompts.json";
    if std::env::var("UPDATE_PROMPT_GOLDEN").as_deref() == Ok("1") {
        std::fs::write(path, &actual).unwrap();
    }
    assert_eq!(actual, std::fs::read_to_string(path).unwrap());
}

#[test]
fn instructor_rendered_prompts_match_the_runtime_composition() {
    for (fixture_name, id) in [
        ("customized", "delimiter-closer"),
        ("uncustomized", "two-sum"),
    ] {
        let exercise = Exercise(Arc::new(record(&fixture(fixture_name)).unwrap()));
        let expected =
            std::fs::read_to_string(format!("tests/golden/task-preview/{id}.txt")).unwrap();
        assert_eq!(
            exercise.task_prompt(Phase::Ready, None, "").unwrap(),
            expected
        );
    }
}

#[test]
fn downloaded_records_refuse_missing_and_malformed_bank_fields() {
    let base = fixture("customized");
    for (section, field) in [
        ("variant", "brief"),
        ("variant", "clarifications"),
        ("variant", "followUps"),
        ("judge", "kind"),
        ("judge", "checker"),
        ("problem", "topics"),
        ("problem", "difficulty"),
        ("problem", "origin"),
    ] {
        let mut bad = base.clone();
        bad[section][field] = Value::Null;
        assert!(record(&bad).is_err(), "accepted invalid {section}.{field}");
    }
    let mut bad = base.clone();
    bad["variant"]["examples"][0]["case"] = json!(true);
    assert!(record(&bad).is_err());
    let mut bad = base;
    bad["variant"]["unknown"] = json!("private notes");
    assert!(record(&bad).is_err());
}

#[test]
fn every_public_judge_and_starter_matches_the_instructor_preview() {
    let problems: Value =
        serde_json::from_str(include_str!("../../../problem-bank/problems.json")).unwrap();
    let variants: Value =
        serde_json::from_str(include_str!("../../../problem-bank/variants.json")).unwrap();
    let judges: Value =
        serde_json::from_str(include_str!("../../../problem-bank/judges.json")).unwrap();
    let expected: Value =
        serde_json::from_str(include_str!("../../golden/task-posed.json")).unwrap();
    for problem in problems.as_array().unwrap() {
        let id = problem["id"].as_str().unwrap();
        let record = TaskRecord::from_bank(problem, &judges[id], &variants[id], None).unwrap();
        let value = json!({"judge": record.judge(), "starterCode": record.starters, "optimal": record.optimal, "pitfalls": record.pitfalls});
        let bytes = serde_json::to_vec(&value).unwrap();
        let hash: String = ring::digest::digest(&ring::digest::SHA256, &bytes)
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(json!(hash), expected[id], "preview mismatch for {id}");
    }
}

#[test]
fn shared_downloaded_record_refusals_match_the_instructor_tool() {
    let refusals: Value =
        serde_json::from_str(include_str!("../../fixtures/task-mode/refusals.json")).unwrap();
    for refusal in refusals.as_array().unwrap() {
        let mut row = fixture(refusal["fixture"].as_str().unwrap_or("customized"));
        let patches = refusal
            .get("patches")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_else(|| vec![refusal.clone()]);
        for patch in patches {
            let mut target = &mut row;
            for key in patch["path"].as_array().unwrap() {
                let key = key.as_str().unwrap();
                if target.is_array() {
                    target = &mut target[key.parse::<usize>().unwrap()];
                } else {
                    target = &mut target[key];
                }
            }
            *target = patch["value"].clone();
        }
        assert!(record(&row).is_err(), "accepted {}", refusal["name"]);
    }
}

#[test]
fn renamed_class_method_has_matching_starters_and_practice_operations() {
    let record = record(&fixture("class-renamed-method")).unwrap();
    let expected: Value = serde_json::from_str(include_str!(
        "../../golden/task-preview/class-renamed-method.json"
    ))
    .unwrap();
    assert_eq!(
        json!({"judge": record.judge, "starterCode": record.starters}),
        expected
    );
}

#[test]
fn task_ids_may_start_with_a_digit_but_not_a_dash() {
    assert!(crate::tasks::record_id("3sum"));
    assert!(crate::tasks::record_id("delimiter-closer"));
    for bad in ["", "-x", "Upper", "under_score", &"a".repeat(65)] {
        assert!(!crate::tasks::record_id(bad), "{bad}");
    }
}
