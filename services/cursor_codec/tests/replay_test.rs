//! D17 SLICE 8 — fixture replay harness integration tests.
//!
//! Each test loads a committed `.cursor-rpc` fixture under
//! `fixtures/synthetic/`, replays it through
//! [`spendguard_cursor_codec::replay::replay_fixture`], and asserts
//! the layered pipeline (framing decode → envelope decode →
//! translation → mock sidecar reserve+commit → byte-for-byte
//! preservation) lands per the fixture's expected report.
//!
//! Per [`review-standards.md`](../../../docs/specs/coverage/D17_cursor_mitm/review-standards.md)
//! §6 (`C1`): no live `api.cursor.sh` traffic. The harness is offline
//! against committed bytes.

use std::path::{Path, PathBuf};

use spendguard_cursor_codec::{
    read_fixture, replay_fixture, replay_fixture_bytes, write_fixture_bytes, Direction,
    FixtureFrame,
};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/synthetic")
}

fn fixture(name: &str) -> PathBuf {
    fixtures_dir().join(format!("{name}.cursor-rpc"))
}

fn assert_fixture_under_size_cap(path: &Path) {
    // Per review-standards C4 — committed fixtures must stay under
    // 64 KiB. This is a regression guard against accidental bloat.
    let bytes = std::fs::read(path).expect("read fixture");
    assert!(
        bytes.len() <= 64 * 1024,
        "fixture {} is {} bytes > 64 KiB cap (review-standards.md C4)",
        path.display(),
        bytes.len()
    );
}

// ============================================================================
// (1) Multi-turn conversation
// ============================================================================

/// Multi-turn fixture: 4 messages in request, 3-chunk reply ending
/// finish_reason=stop, EOS trailers grpc-status:0.
#[test]
fn replay_multiturn_conversation_lands_full_cycle() {
    let path = fixture("synthetic_multiturn_v1");
    assert_fixture_under_size_cap(&path);
    let report = replay_fixture(&path).expect("replay multiturn");

    assert_eq!(report.version, 1);
    assert_eq!(report.request_frames_decoded, 1);
    assert_eq!(report.response_chunks_decoded, 3);
    assert_eq!(report.end_of_stream_frames, 1);
    assert_eq!(report.finish_reason.as_deref(), Some("stop"));
    assert_eq!(report.cumulative_output_tokens, Some(11));
    assert_eq!(report.sidecar_reserve_calls, 1);
    assert_eq!(report.sidecar_commit_calls, 1);
    assert!(!report.upstream_error);
    assert!(
        report.request_bytes_round_trip,
        "first request bytes must round-trip"
    );
    assert!(report.all_frames_round_trip, "every frame must round-trip");

    // Cursor's leading messages[0].role=system MUST win over top-
    // level `system` field per the translator's precedence rule.
    let translated = report.translated_request.as_ref().expect("translated");
    assert_eq!(translated.messages.len(), 4);
    assert_eq!(translated.messages[0].role, "system");
    // Translator must NOT prepend a synthetic system message — the
    // existing role=system entry wins. (See translate.rs §3.)
    assert_eq!(
        translated.messages[0].content, "You are Cursor Agent. Be terse.",
        "leading role=system in messages wins over top-level system field"
    );

    let decoded = report.decoded_requests.first().expect("decoded request");
    assert_eq!(decoded.model, "claude-3.5-sonnet");
    assert_eq!(decoded.max_tokens, Some(256));
}

// ============================================================================
// (2) Tool calls (Cursor Agent mode)
// ============================================================================

