use super::*;
use crate::tasks::test_support::{PIN, download as fixture};
use serde_json::json;

#[test]
fn a_python_built_package_opens_with_its_pin_and_nothing_else() {
    let fixture = fixture();
    let open = |set: &str, version, manifest: &[u8], cipher: &[u8], pin| {
        open_package(set, version, manifest, cipher.to_vec(), pin)
    };
    let set = open("classroom", 7, &fixture.manifest, &fixture.ciphertext, PIN).unwrap();
    assert_eq!(set.records.len(), 2);
    assert_eq!(set.config.rule("durationMin"), 15);
    assert_eq!(set.config.rule("lookAwaySeconds"), 8);

    assert_eq!(
        open(
            "classroom",
            7,
            &fixture.manifest,
            &fixture.ciphertext,
            "654321"
        )
        .err(),
        Some(OpenError::WrongPin)
    );
    assert_eq!(
        open(
            "classroom",
            7,
            &fixture.manifest,
            &fixture.ciphertext,
            "12a456"
        )
        .err(),
        Some(OpenError::WrongPin)
    );
    // A damaged download fails its hash before any key is derived.
    let mut corrupt = fixture.ciphertext.clone();
    *corrupt.last_mut().unwrap() ^= 1;
    assert!(matches!(
        open("classroom", 7, &fixture.manifest, &corrupt, PIN),
        Err(OpenError::Invalid(_))
    ));
    for (set, version) in [("other", 7), ("classroom", 8)] {
        assert!(matches!(
            open(set, version, &fixture.manifest, &fixture.ciphertext, PIN),
            Err(OpenError::Invalid(_))
        ));
    }
}

#[test]
fn the_salt_is_bound_into_the_key() {
    let fixture = fixture();
    let mut manifest: Value = serde_json::from_slice(&fixture.manifest).unwrap();
    manifest["salt"] = Value::from("AAAAAAAAAAAAAAAAAAAAAA==");
    let manifest = serde_json::to_vec(&manifest).unwrap();
    assert_eq!(
        open_package("classroom", 7, &manifest, fixture.ciphertext, PIN).err(),
        Some(OpenError::WrongPin)
    );
}

#[test]
fn a_site_is_an_https_base_that_sets_join_under() {
    assert_eq!(
        site_base("https://teacher.github.io/course")
            .unwrap()
            .as_str(),
        "https://teacher.github.io/course/"
    );
    assert_eq!(
        site_base("https://teacher.github.io/course/")
            .unwrap()
            .join("sets/classroom/7/manifest.json")
            .unwrap()
            .as_str(),
        "https://teacher.github.io/course/sets/classroom/7/manifest.json"
    );
    for bad in [
        "http://teacher.github.io/course",
        "https://user:pw@teacher.github.io",
        "https://teacher.github.io/course?x=1",
        "https://teacher.github.io/course#top",
        "teacher.github.io",
    ] {
        assert!(site_base(bad).is_err(), "{bad}");
    }
    assert!(parse_close("2026-10-06T12:00:00+00:00").is_err());
    assert!(parse_close("2026-10-06T12:00:00Z").is_ok());
    // The packaging tool writes whole seconds only; both sides take one form.
    assert!(parse_close("2026-10-06T12:00:00.5Z").is_err());
    assert!(parse_close("2026-02-31T12:00:00Z").is_err());
}

/// Serves fixed responses by path until dropped, with the `Date` header a
/// static host sends.
async fn routed_fixture(
    routes: Vec<(&'static str, u16, Vec<u8>, Option<String>)>,
) -> (reqwest::Url, tokio::task::JoinHandle<()>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = reqwest::Url::parse(&format!(
        "http://{}/course/",
        listener.local_addr().unwrap()
    ))
    .unwrap();
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let mut request = [0; 8192];
            let read = stream.read(&mut request).await.unwrap_or(0);
            let line = String::from_utf8_lossy(&request[..read]);
            let path = line.split_whitespace().nth(1).unwrap_or("").to_owned();
            let (status, body, location) = routes
                .iter()
                .find(|(route, ..)| *route == path)
                .map(|(_, status, body, location)| (*status, body.clone(), location.clone()))
                .unwrap_or((404, b"missing".to_vec(), None));
            let location = location
                .map(|value| format!("Location: {value}\r\n"))
                .unwrap_or_default();
            let header = format!(
                "HTTP/1.1 {status} fixture\r\nContent-Length: {}\r\nDate: Tue, 06 Oct 2026 12:00:00 GMT\r\nConnection: close\r\n{location}\r\n",
                body.len()
            );
            let _ = stream.write_all(header.as_bytes()).await;
            let _ = stream.write_all(&body).await;
        }
    });
    (url, task)
}

