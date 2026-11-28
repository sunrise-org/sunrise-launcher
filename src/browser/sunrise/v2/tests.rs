use super::*;
use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::sync::mpsc;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/smod/tests/index_v2")
}

fn load_fixture(name: &str) -> (Vec<u8>, Vec<u8>) {
    let bytes = fs::read(fixtures().join(name)).unwrap();
    let sig = fs::read(fixtures().join(format!("{name}.minisig"))).unwrap();
    (bytes, sig)
}

/// The published `index.signed`: the signature file's four lines, then the index bytes.
fn signed(index: &[u8], sig: &[u8]) -> Vec<u8> {
    [sig, index].concat()
}

fn now_2026_11_24() -> i64 {
    index_v2::parse_rfc3339_utc("2026-11-24T00:00:00Z").unwrap()
}

fn spawn_loopback_server(
    index_bytes: Vec<u8>,
    sig_bytes: Vec<u8>,
) -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let body = signed(&index_bytes, &sig_bytes);
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request_line = String::new();
        reader.read_line(&mut request_line).unwrap();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" || line.is_empty() {
                break;
            }
        }
        let path = request_line.split_whitespace().nth(1).unwrap_or("");
        let (status, body): (&str, &[u8]) = match path {
            "/v2/index.signed" => ("200 OK", &body),
            _ => ("404 Not Found", b"missing"),
        };
        let header = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(header.as_bytes()).unwrap();
        stream.write_all(body).unwrap();
        stream.flush().unwrap();
    });
    (format!("http://{addr}/v2/index.signed"), server)
}

#[test]
fn test_key_constant_matches_fixture_and_good_signature_caches_golden_index() {
    let fixture_key = fs::read_to_string(fixtures().join("test.pub")).unwrap();
    assert_eq!(REGISTRY_KEYS, [fixture_key.trim()].as_slice());

    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join(CACHE_DIR);
    let mut store = Store::new(cache.clone(), REGISTRY_KEYS);

    let (bytes, sig) = load_fixture("two-ports-index.json");
    let catalog = store
        .load_from_fetch(Ok((bytes, sig)), now_2026_11_24())
        .unwrap();
    assert!(!catalog.from_cache);
    assert_eq!(catalog.stale_warning, None);
    assert_eq!(catalog.index.serial, 41);
    assert_eq!(catalog.index.builds.len(), 3);
    assert_eq!(catalog.index.projects.len(), 2);
    assert_eq!(catalog.index.revoked.len(), 2);
    assert_eq!(store.highest_serial().unwrap(), Some(41));

    // Offline load falls back to the verified v2 cache namespace.
    let mut reopened = Store::new(cache, REGISTRY_KEYS);
    let cached = reopened
        .load_from_fetch(Err("offline".into()), now_2026_11_24())
        .unwrap();
    assert!(cached.from_cache);
    assert_eq!(cached.index.serial, 41);
    assert_eq!(cached.index.projects.len(), 2);
}

#[test]
fn without_a_configured_key_the_reader_reports_registry_key_not_configured_and_trusts_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join(CACHE_DIR);

    // Seed a genuinely signed cache copy first.
    let (bytes, sig) = load_fixture("two-ports-index.json");
    let mut seeded = Store::new(cache.clone(), REGISTRY_KEYS);
    seeded
        .load_from_fetch(Ok((bytes.clone(), sig.clone())), now_2026_11_24())
        .unwrap();

    // A store with no configured keys trusts neither the live response nor the disk cache.
    let mut unconfigured = Store::new(cache, &[]);
    let live_err = unconfigured
        .load_from_fetch(Ok((bytes, sig)), now_2026_11_24())
        .unwrap_err();
    assert_eq!(live_err, LoadError::KeyNotConfigured);
    assert!(
        live_err.to_string().contains("registry key not configured"),
        "unexpected error: {live_err}"
    );

    let cache_err = unconfigured.load_cached(now_2026_11_24()).unwrap_err();
    assert_eq!(cache_err, LoadError::KeyNotConfigured);
    assert!(
        cache_err
            .to_string()
            .contains("registry key not configured"),
        "unexpected error: {cache_err}"
    );
}

