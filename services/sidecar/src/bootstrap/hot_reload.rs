//! CA-P3.7 — contract bundle hot-reload watcher.
//!
//! Closes the last gap in the Cost Advisor closed loop. Before this
//! module, the sidecar loaded its contract bundle ONCE at startup
//! (`bootstrap::bundles::install_contract_bundle`); a bundle rotation
//! by `services/bundle_registry` was invisible to the running sidecar
//! until an operator-driven restart. This watcher polls
//! `runtime.env` for hash changes and atomically swaps the cached
//! bundle in-place when a new hash is published, completing the
//! cost_advisor → operator-approve → bundle_registry → sidecar
//! feedback loop.
//!
//! ## Why poll (vs inotify)
//!
//! `bundles-data` is a shared docker volume in the demo and a
//! `ReadWriteOnce` PV in Helm. inotify events on shared/network
//! volumes are unreliable in practice — particularly across the
//! Docker for Mac VFS boundary and across volume-fanout writes that
//! happen via NFS/CSI underneath. Polling `runtime.env` (a tiny file,
//! O(100B)) every 500ms is bounded, predictable, and observable.
//!
//! ## What we watch
//!
//! The authoritative pointer is `runtime.env`'s
//! `SPENDGUARD_SIDECAR_CONTRACT_BUNDLE_HASH_HEX=<sha256-hex>` line.
//! `bundle_registry::bundle::update_runtime_env` rewrites this line
//! LAST in its atomic publish sequence (`.tgz` → `.sig` → `runtime.env`)
//! so observing a new hash means the new `.tgz` is already on disk
//! and durable (codex CA-P3.5 r1 P2 design). Watching the `.tgz`
//! file's mtime would be a TOCTOU race: the writer is mid-publish.
//!
//! ## Reload semantics
//!
//! - Same `bundle_id`, different bytes: re-load via the existing
//!   `load_contract_bundle` (which re-verifies the sha256 against the
//!   new hash + re-parses contract.yaml) + `install_contract_bundle`
//!   (which writes the new Arc into `state.inner.contract_bundle` via
//!   `RwLock`).
//! - Fail-closed on parse or hash mismatch: keep the previously
//!   installed bundle, log structured + bump a counter. The sidecar
//!   never serves decisions against a half-loaded bundle.
//! - In-flight decisions are naturally pinned: the hot path does
//!   `state.inner.contract_bundle.read().clone()` which clones the
//!   `CachedContractBundle` (whose `parsed: SharedContract` is
//!   `Arc<Contract>`). Once a request has its clone, a swap does not
//!   affect it. This matches Contract §pinning ("In-flight requests
//!   use pinned old bundle").

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use tracing::{debug, info, warn};

use crate::bootstrap::bundles::{
    install_contract_bundle, load_contract_bundle, BundleSource,
};
use crate::config::Config;
use crate::domain::state::SidecarState;

const RUNTIME_ENV_HASH_KEY: &str = "SPENDGUARD_SIDECAR_CONTRACT_BUNDLE_HASH_HEX";

/// Spawn a background task that polls `runtime.env` and reloads the
/// active contract bundle when its hash changes. Returns an error if
/// the watcher cannot be started at all (e.g., the configured
/// `contract_bundle_id` is not a valid UUID — a fail-fast startup
/// misconfiguration rather than a silent runtime degradation, per
/// codex CA-P3.7 r1 P1-2).
///
/// Polling cadence: 500ms. The latency budget for "operator approves
/// in dashboard → sidecar uses new contract" is ~2s end-to-end in the
/// demo (NOTIFY → bundle_registry apply → file write → watcher poll →
/// atomic swap). Tightening this is cheap but the bottleneck is
/// usually elsewhere (NOTIFY + tar repack); 500ms keeps the watcher
/// below the noise floor of upstream latencies.
///
/// Blocking I/O safety: `runtime.env` is read async via
/// `tokio::fs::read_to_string` (tiny file, hot path). The `.tgz`
/// re-read + sha256 + tarball parse only happens on a hash mismatch,
/// and is wrapped in `spawn_blocking` so it does not stall a tokio
/// worker thread for the 10–30ms it takes (codex CA-P3.7 r1 P1-1).
pub fn spawn_loop(cfg: &Config, state: SidecarState) -> Result<()> {
    let runtime_env_path = PathBuf::from(&cfg.runtime_env_path);
    let bundle_root = PathBuf::from(&cfg.bundle_root);
    // Parse the bundle_id up front so a misconfigured Helm value
    // surfaces as a startup failure (the operator's job to fix)
    // rather than silently disabling hot-reload at runtime. The
    // sidecar `install_bundles` path parsed the same string at
    // startup with `?`; this is the second eye on the same value.
    let bundle_id = uuid::Uuid::parse_str(&cfg.contract_bundle_id)
        .with_context(|| {
            format!(
                "CA-P3.7: SPENDGUARD_SIDECAR_CONTRACT_BUNDLE_ID is not a valid UUID: {}",
                cfg.contract_bundle_id
            )
        })?;
    let poll_interval = Duration::from_millis(cfg.hot_reload_poll_ms);

    info!(
        runtime_env = %runtime_env_path.display(),
        bundle_root = %bundle_root.display(),
        contract_bundle_id = %bundle_id,
        poll_ms = poll_interval.as_millis() as u64,
        "CA-P3.7: hot-reload watcher starting"
    );

    tokio::spawn(async move {
        let source = BundleSource { root: bundle_root };
        loop {
            tokio::time::sleep(poll_interval).await;
            // Best-effort early exit on drain (not formally
            // drain-coordinated: the watcher is fire-and-forget and
            // the JoinHandle is dropped, so drain::run_drain never
            // awaits it. An in-flight tick may complete a swap after
            // the drain flag is set — harmless because the RPC
            // server is already shutting down).
            if state.is_draining() {
                debug!("hot-reload watcher: sidecar draining, exiting loop");
                return;
            }

            if let Err(e) = tick(&runtime_env_path, &source, bundle_id, &state).await {
                // Tick errors are non-fatal by design — a partial write
                // by bundle_registry is observable as either a missing
                // hash key (warn) or a hash that doesn't match the .tgz
                // (info, retried next tick). Logging at error level
                // would spam the SIEM during a normal rotation race.
                debug!(err = %format!("{:#}", e), "hot-reload tick recoverable error");
            }
        }
    });
    Ok(())
}

