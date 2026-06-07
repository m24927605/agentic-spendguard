//! Regenerate the synthetic `.windsurf-rpc` fixture corpus.
//!
//! D18 SLICE 80: the codec's fixture corpus is reproducible from a
//! Rust generator so reviewers can re-derive every byte of every
//! fixture without trusting opaque blobs. Mirrors the D17
//! cursor_codec/examples/regenerate_fixtures.rs pattern.
//!
//! Usage:
//!
//! ```sh
//! cargo run --manifest-path services/windsurf_codec/Cargo.toml \
//!     --example regenerate_fixtures
//! ```
//!
//! Six synthetic fixtures land under `fixtures/synthetic/`:
//!
//! 1. `cascade_chat_simple` — happy path single-turn / 2-delta stream.
//! 2. `cascade_chat_with_tools` — tool-declarations attached.
//! 3. `cascade_chat_streaming` — long streaming with terminal stop.
//! 4. `cascade_chat_error` — upstream grpc-status:13 trailers.
//! 5. `cascade_chat_unknown_wire_version` — `cascade.v9.9` stamp →
//!    `unsupported_wire_version_seen`.
//! 6. `cascade_chat_truncated` — known wire version but body fails
//!    prost decode → `decoder_skipped`.
//!
//! Each fixture also writes a sidecar `.manifest.json` documenting
//! the SOW provenance (mirror of D17 PROVENANCE.md convention).

use std::path::PathBuf;

use bytes::Bytes;
use spendguard_windsurf_codec::windsurf_proto::{
    CascadeMessage, CascadeRequest, CascadeResponseDelta, CascadeToolDecl, CascadeUsage,
};
use spendguard_windsurf_codec::{
    raw_frame, request_frame, response_delta_frame, trailers_frame, write_fixture_bytes, Direction,
    FixtureFrame,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/synthetic");
    std::fs::create_dir_all(&out_dir)?;

    let mut total_bytes = 0usize;

    for (name, frames, manifest) in fixtures() {
        let bytes = write_fixture_bytes(&frames);
        if bytes.len() > 64 * 1024 {
            return Err(format!("fixture {name} is {} bytes (cap is 64 KiB)", bytes.len()).into());
        }
        let fixture_path = out_dir.join(format!("{name}.windsurf-rpc"));
        let manifest_path = out_dir.join(format!("{name}.windsurf-rpc.manifest.json"));
        std::fs::write(&fixture_path, &bytes)?;
        std::fs::write(&manifest_path, manifest)?;
        total_bytes += bytes.len();
        println!(
            "[regenerate_fixtures] wrote {name} ({} bytes, {} frames)",
            bytes.len(),
            frames.len()
        );
    }

    println!(
        "[regenerate_fixtures] DONE — total {total_bytes} bytes across {} fixtures",
        6
    );
    Ok(())
}

fn fixtures() -> Vec<(&'static str, Vec<FixtureFrame>, &'static str)> {
    vec![
        ("cascade_chat_simple", simple_fixture(), SIMPLE_MANIFEST),
        (
            "cascade_chat_with_tools",
            with_tools_fixture(),
            WITH_TOOLS_MANIFEST,
        ),
        (
            "cascade_chat_streaming",
            streaming_fixture(),
            STREAMING_MANIFEST,
        ),
        ("cascade_chat_error", error_fixture(), ERROR_MANIFEST),
        (
            "cascade_chat_unknown_wire_version",
            unknown_wire_version_fixture(),
            UNKNOWN_VERSION_MANIFEST,
        ),
        (
            "cascade_chat_truncated",
            truncated_fixture(),
            TRUNCATED_MANIFEST,
        ),
    ]
}

fn user(content: &str) -> CascadeMessage {
    CascadeMessage {
        role: "user".to_string(),
        content: content.to_string(),
    }
}

fn assistant(content: &str) -> CascadeMessage {
    CascadeMessage {
        role: "assistant".to_string(),
        content: content.to_string(),
    }
}

fn system_msg(content: &str) -> CascadeMessage {
    CascadeMessage {
        role: "system".to_string(),
        content: content.to_string(),
    }
}