#[test]
fn bad_signature_and_tampered_bytes_are_refused_without_falling_back_to_cache() {
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join(CACHE_DIR);
    let mut store = Store::new(cache.clone(), REGISTRY_KEYS);

    let (bytes, sig) = load_fixture("two-ports-index.json");
    store
        .load_from_fetch(Ok((bytes.clone(), sig.clone())), now_2026_11_24())
        .unwrap();

    // Wrong key signature is refused directly (does not silently serve the cached copy).
    let other_sig = fs::read(fixtures().join("two-ports-index.other.minisig")).unwrap();
    let err = store
        .load_from_fetch(Ok((bytes.clone(), other_sig)), now_2026_11_24())
        .unwrap_err();
    assert!(matches!(err, LoadError::BadSignature(_)), "{err:?}");

    // Tampered index bytes are refused directly.
    let mut tampered = bytes;
    tampered[0] ^= 0x01;
    let err = store
        .load_from_fetch(Ok((tampered, sig)), now_2026_11_24())
        .unwrap_err();
    assert!(matches!(err, LoadError::BadSignature(_)), "{err:?}");

    // Tampered disk cache is never trusted offline either.
    let (active_index, _) = store.active_cache_paths().unwrap().unwrap();
    fs::write(active_index, b"{}").unwrap();
    let err = store.load_cached(now_2026_11_24()).unwrap_err();
    assert!(matches!(err, LoadError::BadSignature(_)), "{err:?}");
}

#[test]
fn lower_serial_is_refused_for_both_fetched_and_cached_copies() {
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join(CACHE_DIR);
    let mut store = Store::new(cache.clone(), REGISTRY_KEYS);

    let (bytes_41, sig_41) = load_fixture("two-ports-index.json");
    let (bytes_40, sig_40) = load_fixture("serial-40.json");
    let (bytes_42, sig_42) = load_fixture("serial-42.json");

    // Accept serial 41.
    store
        .load_from_fetch(Ok((bytes_41.clone(), sig_41.clone())), now_2026_11_24())
        .unwrap();

    // Refuse lower serial 40 from network.
    let err = store
        .load_from_fetch(Ok((bytes_40.clone(), sig_40.clone())), now_2026_11_24())
        .unwrap_err();
    assert_eq!(
        err,
        LoadError::Rollback {
            got: 40,
            highest: 41
        }
    );

    // Equal serial 41 is still accepted.
    let same = store
        .load_from_fetch(Ok((bytes_41, sig_41)), now_2026_11_24())
        .unwrap();
    assert_eq!(same.index.serial, 41);

    // Higher serial 42 advances the persisted high-water mark.
    let newer = store
        .load_from_fetch(Ok((bytes_42, sig_42)), now_2026_11_24())
        .unwrap();
    assert_eq!(newer.index.serial, 42);
    assert_eq!(
        newer.index.project("Ab3dE9xQ").unwrap().releases[0].workflow,
        "sunrise-org/sunrise-mods/.github/workflows/release.yml@refs/tags/release-v2"
    );

    // Replace active cached files with genuinely signed older serial 40; offline load MUST refuse.
    let (active_index, active_sig) = store.active_cache_paths().unwrap().unwrap();
    fs::write(active_index, &bytes_40).unwrap();
    fs::write(active_sig, &sig_40).unwrap();
    let mut reopened = Store::new(cache, REGISTRY_KEYS);
    let cache_err = reopened
        .load_from_fetch(Err("offline".into()), now_2026_11_24())
        .unwrap_err();
    assert_eq!(
        cache_err,
        LoadError::Rollback {
            got: 40,
            highest: 42
        }
    );
}

