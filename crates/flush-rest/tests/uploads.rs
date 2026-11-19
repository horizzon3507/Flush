//! Upload and account routes against a loopback HTTP server (no external network).

use std::sync::{Arc, Mutex};

use async_std::io::prelude::*;
use async_std::net::{TcpListener, TcpStream};
use async_std::task;
use serde_json::{json, Value};

use flush_core::{MessageSend, Snowflake};
use flush_rest::{
    multipart_body, split_parts, AttachmentUploadPart, AttachmentUploadPlan, BulkMessageRequest,
    FileUpload, RestClient, RestConfig, RestError, RoleHoistPosition, ThreadMemberSettings,
};

#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
    auth: Option<String>,
    content_type: Option<String>,
    body: Vec<u8>,
}

impl Seen {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap()
    }
}

type Log = Arc<Mutex<Vec<Seen>>>;
type Responder = Arc<dyn Fn(&Seen, &str) -> (u16, Value) + Send + Sync>;

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

async fn serve(mut stream: TcpStream, base: String, log: Log, respond: Responder) {
    let mut buf = Vec::new();
    loop {
        let head_end = loop {
            if let Some(i) = find(&buf, b"\r\n\r\n") {
                break i;
            }
            let mut chunk = [0u8; 8192];
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => return,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        };
        let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
        let mut lines = head.split("\r\n");
        let mut first = lines.next().unwrap_or("").split(' ');
        let method = first.next().unwrap_or("").to_owned();
        let path = first.next().unwrap_or("").to_owned();
        let mut seen = Seen {
            method,
            path,
            auth: None,
            content_type: None,
            body: Vec::new(),
        };
        let mut length = 0usize;
        for line in lines {
            let (k, v) = line.split_once(':').unwrap_or((line, ""));
            let v = v.trim().to_owned();
            match k.to_ascii_lowercase().as_str() {
                "content-length" => length = v.parse().unwrap_or(0),
                "authorization" => seen.auth = Some(v),
                "content-type" => seen.content_type = Some(v),
                _ => {}
            }
        }
        let start = head_end + 4;
        while buf.len() < start + length {
            let mut chunk = [0u8; 8192];
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => return,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        }
        seen.body = buf[start..start + length].to_vec();
        buf.drain(..start + length);
        let (status, value) = respond(&seen, &base);
        log.lock().unwrap().push(seen);
        let text = if value.is_null() {
            String::new()
        } else {
            value.to_string()
        };
        let out = format!(
            "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{text}",
            text.len()
        );
        if stream.write_all(out.as_bytes()).await.is_err() {
            return;
        }
    }
}

async fn start(respond: Responder) -> (RestClient, Log) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let log: Log = Arc::default();
    let (log2, base2) = (log.clone(), base.clone());
    task::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            task::spawn(serve(stream, base2.clone(), log2.clone(), respond.clone()));
        }
    });
    let mut config = RestConfig::new("tok");
    config.api_base = Some(base);
    config.gateway_url = Some("ws://unused".into());
    (RestClient::new(config).await.unwrap(), log)
}

fn message() -> Value {
    json!({ "id": "5", "channel_id": "10", "author": { "id": "1", "username": "a" } })
}