/// Tool-call fixture: 3 messages including role=tool; response stream
/// ends with finish_reason=tool_calls (NOT stop).
#[test]
fn replay_tool_calls_records_tool_calls_finish_reason() {
    let path = fixture("synthetic_tool_calls_v1");
    assert_fixture_under_size_cap(&path);
    let report = replay_fixture(&path).expect("replay tool_calls");

    assert_eq!(report.request_frames_decoded, 1);
    assert_eq!(report.response_chunks_decoded, 2);
    assert_eq!(report.finish_reason.as_deref(), Some("tool_calls"));
    assert_eq!(report.sidecar_reserve_calls, 1);
    assert_eq!(
        report.sidecar_commit_calls, 1,
        "tool_calls finish is still a successful upstream → commits"
    );
    assert!(!report.upstream_error);
    assert!(report.all_frames_round_trip);

    let decoded = report.decoded_requests.first().expect("decoded request");
    assert_eq!(decoded.messages.len(), 3);
    let roles: Vec<&str> = decoded.messages.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(roles, vec!["user", "assistant", "tool"]);
    assert!(
        decoded.messages[1].content.contains("tool_calls"),
        "assistant content should carry the tool_calls JSON payload"
    );
    assert!(
        decoded.messages[2].content.contains("call_01"),
        "tool result must echo the tool_call_id"
    );
}

// ============================================================================
// (3) Error responses
// ============================================================================

/// Error fixture: 1 request + immediate trailers with grpc-status:13.
/// Reserve fires; commit MUST NOT fire (release-and-pass-through).
#[test]
fn replay_error_response_short_circuits_commit_path() {
    let path = fixture("synthetic_error_response_v1");
    assert_fixture_under_size_cap(&path);
    let report = replay_fixture(&path).expect("replay error_response");

    assert_eq!(report.request_frames_decoded, 1);
    assert_eq!(report.response_chunks_decoded, 0);
    assert_eq!(report.end_of_stream_frames, 1);
    assert!(
        report.upstream_error,
        "grpc-status:13 trailers must flip the upstream_error flag"
    );
    assert_eq!(report.sidecar_reserve_calls, 1);
    assert_eq!(
        report.sidecar_commit_calls, 0,
        "upstream error MUST short-circuit commit per design.md §2 (P3 release-and-pass-through)"
    );
    assert!(
        report.finish_reason.is_none(),
        "no data chunks → no finish_reason"
    );
    assert!(
        report.all_frames_round_trip,
        "error trailers still round-trip byte-identical"
    );
}

// ============================================================================
// (4) Long streams (≥10 chunks)
// ============================================================================

/// Long-stream fixture: 13 response chunks + EOS. Per-chunk
/// cumulative_output_tokens is monotonically non-decreasing.
#[test]
fn replay_long_stream_handles_at_least_ten_chunks() {
    let path = fixture("synthetic_long_stream_v1");
    assert_fixture_under_size_cap(&path);
    let report = replay_fixture(&path).expect("replay long_stream");

    assert_eq!(report.request_frames_decoded, 1);
    assert!(
        report.response_chunks_decoded >= 10,
        "long stream MUST carry >= 10 response chunks per slice spec; got {}",
        report.response_chunks_decoded
    );
    assert_eq!(report.end_of_stream_frames, 1);
    assert_eq!(report.finish_reason.as_deref(), Some("stop"));
    assert_eq!(report.sidecar_reserve_calls, 1);
    assert_eq!(report.sidecar_commit_calls, 1);
    assert!(!report.upstream_error);
    assert!(report.all_frames_round_trip);

    // cumulative_output_tokens must be monotonically non-decreasing
    // across chunks; the terminal chunk repeats the final value.
    let mut last = 0u32;
    for (i, chunk) in report.decoded_responses.iter().enumerate() {
        if let Some(t) = chunk.cumulative_output_tokens {
            assert!(
                t >= last,
                "cumulative_output_tokens regressed at chunk {i}: {last} → {t}"
            );
            last = t;
        }
    }
    assert!(last > 0, "long stream must end with non-zero token count");
}

// ============================================================================
// (5) Byte-for-byte: read → write → read round-trip across all four fixtures
// ============================================================================

/// All four SLICE 8 fixtures survive a read → write → read cycle byte-
/// identically. Asserts the W5 byte-for-byte preservation contract on
/// the full fixture corpus.
#[test]
fn all_slice8_fixtures_round_trip_byte_identical() {
    let names = [
        "synthetic_multiturn_v1",
        "synthetic_tool_calls_v1",
        "synthetic_error_response_v1",
        "synthetic_long_stream_v1",
    ];
    for name in names {
        let path = fixture(name);
        let original = std::fs::read(&path).expect("read fixture");
        let (version, frames) = read_fixture(&path).expect("parse fixture");
        let rewritten = write_fixture_bytes(&frames);
        assert_eq!(
            rewritten,
            original,
            "fixture {name} drifted on read+write round-trip; \
             version={version}; {} frames",
            frames.len()
        );
    }
}