#[tokio::test]
async fn a_download_takes_the_versioned_files_and_the_site_clock_and_refuses_the_rest() {
    let fixture = fixture();
    const MANIFEST: &str = "/course/sets/classroom/7/manifest.json";
    const CIPHER: &str = "/course/sets/classroom/7/tasks.enc";
    let valid = || {
        vec![
            (MANIFEST, 200, fixture.manifest.clone(), None),
            (CIPHER, 200, fixture.ciphertext.clone(), None),
        ]
    };
    let fetched = |routes| async move {
        let (site, server) = routed_fixture(routes).await;
        let downloaded = download(&site, "classroom", 7).await;
        server.abort();
        downloaded
    };
    let downloaded = fetched(valid()).await.unwrap();
    assert_eq!(downloaded.ciphertext, fixture.ciphertext);
    assert_eq!(
        downloaded.site_time.map(|(at, _)| at),
        Some(parse_close("2026-10-06T12:00:00Z").unwrap())
    );
    let redirect = vec![
        (
            MANIFEST,
            302,
            Vec::new(),
            Some("https://elsewhere.invalid/manifest.json".to_owned()),
        ),
        (CIPHER, 200, fixture.ciphertext.clone(), None),
    ];
    let missing = vec![(MANIFEST, 200, fixture.manifest.clone(), None)];
    let oversize = vec![
        (MANIFEST, 200, vec![b' '; MAX_MANIFEST_BYTES + 1], None),
        (CIPHER, 200, fixture.ciphertext.clone(), None),
    ];
    for (name, routes) in [
        ("redirect", redirect),
        ("404", missing),
        ("oversize", oversize),
    ] {
        assert!(fetched(routes).await.is_err(), "{name} downloaded");
    }
    let (site, server) = routed_fixture(valid()).await;
    assert!(download(&site, "../etc", 7).await.is_err());
    server.abort();
}

/// The fixture set as the packaging tool encrypts it: two tasks, default
/// rules.
fn plaintext() -> Value {
    let rows: Vec<Value> = [
        include_str!("../../fixtures/task-mode/customized.json"),
        include_str!("../../fixtures/task-mode/uncustomized.json"),
    ]
    .iter()
    .map(|text| serde_json::from_str(text).unwrap())
    .collect();
    let by_id = |field: &str| {
        Value::Object(
            rows.iter()
                .filter(|row| row.get(field).is_some())
                .map(|row| {
                    (
                        row["problem"]["id"].as_str().unwrap().to_owned(),
                        row[field].clone(),
                    )
                })
                .collect(),
        )
    };
    json!({"packageVersion": 1, "setId": "classroom", "setVersion": 7,
        "rules": {"durationMin": 15, "maxHintRungs": 3, "closesAt": null,
            "lookAwaySeconds": 8, "rulesNote": null},
        "problems": rows.iter().map(|row| row["problem"].clone()).collect::<Vec<_>>(),
        "judges": by_id("judge"), "variants": by_id("variant"), "sidecars": by_id("sidecar")})
}

fn decoded(value: &Value) -> Result<LoadedSet, TaskError> {
    decode("classroom", 7, &serde_json::to_vec(value).unwrap())
}

#[test]
fn a_decoded_package_must_name_the_set_it_was_opened_for() {
    let set = decoded(&plaintext()).unwrap();
    assert_eq!(set.records.len(), 2);
    assert_eq!(
        (set.config.id.as_str(), set.config.version),
        ("classroom", 7)
    );
    for (field, value) in [
        ("packageVersion", json!(2)),
        ("setId", json!("other")),
        ("setVersion", json!(8)),
        ("problems", json!([])),
    ] {
        let mut changed = plaintext();
        changed[field] = value;
        assert!(decoded(&changed).is_err(), "{field}");
    }
}