// ─────────────────────────────────────────────────────────────────────
// FIXTURE 1: cascade_chat_simple
// ─────────────────────────────────────────────────────────────────────
fn simple_fixture() -> Vec<FixtureFrame> {
    // NOTE: per legal posture / SOW.md §3, all message content is
    // SpendGuard-authored synthetic. The redaction sentinels we use
    // here (`FAKE_SOW_USER_TURN`, etc.) document the redaction shape
    // that real captures would carry in PROVENANCE.md.
    let req = CascadeRequest {
        messages: vec![user("FAKE_SOW_USER_TURN: write a haiku about budgets")],
        model_name: "gpt-4o".to_string(),
        max_tokens: Some(64),
        tool_declarations: vec![],
        workspace_id: Some("FAKE_WORKSPACE_SIMPLE".to_string()),
        cascade_wire_version: Some("cascade.v2.0".to_string()),
    };
    let delta_1 = CascadeResponseDelta {
        model_name: "gpt-4o".to_string(),
        text_chunk: Some("Tokens flow like silk,".to_string()),
        finish_reason: None,
        usage: None,
        cascade_wire_version: Some("cascade.v2.0".to_string()),
    };
    let delta_2 = CascadeResponseDelta {
        model_name: "gpt-4o".to_string(),
        text_chunk: Some(" budget watcher hums softly,".to_string()),
        finish_reason: None,
        usage: None,
        cascade_wire_version: Some("cascade.v2.0".to_string()),
    };
    let delta_3 = CascadeResponseDelta {
        model_name: "gpt-4o".to_string(),
        text_chunk: Some(" cost cap holds firm.".to_string()),
        finish_reason: Some("stop".to_string()),
        usage: Some(CascadeUsage {
            input_tokens: 11,
            output_tokens: 18,
        }),
        cascade_wire_version: Some("cascade.v2.0".to_string()),
    };
    vec![
        request_frame(1_700_000_000_000, &req),
        response_delta_frame(1_700_000_000_010, &delta_1, 0x00),
        response_delta_frame(1_700_000_000_020, &delta_2, 0x00),
        response_delta_frame(1_700_000_000_030, &delta_3, 0x00),
        trailers_frame(1_700_000_000_040, Bytes::from_static(b"grpc-status:0")),
    ]
}

const SIMPLE_MANIFEST: &str = r#"{
  "fixture": "cascade_chat_simple.windsurf-rpc",
  "kind": "synthetic",
  "windsurf_min_version": "synthetic",
  "windsurf_max_version": "synthetic",
  "captured_utc": "2026-06-07T00:00:00Z",
  "frames": 5,
  "wire_version": "cascade.v2.0",
  "shape": "happy path: single user turn, 3-delta streaming response with terminal usage stamp + EOS trailers (grpc-status:0)",
  "redaction_sha256": "synthetic-no-redaction-needed",
  "sow_id": "synthetic",
  "expected_replay_report": {
    "request_frames_decoded": 1,
    "response_frames_decoded": 3,
    "end_of_stream_frames": 1,
    "finish_reason": "stop",
    "cumulative_output_tokens": 18,
    "sidecar_reserve_calls": 1,
    "sidecar_commit_calls": 1,
    "upstream_error": false,
    "all_frames_round_trip": true,
    "unsupported_wire_version_seen": false,
    "decoder_skipped": false
  }
}
"#;

// ─────────────────────────────────────────────────────────────────────
// FIXTURE 2: cascade_chat_with_tools
// ─────────────────────────────────────────────────────────────────────
fn with_tools_fixture() -> Vec<FixtureFrame> {
    let req = CascadeRequest {
        messages: vec![
            system_msg("FAKE_SOW_SYSTEM_TURN: you are a code assistant"),
            user("FAKE_SOW_USER_TURN: list the files in /tmp"),
        ],
        model_name: "claude-3.5-sonnet".to_string(),
        max_tokens: Some(256),
        tool_declarations: vec![
            CascadeToolDecl {
                name: "read_file".to_string(),
                schema: "FAKE_REDACTED_SCHEMA".to_string(),
            },
            CascadeToolDecl {
                name: "list_dir".to_string(),
                schema: "FAKE_REDACTED_SCHEMA".to_string(),
            },
        ],
        workspace_id: Some("FAKE_WORKSPACE_WITH_TOOLS".to_string()),
        cascade_wire_version: Some("cascade.v2.1".to_string()),
    };
    let delta = CascadeResponseDelta {
        model_name: "claude-3.5-sonnet".to_string(),
        text_chunk: Some("I'll list the files now.".to_string()),
        finish_reason: Some("tool_calls".to_string()),
        usage: Some(CascadeUsage {
            input_tokens: 23,
            output_tokens: 9,
        }),
        cascade_wire_version: Some("cascade.v2.1".to_string()),
    };
    vec![
        request_frame(1_700_000_001_000, &req),
        response_delta_frame(1_700_000_001_010, &delta, 0x00),
        trailers_frame(1_700_000_001_020, Bytes::from_static(b"grpc-status:0")),
    ]
}