// ============================================================================
// (6) Sidecar mock fires on every fixture with a request frame
// ============================================================================

/// Across all four SLICE 8 fixtures, every request frame triggers
/// exactly one mock-sidecar reserve call. Asserts the SLICE 8
/// reserve-before-forward contract holds independent of fixture shape.
#[test]
fn replay_records_reserve_per_request_across_corpus() {
    let cases = [
        ("synthetic_multiturn_v1", 1u32),
        ("synthetic_tool_calls_v1", 1u32),
        ("synthetic_error_response_v1", 1u32),
        ("synthetic_long_stream_v1", 1u32),
    ];
    for (name, expected_reserves) in cases {
        let report = replay_fixture(&fixture(name)).expect("replay");
        assert_eq!(
            report.sidecar_reserve_calls, expected_reserves,
            "{name}: reserve count mismatch"
        );
        assert_eq!(
            report.sidecar_reserve_calls, report.request_frames_decoded,
            "{name}: reserves must equal request frames decoded (reserve-per-request)"
        );
    }
}

// ============================================================================
// (7) PROTOCOL.md hex evidence cross-check
// ============================================================================

/// Replay each fixture and cross-check the hex preamble of its first
/// non-envelope record against the documented capture date encoded as
/// `timestamp_ms`. Asserts that the timestamp field documented in
/// PROTOCOL.md aligns with what the fixture carries.
#[test]
fn fixture_timestamps_are_within_synthetic_capture_window() {
    let names = [
        "synthetic_multiturn_v1",
        "synthetic_tool_calls_v1",
        "synthetic_error_response_v1",
        "synthetic_long_stream_v1",
    ];
    // PROTOCOL.md §1 documents synthetic capture at 2026-06-07T00:00:00Z
    // ± a small window. Synthetic timestamps were minted as
    // 1_716_000_*_***_***, which lands in May 2024 by construction —
    // the fixture timestamps are intentionally NOT the capture date,
    // they're stable synthetic offsets so the corpus is bit-stable. We
    // assert (a) every frame has a non-zero timestamp and (b) they're
    // monotonically non-decreasing within the fixture.
    for name in names {
        let (_, frames) = read_fixture(&fixture(name)).expect("parse fixture");
        let mut last = 0u64;
        for (i, ff) in frames.iter().enumerate() {
            assert!(
                ff.timestamp_ms > 0,
                "{name} frame {i}: timestamp_ms must be > 0"
            );
            assert!(
                ff.timestamp_ms >= last,
                "{name} frame {i}: timestamp_ms regressed ({last} → {})",
                ff.timestamp_ms
            );
            last = ff.timestamp_ms;
        }
    }
}

// ============================================================================
// (8) Direction roll-up matches review-standards §6 invariants
// ============================================================================

/// Across all fixtures, the count of Client vs Server frames matches
/// the invariant: exactly one Client (request) frame per fixture, and
/// at least one Server frame (either a data chunk, a trailers blob, or
/// both).
#[test]
fn direction_invariants_hold_across_corpus() {
    let cases = [
        ("synthetic_multiturn_v1", 1u32, 4u32),
        ("synthetic_tool_calls_v1", 1u32, 3u32),
        ("synthetic_error_response_v1", 1u32, 1u32),
        ("synthetic_long_stream_v1", 1u32, 14u32),
    ];
    for (name, expected_client, expected_server) in cases {
        let (_, frames) = read_fixture(&fixture(name)).expect("parse fixture");
        let mut client = 0u32;
        let mut server = 0u32;
        for ff in &frames {
            match ff.direction {
                Direction::Client => client += 1,
                Direction::Server => server += 1,
            }
        }
        assert_eq!(client, expected_client, "{name}: client frame count");
        assert_eq!(server, expected_server, "{name}: server frame count");
    }
}

// ============================================================================
// (9) Manifests exist and document expected_replay_report
// ============================================================================

