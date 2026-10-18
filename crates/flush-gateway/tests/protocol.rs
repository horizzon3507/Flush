use flush_gateway::{gateway_url_with_params, CloseCode, GatewayPayload, Opcode};

#[test]
fn gateway_url_completes_params() {
    assert_eq!(
        gateway_url_with_params("wss://gateway.fluxer.app"),
        "wss://gateway.fluxer.app?v=1&encoding=json"
    );
    assert_eq!(
        gateway_url_with_params("wss://gateway.fluxer.app/?v=1&encoding=json"),
        "wss://gateway.fluxer.app/?v=1&encoding=json"
    );
    assert_eq!(
        gateway_url_with_params("wss://gateway.fluxer.app/?v=1"),
        "wss://gateway.fluxer.app/?v=1&encoding=json"
    );
}

#[test]
fn opcode_registry_matches_docs() {
    assert_eq!(Opcode::from_u8(0), Opcode::Dispatch);
    assert_eq!(Opcode::from_u8(1), Opcode::Heartbeat);
    assert_eq!(Opcode::from_u8(2), Opcode::Identify);
    assert_eq!(Opcode::from_u8(6), Opcode::Resume);
    assert_eq!(Opcode::from_u8(10), Opcode::Hello);
    assert_eq!(Opcode::from_u8(11), Opcode::HeartbeatAck);
    assert_eq!(Opcode::from_u8(28), Opcode::RequestForumUnreads);
    assert_eq!(Opcode::from_u8(99), Opcode::Unknown);
}

#[test]
fn identify_payload_shape() {
    let payload = GatewayPayload::command(
        Opcode::Identify,
        serde_json::json!({
            "token": "tok",
            "properties": { "os": "linux", "browser": "Flush", "device": "flush-bot" },
        }),
    );
    assert_eq!(payload.op, 2);
    let text = serde_json::to_string(&payload).unwrap();
    assert!(text.contains("\"token\":\"tok\""));
}

#[test]
fn close_code_resumable_matrix() {
    assert!(!CloseCode::from_u16(4003).resumable());
    assert!(!CloseCode::from_u16(4010).resumable());
    assert!(!CloseCode::from_u16(4011).resumable());
    assert!(!CloseCode::from_u16(4012).resumable());
    assert!(CloseCode::from_u16(4000).resumable());
    assert!(CloseCode::from_u16(4007).resumable());
    assert!(CloseCode::from_u16(4009).resumable());
}