#[test]
fn overlapping_refreshes_keep_fetch_outside_lock_and_cannot_roll_back_disk_cache() {
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join(CACHE_DIR);
    let loaded: Arc<Mutex<Option<LoadedState>>> = Arc::new(Mutex::new(None));

    // Seed with serial 40 first.
    let (bytes_40, sig_40) = load_fixture("serial-40.json");
    let seed_ctx = RefreshContext {
        cache_dir: &cache,
        keys: REGISTRY_KEYS,
        source: "https://example.org/v2/index.json",
        force_refresh: true,
        now_unix: now_2026_11_24(),
    };
    let seeded = refresh_in(&loaded, &seed_ctx, |_| Ok((bytes_40, sig_40))).unwrap();
    assert_eq!(seeded.index.serial, 40);

    // Start a slower refresh returning serial 41; hold it inside fetch while a faster refresh
    // commits serial 42. Reaching the fast commit proves fetching stays outside the lock.
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let slow_loaded = Arc::clone(&loaded);
    let slow_cache = cache.clone();
    let slow = std::thread::spawn(move || {
        let (bytes_41, sig_41) = load_fixture("two-ports-index.json");
        let ctx = RefreshContext {
            cache_dir: &slow_cache,
            keys: REGISTRY_KEYS,
            source: "https://example.org/v2/index.json",
            force_refresh: true,
            now_unix: now_2026_11_24(),
        };
        refresh_in(&slow_loaded, &ctx, |_| {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok((bytes_41, sig_41))
        })
    });

    entered_rx.recv().unwrap();

    // Fast refresh completes and persists serial 42 while the slow fetch is still in flight.
    let (bytes_42, sig_42) = load_fixture("serial-42.json");
    let fast_ctx = RefreshContext {
        cache_dir: &cache,
        keys: REGISTRY_KEYS,
        source: "https://example.org/v2/index.json",
        force_refresh: true,
        now_unix: now_2026_11_24(),
    };
    let fast = refresh_in(&loaded, &fast_ctx, |_| Ok((bytes_42, sig_42))).unwrap();
    assert_eq!(fast.index.serial, 42);

    // Let the slower serial-41 fetch return; the serialized floor check must reject it without
    // touching the on-disk serial-42 catalogue.
    release_tx.send(()).unwrap();
    let slow_err = slow.join().unwrap().unwrap_err();
    assert_eq!(
        slow_err,
        LoadError::Rollback {
            got: 41,
            highest: 42
        }
    );

    // Simulate a restart: a fresh Store must still load serial 42 from disk.
    let mut reopened = Store::new(cache, REGISTRY_KEYS);
    assert_eq!(reopened.highest_serial().unwrap(), Some(42));
    let cached = reopened.load_cached(now_2026_11_24()).unwrap();
    assert_eq!(cached.index.serial, 42);
    assert!(cached.from_cache);
}

#[test]
fn interrupted_update_preserves_last_good_cached_copy() {
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join(CACHE_DIR);
    let mut store = Store::new(cache.clone(), REGISTRY_KEYS);

    // Seed last-good serial 41.
    let (bytes_41, sig_41) = load_fixture("two-ports-index.json");
    store
        .load_from_fetch(Ok((bytes_41, sig_41)), now_2026_11_24())
        .unwrap();
    let (good_index, good_sig) = store.active_cache_paths().unwrap().unwrap();

    let (bytes_42, sig_42_bytes) = load_fixture("serial-42.json");
    let sig_42 = String::from_utf8(sig_42_bytes).unwrap();

    // Interrupt an update to serial 42 at every stage of publication; the last-good serial-41
    // snapshot and state must remain intact after each failure.
    for fail_at in [
        PublishStep::Index,
        PublishStep::Signature,
        PublishStep::State,
    ] {
        let err = store
            .accept_bytes_with_writer(&bytes_42, &sig_42, now_2026_11_24(), |step, path, data| {
                if step == fail_at {
                    Err(format!("simulated write failure at {step:?}"))
                } else {
                    write_atomic(path, data)
                }
            })
            .unwrap_err();
        assert!(matches!(err, LoadError::Fetch(_)), "{fail_at:?}: {err:?}");

        let mut reopened = Store::new(cache.clone(), REGISTRY_KEYS);
        assert_eq!(reopened.highest_serial().unwrap(), Some(41));
        assert_eq!(
            reopened.active_cache_paths().unwrap(),
            Some((good_index.clone(), good_sig.clone()))
        );
        let cached = reopened.load_cached(now_2026_11_24()).unwrap();
        assert!(cached.from_cache);
        assert_eq!(cached.index.serial, 41);
        assert_eq!(cached.index.projects.len(), 2);

        // No orphaned uncommitted snapshot files are left behind.
        let entries: Vec<_> = fs::read_dir(&cache)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(entries.len(), 3, "{fail_at:?}: {entries:?}");
    }

    // A subsequent uninterrupted update commits serial 42 and removes the superseded snapshot.
    let updated = store
        .accept_bytes(&bytes_42, &sig_42, now_2026_11_24())
        .unwrap();
    assert_eq!(updated.index.serial, 42);
    assert!(!good_index.exists());
    assert!(!good_sig.exists());
    let mut reopened = Store::new(cache, REGISTRY_KEYS);
    assert_eq!(
        reopened.load_cached(now_2026_11_24()).unwrap().index.serial,
        42
    );
}