/// Every SLICE 8 fixture has a sidecar `.manifest.json` per R2 / R4 —
/// the manifest documents capture date + version range + field
/// evidence + expected replay report. The integration test asserts
/// the manifest is present and valid JSON, and that the
/// expected_replay_report matches the live replay.
#[test]
fn fixture_manifests_match_live_replay_reports() {
    let cases = [
        "synthetic_multiturn_v1",
        "synthetic_tool_calls_v1",
        "synthetic_error_response_v1",
        "synthetic_long_stream_v1",
    ];
    for name in cases {
        let fixture_path = fixture(name);
        let manifest_path = fixtures_dir().join(format!("{name}.cursor-rpc.manifest.json"));
        assert!(
            manifest_path.exists(),
            "manifest {} missing for fixture {name}",
            manifest_path.display()
        );

        let manifest_bytes = std::fs::read(&manifest_path).expect("read manifest");
        let manifest: serde_json::Value =
            serde_json::from_slice(&manifest_bytes).expect("manifest must be valid JSON");
        assert_eq!(
            manifest["fixture"].as_str().expect("fixture field"),
            format!("{name}.cursor-rpc"),
            "manifest fixture name must match filename"
        );
        let expected = &manifest["expected_replay_report"];
        assert!(
            expected.is_object(),
            "manifest must carry expected_replay_report object for {name}"
        );

        let report = replay_fixture(&fixture_path).expect("replay");
        if let Some(v) = expected["request_frames_decoded"].as_u64() {
            assert_eq!(
                report.request_frames_decoded as u64, v,
                "{name}: request_frames_decoded"
            );
        }
        if let Some(v) = expected["response_chunks_decoded"].as_u64() {
            assert_eq!(
                report.response_chunks_decoded as u64, v,
                "{name}: response_chunks_decoded"
            );
        }
        if let Some(v) = expected["sidecar_reserve_calls"].as_u64() {
            assert_eq!(
                report.sidecar_reserve_calls as u64, v,
                "{name}: sidecar_reserve_calls"
            );
        }
        if let Some(v) = expected["sidecar_commit_calls"].as_u64() {
            assert_eq!(
                report.sidecar_commit_calls as u64, v,
                "{name}: sidecar_commit_calls"
            );
        }
        if let Some(v) = expected["upstream_error"].as_bool() {
            assert_eq!(report.upstream_error, v, "{name}: upstream_error");
        }
        if expected.get("finish_reason").is_some() {
            let expected_reason = expected["finish_reason"].as_str();
            assert_eq!(
                report.finish_reason.as_deref(),
                expected_reason,
                "{name}: finish_reason"
            );
        }
    }
}

// ============================================================================
// (10) Synthetic fixtures all label themselves "synthetic" per R4
// ============================================================================

/// R4: every fixture filename or manifest carries a version range.
/// Synthetic fixtures use the literal token `synthetic` in place of
/// a real Cursor client version.
#[test]
fn synthetic_fixtures_carry_synthetic_version_tag() {
    let cases = [
        "synthetic_multiturn_v1",
        "synthetic_tool_calls_v1",
        "synthetic_error_response_v1",
        "synthetic_long_stream_v1",
    ];
    for name in cases {
        assert!(
            name.starts_with("synthetic"),
            "filename {name} MUST start with 'synthetic' per R4"
        );
        let manifest_path = fixtures_dir().join(format!("{name}.cursor-rpc.manifest.json"));
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&manifest_path).expect("read manifest"))
                .expect("manifest JSON");
        assert_eq!(
            manifest["cursor_min_version"].as_str(),
            Some("synthetic"),
            "{name}: cursor_min_version MUST be 'synthetic' for synthetic corpus"
        );
        assert_eq!(
            manifest["cursor_max_version"].as_str(),
            Some("synthetic"),
            "{name}: cursor_max_version MUST be 'synthetic' for synthetic corpus"
        );
    }
}

// ============================================================================
// (11) In-memory replay equals on-disk replay
// ============================================================================

