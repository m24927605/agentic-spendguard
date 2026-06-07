//! D18 SLICE 80 — fixture replay harness integration tests.
//!
//! Each test loads a committed `.windsurf-rpc` fixture under
//! `fixtures/synthetic/`, replays it through
//! [`spendguard_windsurf_codec::replay::replay_fixture`], and asserts
//! the layered pipeline (framing decode → envelope decode →
//! version gate → translation → mock sidecar reserve+commit →
//! byte-for-byte preservation) lands per the fixture's expected
//! report.
//!
//! Per D18 design.md §3 decision 7: no live `server.codeium.com`
//! traffic. The harness is offline against committed bytes.
//!
//! Per D18 design.md §3 redaction policy: a regression test
//! (`no_secret_leakage_in_fixtures`) gates every committed
//! fixture against known Codeium credential prefixes.

use std::path::{Path, PathBuf};

use spendguard_windsurf_codec::{replay_fixture, Direction};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/synthetic")
}

fn fixture(name: &str) -> PathBuf {
    fixtures_dir().join(format!("{name}.windsurf-rpc"))
}

fn assert_fixture_under_size_cap(path: &Path) {
    let bytes = std::fs::read(path).expect("read fixture");
    assert!(
        bytes.len() <= 64 * 1024,
        "fixture {} is {} bytes > 64 KiB cap",
        path.display(),
        bytes.len()
    );
}

// ============================================================================
// (1) Simple happy path
// ============================================================================

#[test]
fn replay_cascade_chat_simple_lands_full_cycle() {
    let path = fixture("cascade_chat_simple");
    assert_fixture_under_size_cap(&path);
    let report = replay_fixture(&path).expect("replay simple");

    assert_eq!(report.version, 1);
    assert_eq!(report.request_frames_decoded, 1);
    assert_eq!(report.response_frames_decoded, 3);
    assert_eq!(report.end_of_stream_frames, 1);
    assert_eq!(report.finish_reason.as_deref(), Some("stop"));
    assert_eq!(report.cumulative_output_tokens, Some(18));
    assert_eq!(report.sidecar_reserve_calls, 1);
    assert_eq!(report.sidecar_commit_calls, 1);
    assert!(!report.upstream_error);
    assert!(report.request_bytes_round_trip);
    assert!(report.all_frames_round_trip);
    assert!(!report.unsupported_wire_version_seen);
    assert!(!report.decoder_skipped);

    let translated = report.translated_request.as_ref().expect("translated");
    assert_eq!(translated.model, "gpt-4o");
    assert_eq!(translated.messages.len(), 1);
    assert_eq!(translated.messages[0].role, "user");

    let decoded = report.decoded_requests.first().expect("decoded request");
    assert_eq!(decoded.max_tokens, Some(64));
    assert_eq!(
        decoded.cascade_wire_version.as_deref(),
        Some("cascade.v2.0")
    );
}

// ============================================================================
// (2) Tool calls (Cascade Agent mode)
// ============================================================================

#[test]
fn replay_cascade_chat_with_tools_records_tool_calls_finish_reason() {
    let path = fixture("cascade_chat_with_tools");
    assert_fixture_under_size_cap(&path);
    let report = replay_fixture(&path).expect("replay with_tools");

    assert_eq!(report.request_frames_decoded, 1);
    assert_eq!(report.response_frames_decoded, 1);
    assert_eq!(report.finish_reason.as_deref(), Some("tool_calls"));
    assert_eq!(report.cumulative_output_tokens, Some(9));
    assert_eq!(report.sidecar_reserve_calls, 1);
    assert_eq!(
        report.sidecar_commit_calls, 1,
        "tool_calls finish is still a successful upstream → commits"
    );
    assert!(!report.upstream_error);
    assert!(report.all_frames_round_trip);
    assert!(!report.unsupported_wire_version_seen);
    assert!(!report.decoder_skipped);

    let decoded = report.decoded_requests.first().expect("decoded request");
    assert_eq!(decoded.tool_declarations.len(), 2);
    assert_eq!(decoded.tool_declarations[0].name, "read_file");
    assert_eq!(decoded.tool_declarations[1].name, "list_dir");
    assert_eq!(
        decoded.cascade_wire_version.as_deref(),
        Some("cascade.v2.1")
    );
}

// ============================================================================
// (3) Long streaming
// ============================================================================

#[test]
fn replay_cascade_chat_streaming_handles_long_stream() {
    let path = fixture("cascade_chat_streaming");
    assert_fixture_under_size_cap(&path);
    let report = replay_fixture(&path).expect("replay streaming");

    assert_eq!(report.request_frames_decoded, 1);
    assert_eq!(report.response_frames_decoded, 9);
    assert_eq!(report.finish_reason.as_deref(), Some("stop"));
    assert_eq!(report.cumulative_output_tokens, Some(47));
    assert_eq!(report.sidecar_reserve_calls, 1);
    assert_eq!(report.sidecar_commit_calls, 1);
    assert!(report.all_frames_round_trip);

    let decoded = report.decoded_requests.first().expect("decoded request");
    assert_eq!(decoded.messages.len(), 4);
    assert_eq!(decoded.max_tokens, Some(1024));
}