#[test]
fn non_utf8_signature_is_bad_signature_without_cache_fallback_on_http_and_local_file() {
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join(CACHE_DIR);
    let mut store = Store::new(cache.clone(), REGISTRY_KEYS);

    // Seed a valid last-good cached copy first.
    let (bytes, sig) = load_fixture("two-ports-index.json");
    store
        .load_from_fetch(Ok((bytes.clone(), sig)), now_2026_11_24())
        .unwrap();

    let non_utf8_sig = b"\xff\n\xfe\n\xfd\n\x80\n".to_vec();

    // A local file whose signature lines aren't UTF-8 must return BadSignature, not cached serial 41.
    let local_index = temp.path().join("index.signed");
    fs::write(&local_index, signed(&bytes, &non_utf8_sig)).unwrap();
    let local_err = store
        .fetch_and_load(local_index.to_str().unwrap(), now_2026_11_24())
        .unwrap_err();
    assert!(
        matches!(local_err, LoadError::BadSignature(_)),
        "expected BadSignature for local non-UTF-8 sig, got {local_err:?}"
    );

    // HTTP path with non-UTF-8 `.minisig` must also return BadSignature, not cached serial 41.
    let (url, server) = spawn_loopback_server(bytes, non_utf8_sig);
    let http_err = store.fetch_and_load(&url, now_2026_11_24()).unwrap_err();
    server.join().unwrap();
    assert!(
        matches!(http_err, LoadError::BadSignature(_)),
        "expected BadSignature for HTTP non-UTF-8 sig, got {http_err:?}"
    );

    // The last-good cached copy remains intact for genuine offline fallback.
    let cached = store.load_cached(now_2026_11_24()).unwrap();
    assert!(cached.from_cache);
    assert_eq!(cached.index.serial, 41);
}

#[test]
fn stale_generated_timestamp_warns_without_failing() {
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join(CACHE_DIR);
    let mut store = Store::new(cache, REGISTRY_KEYS);

    let (bytes, sig) = load_fixture("stale.json");
    let catalog = store
        .load_from_fetch(Ok((bytes, sig)), now_2026_11_24())
        .unwrap();
    assert_eq!(catalog.index.serial, 41);
    let warning = catalog
        .stale_warning
        .expect("stale generated timestamp must produce a warning");
    assert!(warning.contains("2025-01-01T00:00:00Z"), "{warning}");
}

#[test]
fn malformed_signed_indexes_are_refused_with_no_salvage() {
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join(CACHE_DIR);
    let mut store = Store::new(cache.clone(), REGISTRY_KEYS);

    for fixture in [
        "malformed-json.json",
        "malformed-schema.json",
        "malformed-missing-revoked.json",
        "malformed-empty-releases.json",
        "malformed-bad-generated.json",
    ] {
        let (bytes, sig) = load_fixture(fixture);
        let err = store
            .load_from_fetch(Ok((bytes, sig)), now_2026_11_24())
            .unwrap_err();
        assert!(
            matches!(err, LoadError::Malformed(_)),
            "{fixture}: expected Malformed, got {err:?}"
        );
    }
    assert!(!cache.join(STATE_FILE).exists());
    assert_eq!(store.highest_serial().unwrap(), None);
}

