//! MusicManager / MusicPlayer / TrackQueue over MockMusicBackend (loopback only).

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_std::task;

use flush_music::{
    mock_track, LoopMode, MockMusicBackend, MusicBackend, MusicEvent, MusicManager, MusicManagerOptions,
    MusicNodeEvent, MusicPlayerOptions, TrackEndReason, TrackLoadResult, TrackQueue, UpdateTrack,
    PLAYBACK_MARKER,
};

fn manager(node: Arc<MockMusicBackend>) -> MusicManager {
    MusicManager::new(MusicManagerOptions {
        nodes: vec![node],
        ..MusicManagerOptions::default()
    })
    .unwrap()
}

fn marker_of(node: &MockMusicBackend, update_index: usize) -> String {
    let updates = node.updates.lock().unwrap();
    let update = &updates[update_index].update;
    match &update.track {
        Some(UpdateTrack::Encoded {
            user_data: Some(user_data),
            ..
        }) => user_data
            .get(PLAYBACK_MARKER)
            .and_then(|value| value.as_str())
            .unwrap()
            .to_string(),
        other => panic!("expected encoded track update, got {other:?}"),
    }
}

fn ended(identifier: &str, marker: &str, reason: TrackEndReason) -> MusicEvent {
    let mut track = mock_track(identifier);
    track
        .user_data
        .insert(PLAYBACK_MARKER.to_string(), marker.into());
    MusicEvent::TrackEnd {
        guild_id: "g1".to_string(),
        track,
        reason,
    }
}

fn last_encoded(node: &MockMusicBackend) -> Option<String> {
    let updates = node.updates.lock().unwrap();
    match &updates.last().unwrap().update.track {
        Some(UpdateTrack::Encoded { encoded, .. }) => encoded.clone(),
        _ => None,
    }
}

#[async_std::test]
async fn track_queue_operations() {
    let mut queue: TrackQueue<u32> = TrackQueue::new(3).unwrap();
    assert!(TrackQueue::<u32>::new(0).is_err());
    queue.enqueue(1).unwrap();
    queue.add([2, 3]).unwrap();
    assert_eq!(queue.enqueue(4).unwrap_err().code, "MUSIC_QUEUE_LIMIT");
    assert_eq!(queue.at(1).unwrap(), Some(&2));
    assert_eq!(queue.slice(0, 2).unwrap(), vec![1, 2]);
    queue.insert(0, [9]).unwrap_err();
    queue.move_item(2, 0).unwrap();
    assert_eq!(queue.to_vec(), vec![3, 1, 2]);
    assert_eq!(queue.remove(1, 1).unwrap(), vec![1]);
    queue.shuffle(|| 0.5).unwrap();
    assert_eq!(queue.shuffle(|| 2.0).unwrap_err().code, "MUSIC_QUEUE_RANDOM");
    assert_eq!(queue.dequeue(), Some(3));
    assert_eq!(queue.dequeue(), Some(2));
    assert_eq!(queue.dequeue(), None);
}