// ============================================================================
// (4) Error path (no commit)
// ============================================================================

#[test]
fn replay_cascade_chat_error_short_circuits_commit() {
    let path = fixture("cascade_chat_error");
    assert_fixture_under_size_cap(&path);
    let report = replay_fixture(&path).expect("replay error");

    assert_eq!(report.request_frames_decoded, 1);
    assert_eq!(report.response_frames_decoded, 0);
    assert!(report.upstream_error);
    assert_eq!(report.sidecar_reserve_calls, 1);
    assert_eq!(
        report.sidecar_commit_calls, 0,
        "no commit on upstream grpc-status:13"
    );
    assert!(report.all_frames_round_trip);
}

// ============================================================================
// (5) Unknown wire version
// ============================================================================

#[test]
fn replay_cascade_chat_unknown_wire_version_flagged() {
    let path = fixture("cascade_chat_unknown_wire_version");
    assert_fixture_under_size_cap(&path);
    let report = replay_fixture(&path).expect("replay unknown wire version");

    // The request frame has cascade_wire_version=cascade.v9.9, which
    // is NOT in the registry. Per design.md §3 decision 5: fail
    // closed at decode boundary, no reserve, no commit.
    assert!(report.unsupported_wire_version_seen);
    assert_eq!(report.request_frames_decoded, 0);
    assert_eq!(report.sidecar_reserve_calls, 0);
    assert_eq!(report.sidecar_commit_calls, 0);
}

// ============================================================================
// (6) Truncated body → decoder_skipped
// ============================================================================

#[test]
fn replay_cascade_chat_truncated_marks_decoder_skipped() {
    let path = fixture("cascade_chat_truncated");
    assert_fixture_under_size_cap(&path);
    let report = replay_fixture(&path).expect("replay truncated");

    // Per design.md §4.4: known-version body decode failure degrades
    // to `decoder_skipped` (best-effort gating). No reserve, no
    // commit, but the request would still have been forwarded by
    // the egress proxy.
    assert!(report.decoder_skipped);
    assert_eq!(report.request_frames_decoded, 0);
    assert_eq!(report.sidecar_reserve_calls, 0);
    assert_eq!(report.sidecar_commit_calls, 0);
}

// ============================================================================
// (7) Redaction regression guard
// ============================================================================

/// Reviewer rejects any committed fixture whose payload bytes contain
/// any of the listed Codeium / Windsurf credential prefixes. The
/// synthetic fixtures use `FAKE_*` sentinels; this test fails the
/// moment a real credential lands by accident.
#[test]
fn no_secret_leakage_in_fixtures() {
    let dir = fixtures_dir();
    let entries: Vec<_> = std::fs::read_dir(&dir)
        .expect("read fixtures dir")
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|x| x == "windsurf-rpc")
                .unwrap_or(false)
        })
        .collect();

    assert!(
        entries.len() >= 6,
        "expected ≥6 .windsurf-rpc fixtures, got {}",
        entries.len()
    );

    let bad_patterns: &[&[u8]] = &[
        b"sk-codeium-",
        b"wsf_",
        b"codeium_pat_",
        b"cdm_",
        // We intentionally do NOT scan for "codeium." or
        // "windsurf-server" because the manifest/markdown docs
        // mention those host strings; the fixture bytes themselves
        // are the protected surface here.
    ];

    for entry in &entries {
        let path = entry.path();
        let bytes = std::fs::read(&path).expect("read fixture");
        for pat in bad_patterns {
            let leaked = bytes.windows(pat.len()).any(|window| window == *pat);
            assert!(
                !leaked,
                "credential prefix {:?} leaked into fixture {}",
                String::from_utf8_lossy(pat),
                path.display()
            );
        }
    }
}

// ============================================================================
// (8) Direction byte coverage
// ============================================================================

#[test]
fn fixtures_cover_both_directions() {
    use spendguard_windsurf_codec::read_fixture;

    // The streaming fixture covers both directions; assert the
    // distribution to catch a regression where the writer flipped a
    // direction byte by accident.
    let path = fixture("cascade_chat_streaming");
    let (_, frames) = read_fixture(&path).expect("read fixture");

    let client_count = frames
        .iter()
        .filter(|f| f.direction == Direction::Client)
        .count();
    let server_count = frames
        .iter()
        .filter(|f| f.direction == Direction::Server)
        .count();
    assert_eq!(client_count, 1, "expected 1 client frame");
    assert!(server_count >= 9, "expected ≥9 server frames");
}

// ============================================================================
// (9) All-frames round-trip across the entire corpus
// ============================================================================

#[test]
fn all_committed_fixtures_round_trip_byte_identical() {
    for name in [
        "cascade_chat_simple",
        "cascade_chat_with_tools",
        "cascade_chat_streaming",
        "cascade_chat_error",
        "cascade_chat_unknown_wire_version",
        "cascade_chat_truncated",
    ] {
        let path = fixture(name);
        let report = replay_fixture(&path).expect("replay");
        assert!(
            report.all_frames_round_trip,
            "fixture {name} violated byte-for-byte preservation"
        );
    }
}