#[test]
fn fetch_and_load_verifies_local_files_and_loopback_http() {
    // A local file override is one signed file too.
    let temp = tempfile::tempdir().unwrap();
    let (bytes_40, sig_40) = load_fixture("serial-40.json");
    let local_index = temp.path().join("index.signed");
    fs::write(&local_index, signed(&bytes_40, &sig_40)).unwrap();
    let mut local_store = Store::new(temp.path().join("local-cache"), REGISTRY_KEYS);
    let local_catalog = local_store
        .fetch_and_load(local_index.to_str().unwrap(), now_2026_11_24())
        .unwrap();
    assert_eq!(local_catalog.index.serial, 40);
    assert_eq!(
        local_catalog.index.project("Ab3dE9xQ").unwrap().releases[0].workflow,
        "sunrise-org/sunrise-mods/.github/workflows/release.yml@refs/tags/release-v1"
    );

    // A loopback HTTP server serves `index.signed`.
    let (index_bytes, sig_bytes) = load_fixture("two-ports-index.json");
    let (url, server) = spawn_loopback_server(index_bytes, sig_bytes);
    let mut http_store = Store::new(temp.path().join("http-cache"), REGISTRY_KEYS);
    let http_catalog = http_store.fetch_and_load(&url, now_2026_11_24()).unwrap();
    server.join().unwrap();
    assert!(!http_catalog.from_cache);
    assert_eq!(http_catalog.index.serial, 41);
    assert_eq!(http_catalog.index.projects.len(), 2);
}

#[test]
fn a_signed_file_without_four_signature_lines_is_a_fetch_error() {
    assert_eq!(
        split_signed(b"one\ntwo\nthree\n".to_vec()).unwrap_err(),
        "The mod registry file has no signature."
    );
    let (index, sig) = split_signed(b"a\nb\nc\nd\n{\"x\":\n1}".to_vec()).unwrap();
    assert_eq!(
        (index.as_slice(), sig.as_slice()),
        (&b"{\"x\":\n1}"[..], &b"a\nb\nc\nd\n"[..])
    );
}

#[test]
fn the_production_key_verifies_the_first_published_index() {
    let (bytes, sig) = load_fixture("first-published.json");
    let index = sunrise_launcher_smod::index_v2::verify_and_parse(
        &bytes,
        std::str::from_utf8(&sig).unwrap(),
        &[PRODUCTION_KEY],
    )
    .unwrap();
    assert_eq!(index.serial, 1795408124);
    assert!(index.build("dev-2026-11-22-7cfdf81").is_some());
}

#[test]
fn a_damaged_snapshot_pointer_drops_the_cache_but_keeps_the_serial_floor() {
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join(CACHE_DIR);
    let mut store = Store::new(cache.clone(), REGISTRY_KEYS);
    let (bytes_42, sig_42) = load_fixture("serial-42.json");
    store
        .load_from_fetch(Ok((bytes_42.clone(), sig_42.clone())), now_2026_11_24())
        .unwrap();

    for active in [
        // Would derive the state file's own name.
        r#"{"snapshot":"state"}"#,
        // An earlier build's pointer shape.
        r#"{"index":"snapshot-42-0000000000000000.json","signature":"x.minisig"}"#,
    ] {
        fs::write(
            cache.join(STATE_FILE),
            format!(r#"{{"highest_serial":42,"active":{active}}}"#),
        )
        .unwrap();
        let mut reopened = Store::new(cache.clone(), REGISTRY_KEYS);
        assert!(reopened.load_cached(now_2026_11_24()).is_err());
        assert!(cache.join(STATE_FILE).exists());

        // The floor still refuses an older index, and a current one replaces the pointer.
        let (bytes_41, sig_41) = load_fixture("two-ports-index.json");
        assert!(matches!(
            reopened.load_from_fetch(Ok((bytes_41, sig_41)), now_2026_11_24()),
            Err(LoadError::Rollback {
                got: 41,
                highest: 42
            })
        ));
        let mut fresh = Store::new(cache.clone(), REGISTRY_KEYS);
        fresh
            .load_from_fetch(Ok((bytes_42.clone(), sig_42.clone())), now_2026_11_24())
            .unwrap();
        assert!(fresh.load_cached(now_2026_11_24()).is_ok());
    }
}

fn change_context(cache: &Path, force_refresh: bool) -> RefreshContext<'_> {
    RefreshContext {
        cache_dir: cache,
        keys: REGISTRY_KEYS,
        source: "https://example.org/v2/index.json",
        force_refresh,
        now_unix: now_2026_11_24(),
    }
}

