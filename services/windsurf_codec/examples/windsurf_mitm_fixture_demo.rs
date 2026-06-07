//! D18 SLICE 82 — `windsurf_mitm_fixture` demo runner.
//!
//! Replays the six SLICE 80 synthetic `.windsurf-rpc` fixtures
//! through the codec pipeline:
//!
//! 1. `replay_fixture` parses + decodes + version-gates +
//!    translates + reserves + commits via the in-memory mock
//!    sidecar.
//! 2. For success fixtures, the translated canonical OpenAI body is
//!    POSTed to the counting-stub HTTP endpoint to prove the
//!    upstream-forward path is wired and reachable.
//!
//! Per [`SOW.md`](../SOW.md) §6: this is a **fixture replay**, not a
//! real Windsurf binary exercise.
//!
//! Per D18 design.md §3 decision 7: no `server.codeium.com` traffic.
//! All POSTs go to the counting-stub at
//! `http://counting-stub:8765/v1/chat/completions` inside the demo
//! network.
//!
//! ## Usage
//!
//! ```sh
//! cargo run --manifest-path services/windsurf_codec/Cargo.toml \
//!     --features mitm \
//!     --example windsurf_mitm_fixture_demo
//! ```
//!
//! Environment variables:
//!
//! * `SPENDGUARD_WINDSURF_MITM_DEMO_FIXTURES_DIR` — override the
//!   corpus path (default
//!   `services/windsurf_codec/fixtures/synthetic/`).
//! * `SPENDGUARD_WINDSURF_MITM_DEMO_COUNTING_STUB_URL` — counting
//!   stub base URL (default `http://counting-stub:8765`).
//! * `SPENDGUARD_WINDSURF_MITM_DEMO_SKIP_STUB` — set to `1` to skip
//!   the counting-stub POST.

use std::collections::HashMap;
use std::io::Read;
use std::io::Write;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::time::Duration;

use spendguard_windsurf_codec::{replay_fixture, ReplayReport};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("[windsurf-mitm-fixture-demo] starting");

    let fixtures_dir: PathBuf = std::env::var("SPENDGUARD_WINDSURF_MITM_DEMO_FIXTURES_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| default_fixtures_dir());
    eprintln!(
        "[windsurf-mitm-fixture-demo] fixtures dir: {}",
        fixtures_dir.display()
    );

    let stub_url = std::env::var("SPENDGUARD_WINDSURF_MITM_DEMO_COUNTING_STUB_URL")
        .unwrap_or_else(|_| "http://counting-stub:8765".to_string());
    let skip_stub = std::env::var("SPENDGUARD_WINDSURF_MITM_DEMO_SKIP_STUB")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    let fixtures = [
        "cascade_chat_simple",
        "cascade_chat_with_tools",
        "cascade_chat_streaming",
        "cascade_chat_error",
        "cascade_chat_unknown_wire_version",
        "cascade_chat_truncated",
    ];

    let mut total_reserves: u32 = 0;
    let mut total_commits: u32 = 0;
    let mut total_errors: u32 = 0;
    let mut total_unsupported: u32 = 0;
    let mut total_decoder_skipped: u32 = 0;
    let mut stub_pre_count: Option<u32> = None;

    if !skip_stub {
        stub_pre_count = Some(read_counter(&stub_url)?);
        eprintln!(
            "[windsurf-mitm-fixture-demo] counting-stub pre-count = {}",
            stub_pre_count.unwrap()
        );
    } else {
        eprintln!(
            "[windsurf-mitm-fixture-demo] SPENDGUARD_WINDSURF_MITM_DEMO_SKIP_STUB=1 — skipping stub"
        );
    }

    let mut expected_stub_hits: u32 = 0;
    let mut per_fixture: HashMap<&str, ReplayReport> = HashMap::new();

    for name in fixtures {
        let path = fixtures_dir.join(format!("{name}.windsurf-rpc"));
        eprintln!("[windsurf-mitm-fixture-demo] replaying {}", path.display());
        let report = replay_fixture(&path)?;
        eprintln!(
            "[windsurf-mitm-fixture-demo]   reserves={} commits={} req_frames={} resp_frames={} upstream_error={} unsupported={} decoder_skipped={} all_round_trip={}",
            report.sidecar_reserve_calls,
            report.sidecar_commit_calls,
            report.request_frames_decoded,
            report.response_frames_decoded,
            report.upstream_error,
            report.unsupported_wire_version_seen,
            report.decoder_skipped,
            report.all_frames_round_trip,
        );

        total_reserves += report.sidecar_reserve_calls;
        total_commits += report.sidecar_commit_calls;
        if report.upstream_error {
            total_errors += 1;
        }
        if report.unsupported_wire_version_seen {
            total_unsupported += 1;
        }
        if report.decoder_skipped {
            total_decoder_skipped += 1;
        }
        if !report.all_frames_round_trip {
            return Err(format!(
                "fixture {name}: all_frames_round_trip=false — byte-for-byte preservation violated"
            )
            .into());
        }

        // POST the translated body to the counting-stub when we
        // actually have one (success fixtures with a decoded
        // request). Unknown-wire-version + truncated fixtures don't
        // produce a translated body, so no stub hit.
        if !skip_stub {
            if let Some(req) = report.translated_request.as_ref() {
                let body = serde_json::to_vec(req)?;
                post_chat_completions(&stub_url, &body)?;
                expected_stub_hits += 1;
            }
        }

        per_fixture.insert(name, report);
    }

    if !skip_stub {
        let stub_post_count = read_counter(&stub_url)?;
        let delta = stub_post_count - stub_pre_count.unwrap();
        eprintln!(
            "[windsurf-mitm-fixture-demo] counting-stub post-count = {}, delta = {}",
            stub_post_count, delta
        );
        if delta != expected_stub_hits {
            return Err(format!(
                "stub counter delta {delta} != expected {expected_stub_hits}; \
                 upstream-forward path drifted"
            )
            .into());
        }
    }

    // Headline assertions:
    // - 4 reserves (3 success fixtures + 1 error fixture decode succeeded
    //   but no commit). simple+with_tools+streaming+error → 4 reserves.
    // - 3 commits (simple, with_tools, streaming).
    // - 1 upstream error (cascade_chat_error).
    // - 1 unsupported wire version (cascade_chat_unknown_wire_version).
    // - 1 decoder skipped (cascade_chat_truncated).
    if total_reserves != 4 {
        return Err(format!(
            "expected 4 mock-sidecar reserves across corpus, got {total_reserves}"
        )
        .into());
    }
    if total_commits != 3 {
        return Err(
            format!("expected 3 mock-sidecar commits across corpus, got {total_commits}").into(),
        );
    }
    if total_errors != 1 {
        return Err(
            format!("expected 1 upstream_error fixture across corpus, got {total_errors}").into(),
        );
    }
    if total_unsupported != 1 {
        return Err(format!(
            "expected 1 unsupported_wire_version fixture across corpus, got {total_unsupported}"
        )
        .into());
    }
    if total_decoder_skipped != 1 {
        return Err(format!(
            "expected 1 decoder_skipped fixture across corpus, got {total_decoder_skipped}"
        )
        .into());
    }

    println!("WINDSURF_MITM_FIXTURE_DEMO_OK");
    println!("  fixtures: {}", fixtures.len());
    println!("  total_reserves: {total_reserves}");
    println!("  total_commits: {total_commits}");
    println!("  total_upstream_errors: {total_errors}");
    println!("  total_unsupported_wire_version: {total_unsupported}");
    println!("  total_decoder_skipped: {total_decoder_skipped}");
    println!("  byte_for_byte_round_trip: true");
    for (name, report) in &per_fixture {
        println!(
            "  {name}: reserves={} commits={} req_frames={} resp_frames={} finish_reason={:?} cumulative_output_tokens={:?}",
            report.sidecar_reserve_calls,
            report.sidecar_commit_calls,
            report.request_frames_decoded,
            report.response_frames_decoded,
            report.finish_reason,
            report.cumulative_output_tokens,
        );
    }

    eprintln!("[windsurf-mitm-fixture-demo] PASS");
    Ok(())
}

