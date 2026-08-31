//! The envelope is the server's public contract. These tests pin its shape.

use mcs51_mcp::envelope::{deliver, envelope_output_schema, Envelope, NextAction, Status, Tail};
use mcs51_mcp::errors::{AppError, ErrorCode};
use serde_json::{json, Value};

/// The JSON a client would actually see in `structuredContent`.
fn structured(env: Envelope) -> Value {
    env.finish()
        .structured_content
        .expect("every tool must return structuredContent")
}

/// The text a human would see in `content`.
fn content_text(env: Envelope) -> String {
    let result = env.finish();
    match result.content.first().expect("content block") {
        rmcp::model::ContentBlock::Text(t) => t.text.clone(),
        other => panic!("expected a text block, got {other:?}"),
    }
}

#[test]
fn success_envelope_has_the_documented_shape() {
    let v = structured(
        Envelope::new("pinout")
            .data(json!({ "pin": 19 }))
            .duration(std::time::Duration::from_millis(7)),
    );

    assert_eq!(v["ok"], json!(true));
    assert_eq!(v["status"], json!("ok"));
    assert_eq!(v["tool"], json!("pinout"));
    assert_eq!(v["error_code"], Value::Null);
    assert_eq!(v["error"], Value::Null);
    assert_eq!(v["data"]["pin"], json!(19));
    assert_eq!(v["duration_ms"], json!(7));
    assert_eq!(v["next_actions"], json!([]));

    // Every documented key is present even when null, so a caller never has to
    // distinguish "absent" from "empty".
    for key in [
        "ok",
        "status",
        "tool",
        "error_code",
        "error",
        "remedy",
        "command",
        "exit_code",
        "duration_ms",
        "stdout",
        "stderr",
        "data",
        "next_actions",
    ] {
        assert!(v.get(key).is_some(), "missing envelope key {key}");
    }
}

#[test]
fn content_and_structured_content_carry_the_same_value() {
    let env = Envelope::new("doctor")
        .data(json!({ "tools": ["sdcc"] }))
        .command("sdcc --version")
        .exit_code(Some(0));

    let text = content_text(env.clone());
    let structured = structured(env);

    let from_text: Value = serde_json::from_str(&text).expect("content must be valid JSON");
    assert_eq!(from_text, structured);
    // Pretty, not compact: a human reads this in a transcript.
    assert!(text.contains("\n  "), "content should be pretty-printed");
}

#[test]
fn failure_sets_ok_status_and_is_error_together() {
    let result = Envelope::new("flash")
        .error(ErrorCode::FlashFailed, "stcgal exited 1")
        .remedy("power-cycle the board")
        .finish();

    assert_eq!(result.is_error, Some(true));
    let v = result.structured_content.unwrap();
    assert_eq!(v["ok"], json!(false));
    assert_eq!(v["status"], json!("error"));
    // SCREAMING_SNAKE on the wire, so a caller can match on it.
    assert_eq!(v["error_code"], json!("FLASH_FAILED"));
    assert_eq!(v["error"], json!("stcgal exited 1"));
    assert_eq!(v["remedy"], json!("power-cycle the board"));
}

#[test]
fn a_warning_still_reports_ok() {
    let v = structured(Envelope::new("doctor").warn());
    assert_eq!(v["ok"], json!(true));
    assert_eq!(v["status"], json!("warning"));
    assert_eq!(v["error_code"], Value::Null);
}

#[test]
fn envelope_round_trips_through_json() {
    let original = Envelope::new("compile")
        .command("sdcc -mmcs51 firmware.c")
        .exit_code(Some(1))
        .stdout(Tail {
            text: "head...tail".into(),
            total_bytes: 4096,
            elided_bytes: 4085,
            truncated: true,
        })
        .stderr(Tail::default())
        .data(json!({ "source": "firmware.c" }))
        .next_action(NextAction::call(
            "flash",
            "write it",
            json!({ "chip": "stc" }),
        ))
        .error(ErrorCode::CompileFailed, "sdcc exited 1");

    let encoded = serde_json::to_value(&original).unwrap();
    let decoded: Envelope = serde_json::from_value(encoded.clone()).expect("round-trip");

    assert!(!decoded.ok);
    assert_eq!(decoded.status, Status::Error);
    assert_eq!(decoded.tool, "compile");
    assert_eq!(decoded.error_code, Some(ErrorCode::CompileFailed));
    assert_eq!(decoded.exit_code, Some(1));
    let stdout = decoded.stdout.clone().expect("stdout");
    assert!(stdout.truncated);
    assert_eq!(stdout.total_bytes, 4096);
    assert_eq!(decoded.next_actions.len(), 1);
    assert_eq!(decoded.next_actions[0].tool.as_deref(), Some("flash"));

    // Re-encoding is byte-identical, so nothing is lost in the trip.
    assert_eq!(serde_json::to_value(&decoded).unwrap(), encoded);
}

#[test]
fn the_declared_output_schema_matches_what_tools_emit() {
    let schema = envelope_output_schema();
    let schema = serde_json::to_value(&*schema).unwrap();

    // Trap #2 from the rmcp notes: a non-object root panics at construction.
    assert_eq!(schema["type"], json!("object"));
    let props = schema["properties"]
        .as_object()
        .expect("schema must describe properties");
    for key in ["ok", "status", "tool", "error_code", "data", "next_actions"] {
        assert!(props.contains_key(key), "schema is missing {key}");
    }

    // And the schema describes real emitted values, not a parallel invention.
    let emitted = structured(Envelope::new("pinout"));
    for key in props.keys() {
        assert!(
            emitted.get(key).is_some(),
            "schema declares `{key}` but the envelope never emits it"
        );
    }
}

#[test]
fn errors_become_envelopes_without_the_call_site_helping() {
    let result = deliver(
        "serial_read",
        Err(AppError::SessionNotFound {
            session: "board".into(),
        }),
    );

    assert_eq!(result.is_error, Some(true));
    let v = result.structured_content.unwrap();
    assert_eq!(v["ok"], json!(false));
    assert_eq!(v["tool"], json!("serial_read"));
    assert_eq!(v["error_code"], json!("SERIAL_SESSION_NOT_FOUND"));
    assert_eq!(v["data"]["session"], json!("board"));
    assert!(v["remedy"].is_string(), "an error should suggest a remedy");
    // PORT_HELD_BY_SESSION-style errors point at the tool that unsticks them.
    assert_eq!(
        v["next_actions"][0]["tool"],
        json!("serial_list_sessions"),
        "unknown-session errors should point at the session list"
    );
}

#[test]
fn port_held_by_session_points_at_serial_close() {
    let result = deliver(
        "flash",
        Err(AppError::PortHeldBySession {
            port: "/dev/cu.usbserial-10".into(),
            session: "board".into(),
        }),
    );
    let v = result.structured_content.unwrap();
    assert_eq!(v["error_code"], json!("PORT_HELD_BY_SESSION"));
    assert_eq!(v["next_actions"][0]["tool"], json!("serial_close"));
    assert_eq!(v["next_actions"][0]["arguments"]["session"], json!("board"));
}