/// One poll iteration. Returns Ok(()) when there is nothing to do OR
/// when a reload succeeded; returns Err when the tick aborted for a
/// recoverable reason (file missing, hash not yet matching the .tgz
/// bytes during a partial-write window, etc). Errors are NOT bubbled
/// to the caller — the outer loop logs them at debug level.
async fn tick(
    runtime_env_path: &Path,
    source: &BundleSource,
    bundle_id: uuid::Uuid,
    state: &SidecarState,
) -> Result<()> {
    let expected_hash_hex = match read_runtime_env_hash_async(runtime_env_path).await? {
        Some(h) => h,
        None => {
            // runtime.env present but no hash key. Don't reload (we'd
            // have no expected hash to verify against). Log + skip.
            return Ok(());
        }
    };
    let current_hash_hex = current_bundle_hash_hex(state);
    if current_hash_hex.as_deref() == Some(expected_hash_hex.as_str()) {
        return Ok(());
    }

    // Mismatch detected — try to load the new bundle on the blocking
    // pool (sync std::fs::read of the multi-KB tarball + sha256 +
    // tar+yaml parse takes 10–30ms; we don't want to stall a tokio
    // worker thread). If anything fails (file partially written, hash
    // still doesn't match the .tgz bytes, contract.yaml structurally
    // invalid), keep the previously installed bundle and surface the
    // failure as a warning so operators can spot apply failures vs
    // steady-state polls.
    let source_clone = source.clone();
    let expected_for_load = expected_hash_hex.clone();
    let load_result = tokio::task::spawn_blocking(move || {
        load_contract_bundle(&source_clone, bundle_id, &expected_for_load)
    })
    .await
    .context("hot-reload load task joined with panic")?;

    let new_bundle = match load_result {
        Ok(b) => b,
        Err(e) => {
            warn!(
                event = "hot_reload_load_failed",
                bundle_id = %bundle_id,
                expected_hash_hex = %expected_hash_hex,
                err = %format!("{:#}", e),
                "hot-reload: new bundle did not verify; keeping previous bundle"
            );
            return Ok(());
        }
    };

    let prev_id = install_contract_bundle(state, new_bundle);
    info!(
        event = "hot_reload_swapped",
        previous_bundle_id = ?prev_id,
        new_bundle_id = %bundle_id,
        new_bundle_hash_hex = %expected_hash_hex,
        previous_bundle_hash_hex = ?current_hash_hex,
        "CA-P3.7: contract bundle hot-reloaded"
    );
    Ok(())
}

/// Async variant of `read_runtime_env_hash` used on the hot path so a
/// slow disk doesn't tie up a tokio worker thread. Most ticks read a
/// ~200B file and return Ok(Some(same_hash)) → no-op.
async fn read_runtime_env_hash_async(path: &Path) -> Result<Option<String>> {
    let contents = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| anyhow::anyhow!("read {}: {}", path.display(), e))?;
    Ok(parse_runtime_env_hash(&contents))
}

/// Read `runtime.env` and extract the
/// SPENDGUARD_SIDECAR_CONTRACT_BUNDLE_HASH_HEX value. Tolerant of:
/// - missing file (Err)
/// - missing key (Ok(None))
/// - trailing whitespace
/// - inline `export ` prefix
/// - quoted values (single or double)
///
/// Does NOT validate that the value is a sha256 hex string — that's
/// `load_contract_bundle`'s job (it hex-decodes against the on-disk
/// .tgz bytes and rejects a mismatch).
pub fn read_runtime_env_hash(path: &Path) -> Result<Option<String>> {
    let contents = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("read {}: {}", path.display(), e))?;
    Ok(parse_runtime_env_hash(&contents))
}