const WITH_TOOLS_MANIFEST: &str = r#"{
  "fixture": "cascade_chat_with_tools.windsurf-rpc",
  "kind": "synthetic",
  "windsurf_min_version": "synthetic",
  "windsurf_max_version": "synthetic",
  "captured_utc": "2026-06-07T00:00:00Z",
  "frames": 3,
  "wire_version": "cascade.v2.1",
  "shape": "Cascade Agent mode with 2 tool declarations + terminal finish_reason=tool_calls",
  "redaction_sha256": "synthetic-no-redaction-needed",
  "sow_id": "synthetic",
  "expected_replay_report": {
    "request_frames_decoded": 1,
    "response_frames_decoded": 1,
    "end_of_stream_frames": 1,
    "finish_reason": "tool_calls",
    "cumulative_output_tokens": 9,
    "sidecar_reserve_calls": 1,
    "sidecar_commit_calls": 1,
    "upstream_error": false,
    "all_frames_round_trip": true,
    "unsupported_wire_version_seen": false,
    "decoder_skipped": false
  }
}
"#;

// ─────────────────────────────────────────────────────────────────────
// FIXTURE 3: cascade_chat_streaming
// ─────────────────────────────────────────────────────────────────────
fn streaming_fixture() -> Vec<FixtureFrame> {
    let req = CascadeRequest {
        messages: vec![
            system_msg("FAKE_SOW_SYSTEM: be thorough"),
            user("FAKE_SOW_USER: write the longer reply"),
            assistant("FAKE_SOW_ASSISTANT_PREVIOUS: prior turn"),
            user("FAKE_SOW_USER_FOLLOWUP: continue"),
        ],
        model_name: "gpt-4o".to_string(),
        max_tokens: Some(1024),
        tool_declarations: vec![],
        workspace_id: Some("FAKE_WORKSPACE_STREAMING".to_string()),
        cascade_wire_version: Some("cascade.v2.0".to_string()),
    };
    let mut frames = vec![request_frame(1_700_000_002_000, &req)];

    // 8 streaming deltas + 1 terminal usage.
    for i in 0..8 {
        let delta = CascadeResponseDelta {
            model_name: if i == 0 {
                "gpt-4o".to_string()
            } else {
                String::new()
            },
            text_chunk: Some(format!(" chunk{i}")),
            finish_reason: None,
            usage: None,
            cascade_wire_version: Some("cascade.v2.0".to_string()),
        };
        frames.push(response_delta_frame(
            1_700_000_002_010 + (i as u64) * 10,
            &delta,
            0x00,
        ));
    }
    let terminal = CascadeResponseDelta {
        model_name: String::new(),
        text_chunk: None,
        finish_reason: Some("stop".to_string()),
        usage: Some(CascadeUsage {
            input_tokens: 31,
            output_tokens: 47,
        }),
        cascade_wire_version: Some("cascade.v2.0".to_string()),
    };
    frames.push(response_delta_frame(1_700_000_002_100, &terminal, 0x00));
    frames.push(trailers_frame(
        1_700_000_002_110,
        Bytes::from_static(b"grpc-status:0"),
    ));
    frames
}

const STREAMING_MANIFEST: &str = r#"{
  "fixture": "cascade_chat_streaming.windsurf-rpc",
  "kind": "synthetic",
  "windsurf_min_version": "synthetic",
  "windsurf_max_version": "synthetic",
  "captured_utc": "2026-06-07T00:00:00Z",
  "frames": 11,
  "wire_version": "cascade.v2.0",
  "shape": "8 streaming deltas + terminal usage with finish_reason=stop + EOS trailers",
  "redaction_sha256": "synthetic-no-redaction-needed",
  "sow_id": "synthetic",
  "expected_replay_report": {
    "request_frames_decoded": 1,
    "response_frames_decoded": 9,
    "end_of_stream_frames": 1,
    "finish_reason": "stop",
    "cumulative_output_tokens": 47,
    "sidecar_reserve_calls": 1,
    "sidecar_commit_calls": 1,
    "upstream_error": false,
    "all_frames_round_trip": true,
    "unsupported_wire_version_seen": false,
    "decoder_skipped": false
  }
}
"#;

// ─────────────────────────────────────────────────────────────────────
// FIXTURE 4: cascade_chat_error
// ─────────────────────────────────────────────────────────────────────
fn error_fixture() -> Vec<FixtureFrame> {
    let req = CascadeRequest {
        messages: vec![user("FAKE_SOW_USER: triggers upstream 500")],
        model_name: "gpt-4o".to_string(),
        max_tokens: Some(64),
        tool_declarations: vec![],
        workspace_id: Some("FAKE_WORKSPACE_ERROR".to_string()),
        cascade_wire_version: Some("cascade.v2.0".to_string()),
    };
    vec![
        request_frame(1_700_000_003_000, &req),
        trailers_frame(
            1_700_000_003_010,
            Bytes::from_static(b"grpc-status:13\rgrpc-message:upstream provider returned 500"),
        ),
    ]
}