#[async_std::test]
async fn player_lifecycle_and_marked_updates() {
    let node = MockMusicBackend::new();
    let manager = manager(node.clone());
    let player = manager
        .create_player("g1", MusicPlayerOptions::default())
        .unwrap();
    assert!(Arc::ptr_eq(
        &manager
            .create_player("g1", MusicPlayerOptions::default())
            .unwrap(),
        &player
    ));

    let events = Arc::new(Mutex::new(Vec::new()));
    let events2 = events.clone();
    let _subscription = player.subscribe(Arc::new(move |event| {
        events2.lock().unwrap().push(format!("{event:?}"));
    }));

    player
        .enqueue(vec![mock_track("t1"), mock_track("t2")])
        .unwrap();
    let played = player.play().await.unwrap().unwrap();
    assert_eq!(played.info.identifier, "t1");
    assert_eq!(player.queue_size(), 1);
    assert_eq!(player.current().unwrap().info.identifier, "t1");

    let marker = marker_of(&node, 0);
    assert!(!marker.is_empty());

    // finished -> auto-advance to t2
    node.emit(ended("t1", &marker, TrackEndReason::Finished));
    player.drain().await;
    assert_eq!(last_encoded(&node).as_deref(), Some("t2"));
    assert!(events
        .lock()
        .unwrap()
        .iter()
        .any(|event| event.starts_with("TrackEnd")));

    // skip -> queue empty -> null track update (stop)
    player.skip().await.unwrap();
    player.drain().await;
    assert_eq!(last_encoded(&node), None);

    player.enqueue(vec![mock_track("t3")]).unwrap();
    player.play().await.unwrap();
    let update_count = node.updates.lock().unwrap().len();
    let marker = marker_of(&node, update_count - 1);

    // unrelated markers are ignored
    node.emit(ended("t3", "other:9", TrackEndReason::Finished));
    player.drain().await;
    assert_eq!(node.updates.lock().unwrap().len(), 4);

    // repeat track restarts the same track on finished
    player.set_loop(LoopMode::Track).unwrap();
    node.emit(ended("t3", &marker, TrackEndReason::Finished));
    player.drain().await;
    assert_eq!(last_encoded(&node).as_deref(), Some("t3"));
    player.set_loop(LoopMode::Off).unwrap();

    player.pause(true).await.unwrap();
    assert!(player.is_paused());
    player.set_volume(50).await.unwrap();
    assert_eq!(player.volume(), 50);
    player.drain().await;
    assert_eq!(node.updates.lock().unwrap().last().unwrap().update.volume, Some(50));
    player.set_volume(1001).await.unwrap_err();
    player.seek(10).await.unwrap();
    player.drain().await;
    assert_eq!(node.updates.lock().unwrap().last().unwrap().update.position, Some(10));

    let failures = player.destroy().await;
    assert!(failures.is_empty());
    assert_eq!(manager.len(), 0);
    assert!(node.destroyed.lock().unwrap().contains(&"g1".to_string()));
}

#[async_std::test]
async fn node_selection_prefers_lower_load() {
    let node_a = MockMusicBackend::new();
    let node_b = MockMusicBackend::new();
    *node_a.penalty.lock().unwrap() = 50.0;
    let manager = MusicManager::new(MusicManagerOptions {
        nodes: vec![node_a.clone(), node_b.clone()],
        ..MusicManagerOptions::default()
    })
    .unwrap();
    let player = manager
        .create_player("g1", MusicPlayerOptions::default())
        .unwrap();
    assert!(Arc::ptr_eq(player.node(), &(node_b.clone() as Arc<dyn flush_music::MusicBackend>)));
    // Second player evens the counts; node_b still holds one player.
    let player2 = manager
        .create_player("g2", MusicPlayerOptions::default())
        .unwrap();
    // node_a's penalty outweighs node_b's single player, so node_b wins again.
    assert!(Arc::ptr_eq(player2.node(), &(node_b.clone() as Arc<dyn flush_music::MusicBackend>)));
}

#[async_std::test]
async fn search_and_capacity() {
    let node = MockMusicBackend::new();
    node.results
        .lock()
        .unwrap()
        .insert("q".to_string(), TrackLoadResult::Track(mock_track("t")));
    let manager = MusicManager::new(MusicManagerOptions {
        nodes: vec![node.clone()],
        max_players: 1,
        ..MusicManagerOptions::default()
    })
    .unwrap();
    match manager.search("q").await.unwrap() {
        TrackLoadResult::Track(track) => assert_eq!(track.info.identifier, "t"),
        other => panic!("expected track, got {other:?}"),
    }
    manager
        .create_player("g1", MusicPlayerOptions::default())
        .unwrap();
    let error = match manager.create_player("g2", MusicPlayerOptions::default()) {
        Err(error) => error,
        Ok(_) => panic!("expected capacity error"),
    };
    assert_eq!(error.code, "MUSIC_CAPACITY");
    assert!(manager.destroy_player("g1").await.unwrap());
    assert!(!manager.destroy_player("g1").await.unwrap());
}

