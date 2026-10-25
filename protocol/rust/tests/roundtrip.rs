//! Every example in protocol/examples decodes into a known variant and re-encodes to the same JSON.
use neo_protocol::{Envelope, SessionEvent};

#[test]
fn examples_round_trip() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../examples");
    let mut count = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        count += 1;
        let original: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let envelope: Envelope = serde_json::from_value(original.clone())
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert!(
            !matches!(envelope, Envelope::Unknown(_)),
            "{}: unknown topic",
            path.display()
        );
        if let Envelope::SessionEvents(m) = &envelope {
            assert!(
                !matches!(m.payload.event, SessionEvent::Unknown(_)),
                "{}: unknown event",
                path.display()
            );
        }
        let encoded = serde_json::to_value(&envelope).unwrap();
        assert_eq!(encoded, original, "{}", path.display());
    }
    assert!(count > 20, "only {count} examples found");
}

#[test]
fn unknown_variants_survive() {
    let input = serde_json::json!({
        "topic": "session.events",
        "payload": {"session": "s", "seq": 3, "ts": 1, "event": {"type": "from_the_future", "x": [1, 2]}}
    });
    let envelope: Envelope = serde_json::from_value(input.clone()).unwrap();
    let Envelope::SessionEvents(m) = &envelope else {
        panic!("wrong topic")
    };
    assert!(matches!(m.payload.event, SessionEvent::Unknown(_)));
    assert_eq!(serde_json::to_value(&envelope).unwrap(), input);
}

#[test]
fn missing_tag_is_an_error() {
    assert!(serde_json::from_str::<Envelope>(r#"{"payload": {}}"#).is_err());
}