#[async_std::test]
async fn multipart_message_carries_payload_and_files() {
    let (rest, log) = start(Arc::new(|_, _| (200, message()))).await;
    let files = [FileUpload::new("a \"b\".txt", b"hello".to_vec()).content_type("text/plain")];
    let sent = rest
        .send_message_with_files(Snowflake(10), &MessageSend::text("hi"), &files)
        .await
        .unwrap();
    assert_eq!(sent.id, Snowflake(5));

    let seen = log.lock().unwrap()[0].clone();
    assert_eq!(
        (seen.method.as_str(), seen.path.as_str()),
        ("POST", "/v1/channels/10/messages")
    );
    assert_eq!(seen.auth.as_deref(), Some("Bot tok"));
    let ct = seen.content_type.unwrap();
    let boundary = ct.strip_prefix("multipart/form-data; boundary=").unwrap();
    let body = String::from_utf8(seen.body).unwrap();
    assert!(body.starts_with(&format!("--{boundary}\r\n")));
    assert!(body.ends_with(&format!("--{boundary}--\r\n")));
    assert!(body.contains("name=\"payload_json\""));
    assert!(body.contains(r#""attachments":[{"id":0,"filename":"a \"b\".txt"}]"#));
    assert!(body.contains(
        "name=\"files[0]\"; filename=\"a %22b%22.txt\"\r\nContent-Type: text/plain\r\n\r\nhello\r\n"
    ));
}

#[async_std::test]
async fn upload_plan_puts_bytes_and_completes_multipart() {
    let respond: Responder = Arc::new(|seen, base| match seen.path.as_str() {
        "/v1/channels/10/attachments" => (
            200,
            json!({ "attachments": [
                { "id": 0, "filename": "a.txt", "upload_filename": "up/a.txt", "file_size": 3,
                  "content_type": "text/plain", "upload_mode": "singlepart",
                  "upload_url": format!("{base}/store/a") },
                { "id": 1, "filename": "b.bin", "upload_filename": "up/b.bin", "file_size": 5,
                  "content_type": "application/octet-stream", "upload_mode": "multipart",
                  "upload_id": "u1", "part_size": 2, "parts": [
                    { "part_number": 3, "upload_url": format!("{base}/store/b3") },
                    { "part_number": 1, "upload_url": format!("{base}/store/b1") },
                    { "part_number": 2, "upload_url": format!("{base}/store/b2") } ] } ] }),
        ),
        "/v1/channels/10/attachments/complete" => (
            200,
            json!({ "uploads": [{ "upload_filename": "up/b.bin" }] }),
        ),
        _ => (200, Value::Null),
    });
    let (rest, log) = start(respond).await;
    let files = [
        FileUpload::new("a.txt", b"abc".to_vec())
            .content_type("text/plain")
            .description("note"),
        FileUpload::new("b.bin", b"12345".to_vec()),
    ];
    let refs = rest
        .upload_attachments(Snowflake(10), &files)
        .await
        .unwrap();
    assert_eq!(refs.len(), 2);
    assert_eq!(refs[0].upload_filename.as_deref(), Some("up/a.txt"));
    assert_eq!(refs[0].description.as_deref(), Some("note"));
    assert_eq!(refs[1].file_size, Some(5));

    let log = log.lock().unwrap().clone();
    let seq: Vec<(&str, &str, &[u8])> = log
        .iter()
        .skip(1)
        .take(4)
        .map(|s| (s.method.as_str(), s.path.as_str(), s.body.as_slice()))
        .collect();
    assert_eq!(
        seq,
        vec![
            ("PUT", "/store/a", &b"abc"[..]),
            ("PUT", "/store/b1", &b"12"[..]),
            ("PUT", "/store/b2", &b"34"[..]),
            ("PUT", "/store/b3", &b"5"[..]),
        ]
    );
    assert_eq!(
        log[0].json(),
        json!({ "attachments": [
            { "id": 0, "filename": "a.txt", "file_size": 3, "content_type": "text/plain" },
            { "id": 1, "filename": "b.bin", "file_size": 5,
              "content_type": "application/octet-stream" } ] })
    );
    assert_eq!(log[1].content_type.as_deref(), Some("text/plain"));
    assert!(log[1..5].iter().all(|s| s.auth.is_none()));
    assert_eq!(log[5].path, "/v1/channels/10/attachments/complete");
    assert_eq!(
        log[5].json(),
        json!({ "uploads": [{ "upload_filename": "up/b.bin", "upload_id": "u1" }] })
    );
}

#[async_std::test]
async fn failed_presigned_put_is_reported() {
    let respond: Responder = Arc::new(|seen, base| match seen.path.as_str() {
        "/v1/channels/10/attachments" => (
            200,
            json!({ "attachments": [{ "id": 0, "upload_filename": "up/a",
                "upload_mode": "singlepart", "upload_url": format!("{base}/store/a") }] }),
        ),
        _ => (403, json!("denied")),
    });
    let (rest, _) = start(respond).await;
    let err = rest
        .upload_attachments(Snowflake(10), &[FileUpload::new("a", vec![1])])
        .await
        .unwrap_err();
    assert!(matches!(err, RestError::Api { status: 403, .. }), "{err}");
}

#[async_std::test]
async fn account_routes_send_sdk_methods_and_bodies() {
    let respond: Responder = Arc::new(|seen, _| match seen.path.as_str() {
        "/v1/channels/10/messages/purge" => (200, json!({ "deleted_count": 7 })),
        "/v1/channels/messages/bulk" => (
            200,
            json!({ "channels": [{ "channel_id": "10", "messages": [message()] }] }),
        ),
        "/v1/users/@me/preload-messages" => (200, json!({ "10": message(), "11": null })),
        "/v1/guilds/3/roles/hoist-positions" => (200, json!([])),
        "/v1/guilds/3/discovery" => (200, json!({ "eligible": true, "min_member_count": 50 })),
        "/v1/users/check-tag?username=a%20b&discriminator=0001" => (200, json!({ "taken": true })),
        "/v1/oauth2/applications/9/bot/reset-token" => (200, json!({ "token": "new" })),
        _ => (204, Value::Null),
    });
    let (rest, log) = start(respond).await;
    let (c, g, u) = (Snowflake(10), Snowflake(3), Snowflake(2));

    assert_eq!(rest.purge_messages(c).await.unwrap(), 7);
    let bulk = rest
        .bulk_fetch_messages(&[BulkMessageRequest::latest(c, 5)])
        .await
        .unwrap();
    assert_eq!(bulk[0].messages.len(), 1);
    let pre = rest.preload_messages(&[c, Snowflake(11)]).await.unwrap();
    assert!(pre[&c].is_some() && pre[&Snowflake(11)].is_none());
    rest.set_role_hoist_positions(
        g,
        &[RoleHoistPosition {
            id: Snowflake(4),
            hoist_position: Some(1),
        }],
    )
    .await
    .unwrap();
    assert!(rest.guild_discovery_status(g).await.unwrap().eligible);
    assert!(rest.check_username_tag("a b", "0001").await.unwrap());
    assert_eq!(
        rest.reset_application_bot_token(Snowflake(9), None)
            .await
            .unwrap(),
        "new"
    );
    rest.delete_guild(g, None).await.unwrap();
    rest.transfer_guild_ownership(g, u, Some("pw"))
        .await
        .unwrap();
    rest.bulk_delete_my_messages(c, None).await.unwrap();
    rest.clear_read_state(c).await.unwrap();
    rest.acknowledge_pins(c).await.unwrap();
    rest.add_group_recipient(c, u).await.unwrap();
    rest.remove_group_recipient(c, u, true).await.unwrap();
    rest.pin_dm_channel(c).await.unwrap();
    rest.unpin_dm_channel(c).await.unwrap();
    let member = rest
        .edit_thread_member_settings(
            c,
            &ThreadMemberSettings {
                muted: Some(true),
                mute_config: Some(None),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(member.is_none());

    let log = log.lock().unwrap().clone();
    let body = |i: usize| -> Value {
        if log[i].body.is_empty() {
            Value::Null
        } else {
            log[i].json()
        }
    };
    let calls: Vec<(String, Value)> = log
        .iter()
        .enumerate()
        .map(|(i, s)| (format!("{} {}", s.method, &s.path[3..]), body(i)))
        .collect();
    let expected = vec![
        ("POST /channels/10/messages/purge", Value::Null),
        (
            "POST /channels/messages/bulk",
            json!({ "requests": [{ "channel_id": "10", "limit": 5 }] }),
        ),
        (
            "POST /users/@me/preload-messages",
            json!({ "channels": ["10", "11"] }),
        ),
        (
            "PATCH /guilds/3/roles/hoist-positions",
            json!([{ "id": "4", "hoist_position": 1 }]),
        ),
        ("GET /guilds/3/discovery", Value::Null),
        (
            "GET /users/check-tag?username=a%20b&discriminator=0001",
            Value::Null,
        ),
        ("POST /oauth2/applications/9/bot/reset-token", Value::Null),
        ("POST /guilds/3/delete", json!({})),
        (
            "POST /guilds/3/transfer-ownership",
            json!({ "new_owner_id": "2", "password": "pw" }),
        ),
        ("POST /channels/10/messages/bulk-delete-mine", Value::Null),
        ("DELETE /channels/10/messages/ack", Value::Null),
        ("POST /channels/10/pins/ack", Value::Null),
        ("PUT /channels/10/recipients/2", Value::Null),
        ("DELETE /channels/10/recipients/2?silent=true", Value::Null),
        ("PUT /users/@me/channels/10/pin", Value::Null),
        ("DELETE /users/@me/channels/10/pin", Value::Null),
        (
            "PATCH /channels/10/thread-members/@me/settings",
            json!({ "muted": true, "mute_config": null }),
        ),
    ];
    for (got, want) in calls.iter().zip(&expected) {
        assert_eq!((got.0.as_str(), &got.1), (want.0, &want.1));
    }
    assert_eq!(calls.len(), expected.len());
    assert!(log.iter().all(|s| s.auth.as_deref() == Some("Bot tok")));
}

#[test]
fn split_parts_rejects_bad_plans() {
    let mut plan: AttachmentUploadPlan = serde_json::from_value(json!({
        "id": 0, "upload_filename": "x", "upload_mode": "multipart", "upload_id": "u",
        "part_size": 4, "parts": null
    }))
    .unwrap();
    assert!(plan.is_multipart());
    assert!(split_parts(b"abcdef", &plan).unwrap().is_empty());
    plan.parts = vec![AttachmentUploadPart {
        part_number: 3,
        upload_url: "u".into(),
    }];
    assert!(split_parts(b"abcdef", &plan).is_err());
    plan.parts[0].part_number = 0;
    assert!(split_parts(b"abcdef", &plan).is_err());
    plan.parts[0].part_number = 2;
    assert_eq!(split_parts(b"abcdef", &plan).unwrap()[0].1, b"ef");
    plan.part_size = None;
    assert!(split_parts(b"abcdef", &plan).is_err());
}

#[test]
fn multipart_body_layout() {
    let body = multipart_body("B", "{}", &[FileUpload::new("f", vec![0, 255])]);
    let mut want = b"--B\r\nContent-Disposition: form-data; name=\"payload_json\"\r\nContent-Type: application/json\r\n\r\n{}\r\n--B\r\nContent-Disposition: form-data; name=\"files[0]\"; filename=\"f\"\r\nContent-Type: application/octet-stream\r\n\r\n".to_vec();
    want.extend_from_slice(&[0, 255]);
    want.extend_from_slice(b"\r\n--B--\r\n");
    assert_eq!(body, want);
}
