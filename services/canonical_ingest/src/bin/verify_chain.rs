//! `verify-chain` CLI — audit-chain replay verifier.
//!
//! Spec: `docs/audit-chain-prediction-extension-v1alpha1.md` §11 + Trace
//! schema spec §11.1 (existing baseline) + §11.2 (new cross-storage
//! consistency check) + §11.3 (`--check-prediction-mirror` flag).
//!
//! ## Round-2 fix B3 — scope of this scaffold
//!
//! SLICE_01 lands the **schema substrate + CLI flag scaffolding**, NOT
//! the full mirror-check implementation. The flag is parsed, validated,
//! and routed; the per-row scan path emits a structured `NOT_IMPLEMENTED`
//! log line for every row the CLI would otherwise verify, then exits 0
//! (idempotent / observable / safe in CI gates that run a smoke test of
//! the binary). The full implementation — actually reading
//! `audit_outbox` / `canonical_events` rows, decoding the embedded
//! CloudEvent, and comparing column ↔ proto-field — lands in SLICE_06
//! along with the producer-side mirror logic that populates the fields
//! in the first place.
//!
//! ## Stopping rule rationale
//!
//! Round-1 review (Codex finding B3) flagged the absent flag as a
//! blocker because subsequent slices cannot depend on a CLI shape that
//! has not been declared. By landing the scaffold here we:
//!
//!   1. Lock the flag name (`--check-prediction-mirror`) and default
//!      semantics (`true`) so SLICE_06 callers can reason about it.
//!   2. Avoid scope creep into the full mirror-check implementation
//!      which requires producer-side mirror writes that do not yet
//!      exist.
//!   3. Keep round-2 atomic: schema fix + flag scaffold, no
//!      half-implemented verification.
//!
//! The deferral is documented in `docs/slices/SLICE_01_canonical_events_migration.md`
//! §10 and is consistent with the round-1 deferred-items rule.

use std::process::ExitCode;

/// CLI argument shape — kept inline to avoid a `clap` dependency in
/// SLICE_01. SLICE_06 may migrate to `clap::Parser` when the full
/// implementation lands.
#[derive(Debug)]
struct Args {
    /// Whether to run the SLICE_06 mirror cross-check.
    /// Default `true` for new versions (per spec §11.3).
    /// Set to `false` to verify legacy NULL-prediction rows.
    check_prediction_mirror: bool,

    /// Tenant filter (optional). When set, only rows for this tenant
    /// are scanned. Mirrors the existing verify-chain baseline.
    tenant_id: Option<String>,

    /// Whether the operator passed `--help` (handled below).
    help: bool,
}

impl Args {
    fn parse(args: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut check_prediction_mirror = true;
        let mut tenant_id: Option<String> = None;
        let mut help = false;
        let mut iter = args.peekable();
        // Skip argv[0].
        iter.next();

        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--help" | "-h" => help = true,
                "--check-prediction-mirror" => {
                    let v = iter.next().ok_or_else(|| {
                        "--check-prediction-mirror requires a value (true|false)".to_string()
                    })?;
                    check_prediction_mirror = match v.as_str() {
                        "true" | "1" | "yes" => true,
                        "false" | "0" | "no" => false,
                        other => {
                            return Err(format!(
                                "--check-prediction-mirror: expected true|false, got {other:?}"
                            ));
                        }
                    };
                }
                "--no-check-prediction-mirror" => {
                    // Convenience alias for --check-prediction-mirror=false.
                    check_prediction_mirror = false;
                }
                "--tenant-id" => {
                    tenant_id = Some(
                        iter.next()
                            .ok_or_else(|| "--tenant-id requires a value".to_string())?,
                    );
                }
                other => {
                    return Err(format!("unrecognized argument: {other:?}"));
                }
            }
        }

        Ok(Self {
            check_prediction_mirror,
            tenant_id,
            help,
        })
    }
}