const ERROR_MANIFEST: &str = r#"{
  "fixture": "cascade_chat_error.windsurf-rpc",
  "kind": "synthetic",
  "windsurf_min_version": "synthetic",
  "windsurf_max_version": "synthetic",
  "captured_utc": "2026-06-07T00:00:00Z",
  "frames": 2,
  "wire_version": "cascade.v2.0",
  "shape": "request + upstream grpc-status:13 trailers (no response deltas) — codec reserves but does NOT commit",
  "redaction_sha256": "synthetic-no-redaction-needed",
  "sow_id": "synthetic",
  "expected_replay_report": {
    "request_frames_decoded": 1,
    "response_frames_decoded": 0,
    "end_of_stream_frames": 1,
    "finish_reason": null,
    "cumulative_output_tokens": null,
    "sidecar_reserve_calls": 1,
    "sidecar_commit_calls": 0,
    "upstream_error": true,
    "all_frames_round_trip": true,
    "unsupported_wire_version_seen": false,
    "decoder_skipped": false
  }
}
"#;

// ─────────────────────────────────────────────────────────────────────
// FIXTURE 5: cascade_chat_unknown_wire_version
// ─────────────────────────────────────────────────────────────────────
fn unknown_wire_version_fixture() -> Vec<FixtureFrame> {
    let req = CascadeRequest {
        messages: vec![user("FAKE_SOW_USER: from future Cascade build")],
        model_name: "gpt-4o-future".to_string(),
        max_tokens: Some(64),
        tool_declarations: vec![],
        workspace_id: Some("FAKE_WORKSPACE_FUTURE".to_string()),
        cascade_wire_version: Some("cascade.v9.9".to_string()), // not in registry
    };
    vec![
        request_frame(1_700_000_004_000, &req),
        trailers_frame(1_700_000_004_010, Bytes::from_static(b"grpc-status:0")),
    ]
}

const UNKNOWN_VERSION_MANIFEST: &str = r#"{
  "fixture": "cascade_chat_unknown_wire_version.windsurf-rpc",
  "kind": "synthetic",
  "windsurf_min_version": "synthetic",
  "windsurf_max_version": "synthetic",
  "captured_utc": "2026-06-07T00:00:00Z",
  "frames": 2,
  "wire_version": "cascade.v9.9 (UNKNOWN — intentionally outside registry)",
  "shape": "request stamped cascade.v9.9 → codec emits windsurf_wire_version_unsupported, no reserve, no commit",
  "redaction_sha256": "synthetic-no-redaction-needed",
  "sow_id": "synthetic",
  "expected_replay_report": {
    "request_frames_decoded": 0,
    "response_frames_decoded": 0,
    "end_of_stream_frames": 1,
    "finish_reason": null,
    "cumulative_output_tokens": null,
    "sidecar_reserve_calls": 0,
    "sidecar_commit_calls": 0,
    "upstream_error": false,
    "all_frames_round_trip": true,
    "unsupported_wire_version_seen": true,
    "decoder_skipped": false
  }
}
"#;

// ─────────────────────────────────────────────────────────────────────
// FIXTURE 6: cascade_chat_truncated
// ─────────────────────────────────────────────────────────────────────
fn truncated_fixture() -> Vec<FixtureFrame> {
    // Known-version envelope shape claimed via the timestamp but the
    // payload is deliberately garbage so prost decode fails → the
    // codec emits `decoder_skipped` and forwards anyway (best-effort
    // gating per design.md §4.4).
    vec![
        raw_frame(
            1_700_000_005_000,
            Direction::Client,
            0x00,
            Bytes::from_static(b"\xff\xff\xff garbage that won't decode as CascadeRequest"),
        ),
        trailers_frame(1_700_000_005_010, Bytes::from_static(b"grpc-status:0")),
    ]
}

const TRUNCATED_MANIFEST: &str = r#"{
  "fixture": "cascade_chat_truncated.windsurf-rpc",
  "kind": "synthetic",
  "windsurf_min_version": "synthetic",
  "windsurf_max_version": "synthetic",
  "captured_utc": "2026-06-07T00:00:00Z",
  "frames": 2,
  "wire_version": "unknown (preamble hash falls back to garbage)",
  "shape": "request body is garbage bytes that prost cannot decode as CascadeRequest → codec emits decoder_skipped, forwards anyway (best-effort gating)",
  "redaction_sha256": "synthetic-no-redaction-needed",
  "sow_id": "synthetic",
  "expected_replay_report": {
    "request_frames_decoded": 0,
    "response_frames_decoded": 0,
    "end_of_stream_frames": 1,
    "finish_reason": null,
    "cumulative_output_tokens": null,
    "sidecar_reserve_calls": 0,
    "sidecar_commit_calls": 0,
    "upstream_error": false,
    "all_frames_round_trip": true,
    "unsupported_wire_version_seen": false,
    "decoder_skipped": true
  }
}
"#;
