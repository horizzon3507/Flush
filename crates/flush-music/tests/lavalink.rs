//! LavalinkNode against a loopback HTTP + WebSocket server (no external network).

use std::sync::{Arc, Mutex};

use async_std::io::prelude::*;
use async_std::net::{TcpListener, TcpStream};
use async_std::task;
use futures::channel::mpsc;
use futures::StreamExt;
use serde_json::{json, Value};

use flush_music::{
    LavalinkNode, LavalinkOptions, MusicBackend, MusicEvent, MusicPlayerUpdate, NodeState,
    TrackLoadResult, UpdateTrack,
};

#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
    auth: Option<String>,
    body: Vec<u8>,
}

type Log = Arc<Mutex<Vec<Seen>>>;

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn parse_head(buf: &[u8]) -> (Seen, usize, bool) {
    let head_end = find(buf, b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or("").split(' ');
    let method = first.next().unwrap_or("").to_owned();
    let path = first.next().unwrap_or("").to_owned();
    let mut seen = Seen {
        method,
        path,
        auth: None,
        body: Vec::new(),
    };
    let mut length = 0usize;
    let mut is_ws = false;
    for line in lines {
        let (k, v) = line.split_once(':').unwrap_or((line, ""));
        let v = v.trim().to_owned();
        match k.to_ascii_lowercase().as_str() {
            "content-length" => length = v.parse().unwrap_or(0),
            "authorization" => seen.auth = Some(v),
            "upgrade" if v.eq_ignore_ascii_case("websocket") => is_ws = true,
            _ => {}
        }
    }
    (seen, head_end + 4 + length, is_ws)
}

fn track_json(encoded: &str) -> Value {
    json!({
        "encoded": encoded,
        "info": {
            "identifier": encoded,
            "isSeekable": true,
            "author": "artist",
            "length": 1000,
            "isStream": false,
            "position": 0,
            "title": encoded,
            "uri": "https://example.com/x",
            "artworkUrl": null,
            "isrc": null,
            "sourceName": "mock"
        },
        "pluginInfo": {},
        "userData": {}
    })
}

fn player_json(guild: &str) -> Value {
    json!({
        "guildId": guild,
        "track": null,
        "volume": 100,
        "paused": false,
        "state": {"time": 0, "position": 0, "connected": true, "ping": 0},
        "voice": {"token": "t", "endpoint": "e", "sessionId": "s"},
        "filters": {}
    })
}

fn respond(seen: &Seen) -> (u16, Value) {
    match (seen.method.as_str(), seen.path.as_str()) {
        ("GET", p) if p.starts_with("/v4/loadtracks") => {
            (200, json!({"loadType": "track", "data": track_json("abc")}))
        }
        ("GET", p) if p.starts_with("/v4/decodetrack") => (200, track_json("abc")),
        ("POST", "/v4/decodetracks") => (200, json!([track_json("abc"), track_json("def")])),
        ("GET", "/v4/info") => (
            200,
            json!({
                "version": {"semver": "4.0.0", "major": 4, "minor": 0, "patch": 0, "preRelease": null, "build": null},
                "buildTime": 1, "jvm": "17", "lavaplayer": "2",
                "git": {"branch": "main", "commit": "abc", "commitTime": 1},
                "sourceManagers": [], "filters": [], "plugins": []
            }),
        ),
        ("GET", "/v4/stats") => (
            200,
            json!({
                "players": 1, "playingPlayers": 0,
                "uptime": 10,
                "memory": {"free": 1, "used": 1, "allocated": 2, "reservable": 4},
                "cpu": {"cores": 1, "systemLoad": 0.1, "lavalinkLoad": 0.1},
                "frameStats": null
            }),
        ),
        ("GET", "/v4/sessions/s1/players") => (200, json!([player_json("g1")])),
        ("GET", "/v4/sessions/s1/players/g1") => (200, player_json("g1")),
        ("PATCH", "/v4/sessions/s1") => (200, json!({"resuming": true, "timeout": 60})),
        ("PATCH", p) if p.starts_with("/v4/sessions/s1/players/") => {
            (200, player_json("g1"))
        }
        ("DELETE", p) if p.starts_with("/v4/sessions/s1/players/") => (204, Value::Null),
        _ => (404, json!({"error": "not found"})),
    }
}

#[async_std::test]
async fn lavalink_rest_routes() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let log: Log = Arc::default();
    let ws_headers: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
    let (ops_tx, ops_rx) = mpsc::unbounded::<Value>();
    let ops_rx = Arc::new(Mutex::new(Some(ops_rx)));
    {
        let log = log.clone();
        task::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                task::spawn(serve_any(
                    stream,
                    log.clone(),
                    ws_headers.clone(),
                    ops_rx.clone(),
                ));
            }
        });
    }
    let _ = ops_tx;

    let node = LavalinkNode::new(LavalinkOptions {
        url: base,
        password: "pw".to_string(),
        user_id: "42".to_string(),
        reconnect: None,
        ..LavalinkOptions::default()
    })
    .unwrap();
    node.connect().await.unwrap();

    match node.load_tracks("search").await.unwrap() {
        TrackLoadResult::Track(track) => assert_eq!(track.encoded, "abc"),
        other => panic!("expected track, got {other:?}"),
    }
    // second call hits the cache: only one HTTP request logged
    let _ = node.load_tracks("search").await.unwrap();
    assert_eq!(
        log.lock()
            .unwrap()
            .iter()
            .filter(|seen| seen.path.starts_with("/v4/loadtracks"))
            .count(),
        1
    );

    let track = node.decode_track("abc").await.unwrap();
    assert_eq!(track.encoded, "abc");
    let tracks = node
        .decode_tracks(&["abc".to_string(), "def".to_string()])
        .await
        .unwrap();
    assert_eq!(tracks.len(), 2);

    let info = node.info().await.unwrap();
    assert_eq!(info.version.semver, "4.0.0");
    let stats = node.get_stats().await.unwrap();
    assert_eq!(stats.players, 1.0);
    let players = node.get_players().await.unwrap();
    assert_eq!(players[0].guild_id, "g1");

    let mut user_data = flush_music::Extra::new();
    user_data.insert("marker".to_string(), json!("p:1"));
    let player = node
        .update_player(
            "g1",
            MusicPlayerUpdate {
                track: Some(UpdateTrack::Encoded {
                    encoded: Some("abc".to_string()),
                    user_data: Some(user_data),
                }),
                ..MusicPlayerUpdate::default()
            },
            false,
        )
        .await
        .unwrap();
    assert_eq!(player.guild_id, "g1");
    node.destroy_player("g1").await.unwrap();

    let log = log.lock().unwrap();
    assert!(log.iter().all(|seen| seen.auth.as_deref() == Some("pw")));
    let patch = log
        .iter()
        .find(|seen| seen.method == "PATCH" && seen.path == "/v4/sessions/s1/players/g1?noReplace=false")
        .expect("player PATCH recorded");
    let patch_body: Value = serde_json::from_slice(&patch.body).unwrap();
    assert_eq!(patch_body["track"]["userData"]["marker"], json!("p:1"));
    assert!(log
        .iter()
        .any(|seen| seen.method == "DELETE" && seen.path == "/v4/sessions/s1/players/g1"));
}