fn parse_runtime_env_hash(contents: &str) -> Option<String> {
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        // Strip an optional `export ` prefix that some operators leave
        // in to support `source runtime.env` from shells.
        let body = trimmed
            .strip_prefix("export ")
            .unwrap_or(trimmed);
        let Some((k, v)) = body.split_once('=') else {
            continue;
        };
        if k.trim() != RUNTIME_ENV_HASH_KEY {
            continue;
        }
        let v = v.trim();
        // Strip wrapping quotes if present (matched pairs only).
        let value = if (v.starts_with('"') && v.ends_with('"') && v.len() >= 2)
            || (v.starts_with('\'') && v.ends_with('\'') && v.len() >= 2)
        {
            &v[1..v.len() - 1]
        } else {
            v
        };
        if value.is_empty() {
            return None;
        }
        return Some(value.to_string());
    }
    None
}

fn current_bundle_hash_hex(state: &SidecarState) -> Option<String> {
    state
        .inner
        .contract_bundle
        .read()
        .as_ref()
        .map(|b| hex::encode(&b.bundle_hash))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_value() {
        let s = "SPENDGUARD_SIDECAR_CONTRACT_BUNDLE_HASH_HEX=abc123\nFOO=bar\n";
        assert_eq!(parse_runtime_env_hash(s).as_deref(), Some("abc123"));
    }

    #[test]
    fn handles_export_prefix() {
        let s = "export SPENDGUARD_SIDECAR_CONTRACT_BUNDLE_HASH_HEX=deadbeef\n";
        assert_eq!(parse_runtime_env_hash(s).as_deref(), Some("deadbeef"));
    }

    #[test]
    fn handles_double_quoted_value() {
        let s = "SPENDGUARD_SIDECAR_CONTRACT_BUNDLE_HASH_HEX=\"feedface\"\n";
        assert_eq!(parse_runtime_env_hash(s).as_deref(), Some("feedface"));
    }

    #[test]
    fn handles_single_quoted_value() {
        let s = "SPENDGUARD_SIDECAR_CONTRACT_BUNDLE_HASH_HEX='cafebabe'\n";
        assert_eq!(parse_runtime_env_hash(s).as_deref(), Some("cafebabe"));
    }

    #[test]
    fn ignores_comments_and_blank_lines() {
        let s = "# header\n\n# SPENDGUARD_SIDECAR_CONTRACT_BUNDLE_HASH_HEX=should_ignore\nSPENDGUARD_SIDECAR_CONTRACT_BUNDLE_HASH_HEX=real\n";
        assert_eq!(parse_runtime_env_hash(s).as_deref(), Some("real"));
    }

    #[test]
    fn missing_key_returns_none() {
        let s = "FOO=bar\nBAZ=qux\n";
        assert_eq!(parse_runtime_env_hash(s), None);
    }

    #[test]
    fn empty_value_returns_none() {
        let s = "SPENDGUARD_SIDECAR_CONTRACT_BUNDLE_HASH_HEX=\n";
        assert_eq!(parse_runtime_env_hash(s), None);
    }

    #[test]
    fn returns_first_match_when_duplicated() {
        // Defensive: real bundle_registry will never duplicate, but a
        // hand-edited file might. First wins (consistent with bash's
        // last-write-wins-when-sourced, since our writer truncates +
        // rewrites and emits exactly one).
        let s = "SPENDGUARD_SIDECAR_CONTRACT_BUNDLE_HASH_HEX=first\nSPENDGUARD_SIDECAR_CONTRACT_BUNDLE_HASH_HEX=second\n";
        assert_eq!(parse_runtime_env_hash(s).as_deref(), Some("first"));
    }

    #[test]
    fn does_not_match_substring_keys() {
        // Guard against accidentally matching keys that merely END
        // with our suffix (no such key today, but defensive).
        let s = "MY_SPENDGUARD_SIDECAR_CONTRACT_BUNDLE_HASH_HEX=wrong\nSPENDGUARD_SIDECAR_CONTRACT_BUNDLE_HASH_HEX=right\n";
        assert_eq!(parse_runtime_env_hash(s).as_deref(), Some("right"));
    }

    #[test]
    fn read_runtime_env_hash_reads_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runtime.env");
        std::fs::write(
            &path,
            "FOO=bar\nSPENDGUARD_SIDECAR_CONTRACT_BUNDLE_HASH_HEX=ondisk\nBAZ=qux\n",
        )
        .unwrap();
        let hash = read_runtime_env_hash(&path).unwrap();
        assert_eq!(hash.as_deref(), Some("ondisk"));
    }

    #[test]
    fn read_runtime_env_hash_propagates_missing_file_as_err() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.env");
        assert!(read_runtime_env_hash(&path).is_err());
    }
}