#[test]
fn set_rules_are_held_to_their_bounds_at_both_ends() {
    let with = |field: &str, value: Value| {
        let mut changed = plaintext();
        changed["rules"][field] = value;
        decoded(&changed)
    };
    let (low, high) = (
        crate::config::MIN_DURATION_MIN as u64,
        crate::config::MAX_DURATION_MIN as u64,
    );
    let hints = default_number("maxHintRungs");
    for (field, accepted, refused) in [
        ("durationMin", vec![low, high], vec![low - 1, high + 1]),
        ("maxHintRungs", vec![0, hints], vec![hints + 1]),
        ("lookAwaySeconds", vec![5, 60], vec![4, 61]),
    ] {
        for value in accepted {
            let set = with(field, json!(value)).unwrap_or_else(|e| panic!("{field}={value}: {e}"));
            assert_eq!(set.config.rule(field), value, "{field}");
        }
        for value in refused {
            assert!(with(field, json!(value)).is_err(), "{field}={value}");
        }
    }
    assert!(with("closesAt", json!("2026-02-31T00:00:00Z")).is_err());
    assert_eq!(
        with("closesAt", json!("2026-12-31T00:00:00Z"))
            .unwrap()
            .config
            .closes_at
            .as_deref(),
        Some("2026-12-31T00:00:00Z")
    );
    let note = "x".repeat(MAX_RULES_NOTE_BYTES);
    assert_eq!(
        with("rulesNote", json!(note)).unwrap().config.rules_note,
        Some(note)
    );
    for refused in [json!(" "), json!("x".repeat(MAX_RULES_NOTE_BYTES + 1))] {
        assert!(with("rulesNote", refused).is_err());
    }
}

#[test]
fn every_record_must_be_whole_and_belong_to_a_task() {
    let id = plaintext()["problems"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    for field in ["judges", "variants"] {
        let mut missing = plaintext();
        missing[field].as_object_mut().unwrap().remove(&id);
        assert!(decoded(&missing).is_err(), "missing {field}");
        let mut orphan = plaintext();
        orphan[field]["stray-task"] = orphan[field][&id].clone();
        assert!(decoded(&orphan).is_err(), "orphan {field}");
    }
    let mut orphan = plaintext();
    orphan["sidecars"]["stray-task"] = orphan["sidecars"][&id].clone();
    assert!(decoded(&orphan).is_err(), "orphan sidecar");
    let mut duplicate = plaintext();
    let first = duplicate["problems"][0].clone();
    duplicate["problems"].as_array_mut().unwrap().push(first);
    assert_eq!(decoded(&duplicate).err().unwrap().0, "duplicate task id");
    let mut nameless = plaintext();
    nameless["problems"][0]
        .as_object_mut()
        .unwrap()
        .remove("id");
    assert!(decoded(&nameless).is_err());
}

#[test]
fn one_task_may_not_outgrow_its_byte_limit() {
    let limit = default_number("taskBytes") as usize;
    let mut padded = plaintext();
    let id = padded["problems"][0]["id"].as_str().unwrap().to_owned();
    let row_bytes = |value: &Value| {
        serde_json::to_vec(
            &json!({"problem": value["problems"][0], "judge": value["judges"][&id],
            "variant": value["variants"][&id], "sidecar": value["sidecars"].get(&id)}),
        )
        .unwrap()
        .len()
    };
    // Exactly at the limit is still a task; one byte more is not.
    let room = limit - row_bytes(&padded);
    padded["problems"][0]["topics"][0] = json!(format!(
        "{}{}",
        padded["problems"][0]["topics"][0].as_str().unwrap(),
        "x".repeat(room)
    ));
    assert_eq!(row_bytes(&padded), limit);
    assert!(
        decoded(&padded)
            .err()
            .is_none_or(|error| error.0 != "plaintext task exceeds limit")
    );
    padded["problems"][0]["topics"][0] = json!(format!(
        "{}x",
        padded["problems"][0]["topics"][0].as_str().unwrap()
    ));
    assert_eq!(
        decoded(&padded).err().unwrap().0,
        "plaintext task exceeds limit"
    );
}