#[async_std::test]
async fn lavalink_rejects_malformed_url_without_panicking() {
    let options = LavalinkOptions {
        url: "http:/localhost:2333".to_string(),
        password: "pw".to_string(),
        user_id: "1".to_string(),
        ..LavalinkOptions::default()
    };
    assert!(LavalinkNode::new(options).is_err());
}

#[async_std::test]
async fn lavalink_websocket_handshake_and_events() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let log: Log = Arc::default();
    let ws_headers: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
    let (ops_tx, ops_rx) = mpsc::unbounded::<Value>();
    let ops_rx = Arc::new(Mutex::new(Some(ops_rx)));
    {
        let log = log.clone();
        let ws_headers = ws_headers.clone();
        task::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                task::spawn(serve_any(
                    stream,
                    log.clone(),
                    ws_headers.clone(),
                    ops_rx.clone(),
                ));
            }
        });
    }

    let node = LavalinkNode::new(LavalinkOptions {
        url: base,
        password: "pw".to_string(),
        user_id: "42".to_string(),
        client_name: Some("flush-test/1".to_string()),
        reconnect: None,
        ..LavalinkOptions::default()
    })
    .unwrap();
    let events: Arc<Mutex<Vec<MusicEvent>>> = Arc::default();
    let events2 = events.clone();
    let _sub = <LavalinkNode as MusicBackend>::subscribe(&node, Arc::new(move |event| {
        events2.lock().unwrap().push(event);
    }));

    node.connect().await.unwrap();
    assert_eq!(node.session_id().as_deref(), Some("s1"));
    assert_eq!(node.state(), NodeState::Ready);

    // Wait for the session PATCH, then confirm WS headers.
    let patch_seen = {
        let mut found = false;
        for _ in 0..40 {
            task::sleep(std::time::Duration::from_millis(25)).await;
            if log
                .lock()
                .unwrap()
                .iter()
                .any(|seen| seen.method == "PATCH" && seen.path == "/v4/sessions/s1")
            {
                found = true;
                break;
            }
        }
        found
    };
    assert!(patch_seen, "session PATCH not seen");
    let headers = ws_headers.lock().unwrap().clone();
    assert!(headers
        .iter()
        .any(|(k, v)| k == "authorization" && v == "pw"));
    assert!(headers.iter().any(|(k, v)| k == "user-id" && v == "42"));
    assert!(headers
        .iter()
        .any(|(k, v)| k == "client-name" && v == "flush-test/1"));

    // Feed ops: stats + a TrackStart event.
    ops_tx
        .unbounded_send(json!({
            "op": "stats", "players": 3, "playingPlayers": 1, "uptime": 5,
            "memory": {"free": 1, "used": 1, "allocated": 2, "reservable": 4},
            "cpu": {"cores": 1, "systemLoad": 0.0, "lavalinkLoad": 0.0},
            "frameStats": null
        }))
        .unwrap();
    ops_tx
        .unbounded_send(json!({
            "op": "event", "type": "TrackStartEvent", "guildId": "g1",
            "track": track_json("abc")
        }))
        .unwrap();
    let mut got = false;
    for _ in 0..40 {
        task::sleep(std::time::Duration::from_millis(25)).await;
        if events
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, MusicEvent::TrackStart { guild_id, .. } if guild_id == "g1"))
        {
            got = true;
            break;
        }
    }
    assert!(got, "TrackStart event not delivered");
    assert_eq!(node.stats().map(|stats| stats.players), Some(3.0));

    node.close().await;
    assert_eq!(node.state(), NodeState::Closed);
}