#[test]
fn a_change_within_the_freshness_window_reuses_the_verified_index_without_fetching() {
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join(CACHE_DIR);
    let loaded = Mutex::new(None);
    let (bytes, sig) = load_fixture("two-ports-index.json");
    refresh_in(&loaded, &change_context(&cache, false), |_| {
        Ok((bytes, sig))
    })
    .unwrap();

    let catalog = change_index_in(&loaded, &change_context(&cache, false), |_| {
        panic!("an index verified within the window must not be fetched again")
    })
    .unwrap();
    assert_eq!(catalog.index.serial, 41);
    assert!(!catalog.from_cache);
}

#[test]
fn a_change_whose_refetch_fails_uses_the_last_verified_index() {
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join(CACHE_DIR);
    let loaded = Mutex::new(None);
    let (bytes, sig) = load_fixture("two-ports-index.json");
    refresh_in(&loaded, &change_context(&cache, false), |_| {
        Ok((bytes.clone(), sig))
    })
    .unwrap();

    // Past the window the index is fetched again, and this copy's signature is for other bytes:
    // the index and signature of two publishes, as the registry site can serve them.
    let other_sig = fs::read(fixtures().join("two-ports-index.other.minisig")).unwrap();
    let mismatched = |_: &str| Ok((bytes.clone(), other_sig.clone()));
    let catalog = change_index_in(&loaded, &change_context(&cache, true), mismatched).unwrap();
    assert_eq!(catalog.index.serial, 41);
    assert!(catalog.from_cache);

    // After a restart only the copy on disk is left, and it serves the same way.
    let restarted = Mutex::new(None);
    let catalog = change_index_in(&restarted, &change_context(&cache, true), mismatched).unwrap();
    assert_eq!(catalog.index.serial, 41);
    assert!(catalog.from_cache);
}

/// Reading the loaded index's serial never waits on `LOADED`, which a refresh holds while it
/// writes the verified index to disk, so a frame that asks for it (Home's suggestion, the
/// picker's preview) doesn't stall on that write (review of #616).
#[test]
fn the_loaded_serial_is_read_without_the_refresh_lock() {
    let held = LOADED.lock().unwrap_or_else(|e| e.into_inner());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(loaded_serial());
    });
    let answer = rx.recv_timeout(Duration::from_secs(2));
    drop(held);
    assert!(answer.is_ok(), "loaded_serial waited on the refresh lock");
}

/// The serial reads back exactly across the whole `u64` range the index's schema allows: a
/// refresh from the second-highest serial to the highest is seen as a change, and an answer a
/// worker records at the highest serial still matches it afterwards, so the redraw that follows
/// keeps it rather than starting the check again (review of #671).
#[test]
fn the_loaded_serial_reads_back_the_top_two_serials_exactly() {
    publish_loaded_serial(u64::MAX - 1);
    let before = loaded_serial();
    assert_eq!(before, Some(u64::MAX - 1));
    publish_loaded_serial(u64::MAX);
    assert_ne!(
        loaded_serial(),
        before,
        "the refresh to the top serial is a change"
    );
    // What Home's lookup and the picker's preview record with an answer at this index.
    let recorded = Some(u64::MAX);
    assert_eq!(
        loaded_serial(),
        recorded,
        "an answer at the top serial stays current, so its redraw doesn't start the check again"
    );
}