fn default_fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/synthetic")
}

/// Tiny synchronous HTTP/1.1 client — POST JSON to the counting-stub.
fn post_chat_completions(base_url: &str, body: &[u8]) -> std::io::Result<()> {
    let url = format!("{}/v1/chat/completions", base_url.trim_end_matches('/'));
    let (host, port, path) = parse_url(&url);
    let addr = (host.as_str(), port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::AddrNotAvailable, "no addr"))?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;

    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n",
        len = body.len()
    );
    stream.write_all(request.as_bytes())?;
    stream.write_all(body)?;

    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;

    let ok_200 = response.starts_with(b"HTTP/1.1 200") || response.starts_with(b"HTTP/1.0 200");
    if !ok_200 {
        let head: String =
            String::from_utf8_lossy(&response[..response.len().min(256)]).into_owned();
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("counting-stub returned non-200: {head}"),
        ));
    }
    Ok(())
}

fn read_counter(base_url: &str) -> std::io::Result<u32> {
    let url = format!("{}/_count", base_url.trim_end_matches('/'));
    let (host, port, path) = parse_url(&url);
    let addr = (host.as_str(), port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::AddrNotAvailable, "no addr"))?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;

    let request = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes())?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;
    let text = String::from_utf8_lossy(&response).into_owned();
    let body = text
        .split("\r\n\r\n")
        .nth(1)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "no body"))?;
    let parsed: serde_json::Value = serde_json::from_str(body).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("counter JSON: {e}"),
        )
    })?;
    parsed["calls"]
        .as_u64()
        .map(|n| n as u32)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "no calls field"))
}

fn parse_url(url: &str) -> (String, u16, String) {
    let url = url.strip_prefix("http://").unwrap_or(url);
    let (authority, path) = url.split_once('/').unwrap_or((url, ""));
    let (host, port) = if let Some((h, p)) = authority.split_once(':') {
        (h.to_string(), p.parse().unwrap_or(8765))
    } else {
        (authority.to_string(), 80)
    };
    let path = format!("/{path}");
    (host, port, path)
}