/// Replays already-read bytes before the real stream so `accept_async` can
/// re-read the upgrade request we consumed while sniffing the head.
struct Replay {
    buf: Vec<u8>,
    stream: TcpStream,
}

impl futures::AsyncRead for Replay {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buffer: &mut [u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        if self.buf.is_empty() {
            return std::pin::Pin::new(&mut self.stream).poll_read(cx, buffer);
        }
        let take = self.buf.len().min(buffer.len());
        buffer[..take].copy_from_slice(&self.buf[..take]);
        self.buf.drain(..take);
        std::task::Poll::Ready(Ok(take))
    }
}

impl futures::AsyncWrite for Replay {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buffer: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.stream).poll_write(cx, buffer)
    }
    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.stream).poll_flush(cx)
    }
    fn poll_close(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.stream).poll_close(cx)
    }
}

async fn serve_any(
    mut stream: TcpStream,
    log: Log,
    ws_headers: Arc<Mutex<Vec<(String, String)>>>,
    ops_rx: Arc<Mutex<Option<mpsc::UnboundedReceiver<Value>>>>,
) {
    let mut buf = Vec::new();
    loop {
        if find(&buf, b"\r\n\r\n").is_some() {
            break;
        }
        let mut chunk = [0u8; 8192];
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
    let (seen, _, is_ws) = parse_head(&buf);
    if !is_ws {
        // Hand the buffered head to the HTTP loop.
        return serve_http_buffered(stream, buf, log, seen).await;
    }
    // Capture the upgrade headers.
    let head = String::from_utf8_lossy(&buf).to_string();
    let mut headers = Vec::new();
    for line in head.split("\r\n").skip(1) {
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    *ws_headers.lock().unwrap() = headers;
    let replay = Replay { buf, stream };
    let mut ws = match async_tungstenite::accept_async(replay).await {
        Ok(ws) => ws,
        Err(_) => return,
    };
    let Some(ops_rx) = ops_rx.lock().unwrap().take() else {
        return;
    };
    // Node connects -> send the ready op immediately.
    let _ = ws
        .send(async_tungstenite::tungstenite::Message::Text(
            json!({"op": "ready", "sessionId": "s1", "resumed": false})
                .to_string()
                .into(),
        ))
        .await;
    let mut ops_rx = ops_rx;
    while let Some(value) = ops_rx.next().await {
        if ws
            .send(async_tungstenite::tungstenite::Message::Text(
                value.to_string().into(),
            ))
            .await
            .is_err()
        {
            return;
        }
    }
}

async fn serve_http_buffered(stream: TcpStream, buf: Vec<u8>, log: Log, seen: Seen) {
    let _ = seen;
    // The first request was fully buffered: hand the stream to the HTTP loop.
    // `serve_http` reads from the stream; prepend buf by replaying it.
    serve_http_with_prefix(stream, buf, log).await
}

async fn serve_http_with_prefix(mut stream: TcpStream, mut buf: Vec<u8>, log: Log) {
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
        let (mut seen, _, _) = parse_head(&buf);
        let length = {
            let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
            head.split("\r\n")
                .find_map(|line| {
                    let (k, v) = line.split_once(':')?;
                    (k.trim().eq_ignore_ascii_case("content-length"))
                        .then(|| v.trim().parse::<usize>().unwrap_or(0))
                })
                .unwrap_or(0)
        };
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
        let (status, value) = respond(&seen);
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