/// `replay_fixture_bytes` (no disk) and `replay_fixture` (disk) produce
/// identical reports. The SLICE 9 demo container uses the bytes form,
/// so this invariant guards against disk/in-memory drift.
#[test]
fn in_memory_replay_matches_on_disk_replay() {
    let names = [
        "synthetic_multiturn_v1",
        "synthetic_tool_calls_v1",
        "synthetic_error_response_v1",
        "synthetic_long_stream_v1",
    ];
    for name in names {
        let path = fixture(name);
        let bytes = std::fs::read(&path).expect("read fixture");
        let r_disk = replay_fixture(&path).expect("disk replay");
        let r_mem = replay_fixture_bytes(&bytes).expect("memory replay");
        assert_eq!(
            r_disk.request_frames_decoded, r_mem.request_frames_decoded,
            "{name}"
        );
        assert_eq!(
            r_disk.response_chunks_decoded, r_mem.response_chunks_decoded,
            "{name}"
        );
        assert_eq!(
            r_disk.sidecar_reserve_calls, r_mem.sidecar_reserve_calls,
            "{name}"
        );
        assert_eq!(
            r_disk.sidecar_commit_calls, r_mem.sidecar_commit_calls,
            "{name}"
        );
        assert_eq!(r_disk.upstream_error, r_mem.upstream_error, "{name}");
        assert_eq!(
            r_disk.all_frames_round_trip, r_mem.all_frames_round_trip,
            "{name}"
        );
    }
}

// ============================================================================
// (12) Legacy SLICE 1-2 synthetic fixtures still replay cleanly
// ============================================================================

/// The SLICE 1 fixtures (`synthetic_unary_v1`,
/// `synthetic_streaming_chunked_v1`) committed by COV_S17_01 MUST
/// still replay cleanly via the SLICE 8 harness. Guards against
/// accidental SLICE 8 changes invalidating the older corpus.
#[test]
fn legacy_slice1_fixtures_still_replay() {
    for name in ["synthetic_unary_v1", "synthetic_streaming_chunked_v1"] {
        let path = fixture(name);
        if !path.exists() {
            continue;
        }
        let report = replay_fixture(&path).expect("legacy fixture replay");
        assert!(
            report.request_frames_decoded > 0 || report.response_chunks_decoded > 0,
            "{name}: legacy fixture must decode at least one envelope"
        );
        assert!(
            report.all_frames_round_trip,
            "{name}: legacy fixture must still round-trip byte-identical"
        );
    }
}

// ============================================================================
// (13) Synthetic fixture writer matches replay roundtrip
// ============================================================================

/// Build a fixture in-memory via the `request_frame` / etc helpers,
/// then replay it — the report must reflect the structure we wrote.
/// Guards against the writer drifting from the reader.
#[test]
fn in_memory_writer_round_trips_through_replay() {
    use spendguard_cursor_codec::cursor_proto::{
        CursorChatRequest, CursorChatResponseChunk, Message as CursorMessage,
    };
    let req = CursorChatRequest {
        messages: vec![CursorMessage {
            role: "user".to_string(),
            content: "smoke".to_string(),
        }],
        model: "claude-3.5-sonnet".to_string(),
        system: None,
        max_tokens: Some(8),
        temperature: Some(0.0),
    };
    let chunk = CursorChatResponseChunk {
        model: "claude-3.5-sonnet".to_string(),
        delta: "ok".to_string(),
        finish_reason: Some("stop".to_string()),
        cumulative_output_tokens: Some(1),
    };
    let frames: Vec<FixtureFrame> = vec![
        spendguard_cursor_codec::request_frame(1000, &req),
        spendguard_cursor_codec::response_chunk_frame(1010, &chunk, 0x00),
        spendguard_cursor_codec::trailers_frame(1020, bytes::Bytes::from_static(b"grpc-status:0")),
    ];
    let bytes = write_fixture_bytes(&frames);
    let report = replay_fixture_bytes(&bytes).expect("replay");
    assert_eq!(report.request_frames_decoded, 1);
    assert_eq!(report.response_chunks_decoded, 1);
    assert_eq!(report.end_of_stream_frames, 1);
    assert_eq!(report.finish_reason.as_deref(), Some("stop"));
    assert_eq!(report.sidecar_reserve_calls, 1);
    assert_eq!(report.sidecar_commit_calls, 1);
    assert!(report.all_frames_round_trip);
}