fn print_help() {
    println!(
        r#"verify-chain — SpendGuard audit-chain replay verifier (SLICE_01 stub)

USAGE:
    verify-chain [OPTIONS]

OPTIONS:
    --check-prediction-mirror <true|false>
        Enable the cross-storage prediction-mirror check
        (per audit-chain-prediction-extension-v1alpha1.md §11.2 + §11.3).
        Default: true.

    --no-check-prediction-mirror
        Alias for --check-prediction-mirror=false. Use for legacy
        NULL-prediction rows where the mirror does not yet apply.

    --tenant-id <uuid>
        Scan only rows for the given tenant_id.

    -h, --help
        Print this help and exit.

STATUS:
    SLICE_01 ships the flag scaffold only. The per-row scan path
    currently emits structured NOT_IMPLEMENTED log lines and exits 0.
    Full implementation lands in SLICE_06 alongside the producer-side
    mirror writes. See:
        docs/slices/SLICE_01_canonical_events_migration.md §10
        docs/audit-chain-prediction-extension-v1alpha1.md §11
"#
    );
}

fn main() -> ExitCode {
    let args = match Args::parse(std::env::args()) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("verify-chain: {e}\n");
            print_help();
            return ExitCode::from(2);
        }
    };

    if args.help {
        print_help();
        return ExitCode::SUCCESS;
    }

    // Single structured log line per row scan (stdout JSONL).
    // In SLICE_01 we emit one synthetic line so smoke tests can assert
    // the stub is wired without depending on a Postgres connection.
    // SLICE_06 will replace this with the real scan loop.
    let line = serde_json::json!({
        "level": "info",
        "event": "verify_chain.row_scan",
        "status": "NOT_IMPLEMENTED",
        "reason": "SLICE_01 ships flag scaffold only; full mirror check lands in SLICE_06",
        "check_prediction_mirror": args.check_prediction_mirror,
        "tenant_id": args.tenant_id,
        "spec_ref": "docs/audit-chain-prediction-extension-v1alpha1.md §11.3",
    });
    println!("{line}");

    // Exit 0 — the stub completes "successfully" because the absent
    // check is documented and expected. Operators wiring this into CI
    // before SLICE_06 lands get an observable signal (the
    // NOT_IMPLEMENTED log line) rather than a silent no-op.
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_default_flag_is_true() {
        let args = Args::parse(["verify-chain".to_string()].into_iter()).unwrap();
        assert!(args.check_prediction_mirror);
    }

    #[test]
    fn parse_explicit_flag_false() {
        let args = Args::parse(
            [
                "verify-chain".to_string(),
                "--check-prediction-mirror".to_string(),
                "false".to_string(),
            ]
            .into_iter(),
        )
        .unwrap();
        assert!(!args.check_prediction_mirror);
    }

    #[test]
    fn parse_no_check_alias() {
        let args = Args::parse(
            [
                "verify-chain".to_string(),
                "--no-check-prediction-mirror".to_string(),
            ]
            .into_iter(),
        )
        .unwrap();
        assert!(!args.check_prediction_mirror);
    }

    #[test]
    fn parse_tenant_id() {
        let args = Args::parse(
            [
                "verify-chain".to_string(),
                "--tenant-id".to_string(),
                "00000000-0000-4000-8000-000000000001".to_string(),
            ]
            .into_iter(),
        )
        .unwrap();
        assert_eq!(
            args.tenant_id.as_deref(),
            Some("00000000-0000-4000-8000-000000000001")
        );
    }

    #[test]
    fn parse_rejects_unrecognized() {
        let err = Args::parse(
            [
                "verify-chain".to_string(),
                "--check-prediction-mirror".to_string(),
                "maybe".to_string(),
            ]
            .into_iter(),
        )
        .unwrap_err();
        assert!(err.contains("expected true|false"));
    }

    #[test]
    fn parse_rejects_missing_value() {
        let err = Args::parse(
            [
                "verify-chain".to_string(),
                "--check-prediction-mirror".to_string(),
            ]
            .into_iter(),
        )
        .unwrap_err();
        assert!(err.contains("requires a value"));
    }
}