#[async_std::test]
async fn recovery_after_fresh_session() {
    let node = MockMusicBackend::new();
    let errors = Arc::new(AtomicUsize::new(0));
    let errors2 = errors.clone();
    let manager = MusicManager::new(MusicManagerOptions {
        nodes: vec![node.clone()],
        on_error: Some(Arc::new(move |_, _| {
            errors2.fetch_add(1, Ordering::SeqCst);
        })),
        ..MusicManagerOptions::default()
    })
    .unwrap();
    let player = manager
        .create_player("g1", MusicPlayerOptions::default())
        .unwrap();
    player.enqueue(vec![mock_track("t1")]).unwrap();
    player.play().await.unwrap();

    // A new (non-resumed) session invalidates and recovers the player.
    node.emit_lifecycle(MusicNodeEvent::Ready {
        session_id: "s2".to_string(),
        resumed: false,
    });
    let mut recovered = false;
    for _ in 0..40 {
        task::sleep(Duration::from_millis(50)).await;
        if !player.is_recovering() && node.updates.lock().unwrap().len() >= 2 {
            recovered = true;
            break;
        }
    }
    assert!(recovered, "player did not recover");
    assert_eq!(errors.load(Ordering::SeqCst), 0);
    let update = node.updates.lock().unwrap().last().unwrap().update.clone();
    assert!(update.track.is_some());
    assert!(update.volume.is_some() || update.paused.is_some());

    manager.close().await.unwrap();
    assert_eq!(manager.len(), 0);
}

#[async_std::test]
async fn early_track_event_during_update_is_accepted() {
    let node = MockMusicBackend::new();
    let manager = manager(node.clone());
    let player = manager
        .create_player("g1", MusicPlayerOptions::default())
        .unwrap();
    player
        .enqueue(vec![mock_track("t1"), mock_track("t2")])
        .unwrap();

    // Lavalink may deliver a track event while the PATCH is still in flight.
    let emitter = node.clone();
    let fired = Arc::new(AtomicBool::new(false));
    let flag = fired.clone();
    *node.before_update.lock().unwrap() = Some(Arc::new(move |_guild, update| {
        if flag.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Some(UpdateTrack::Encoded {
            user_data: Some(user_data),
            ..
        }) = &update.track
        {
            let marker = user_data
                .get(PLAYBACK_MARKER)
                .and_then(|value| value.as_str())
                .unwrap()
                .to_string();
            emitter.emit(ended("t1", &marker, TrackEndReason::LoadFailed));
        }
    }));
    player.play().await.unwrap();
    player.drain().await;
    // The early LoadFailed advanced the queue to t2.
    assert_eq!(last_encoded(&node).as_deref(), Some("t2"));
    assert_eq!(player.current().unwrap().info.identifier, "t2");
}

#[async_std::test]
async fn dropped_reply_futures_do_not_leak_control_slots() {
    let node = MockMusicBackend::new();
    let manager = manager(node.clone());
    let player = manager
        .create_player(
            "g1",
            MusicPlayerOptions {
                max_pending_controls: Some(1),
                ..MusicPlayerOptions::default()
            },
        )
        .unwrap();
    drop(player.pause(true));
    drop(player.set_volume(20));
    player.drain().await;
    player.set_volume(50).await.unwrap();
    player.skip().await.unwrap();
    player.destroy().await;
}

#[async_std::test]
async fn partial_queue_pages_and_reentrant_emit() {
    let node = MockMusicBackend::new();
    let manager = manager(node.clone());
    let player = manager
        .create_player("g1", MusicPlayerOptions::default())
        .unwrap();
    player
        .enqueue(vec![mock_track("t1"), mock_track("t2")])
        .unwrap();
    // A page larger than the queue returns what is queued, not an error.
    assert_eq!(player.queue_page(0, 10).unwrap().len(), 2);
    assert_eq!(player.queue_page(1, 10).unwrap().len(), 1);
    assert!(player.queue_page(5, 10).unwrap().is_empty());

    // A listener may emit again (or unsubscribe) from inside a dispatch.
    let emitter = node.clone();
    let hits = Arc::new(AtomicUsize::new(0));
    let once = Arc::new(AtomicBool::new(false));
    let hits_in_listener = hits.clone();
    let _subscription = MusicBackend::subscribe(
        node.as_ref(),
        Arc::new(move |_event| {
            hits_in_listener.fetch_add(1, Ordering::SeqCst);
            if !once.swap(true, Ordering::SeqCst) {
                emitter.emit(ended("t2", "m:2", TrackEndReason::Finished));
            }
        }),
    );
    node.emit(ended("t1", "m:1", TrackEndReason::Finished));
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}
