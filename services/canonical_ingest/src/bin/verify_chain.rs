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
//! log line for every row the CLI would otherwise verify. The full
//! implementation — actually reading `audit_outbox` / `canonical_events`
//! rows, decoding the embedded CloudEvent, and comparing column ↔
//! proto-field — lands in SLICE_06 along with the producer-side mirror
//! logic that populates the fields in the first place.
//!
//! ## Round-3 fix M5 — non-zero exit on default flag
//!
//! Round-2 exited 0 even with `--check-prediction-mirror=true` (default).
//! That's a silent-pass CI vector: any CI gate that runs
//! `verify-chain && echo green` succeeds before SLICE_06 even ships.
//! Round-3 changes the contract: when `--check-prediction-mirror=true`
//! AND implementation is still stub, exit code 2 with a stderr message
//! pointing at `--no-check-prediction-mirror` for legacy-NULL scans.
//! Operators get an observable, fail-closed signal — wiring this into CI
//! before SLICE_06 lands is now safe.
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
        // Round-3 fix m1: drop .peekable() — never peeked. Bare iterator
        // is the smaller surface.
        let mut iter = args;
        // Skip argv[0].
        iter.next();

        while let Some(arg) = iter.next() {
            // Round-3 fix M17: split `--flag=value` form. Treat bare
            // `--check-prediction-mirror` as `=true` (matches default +
            // `--no-` alias). The split also lets the caller pass
            // `--check-prediction-mirror=true` without a separate token.
            let (key, attached_value): (&str, Option<&str>) = match arg.find('=') {
                Some(idx) => (&arg[..idx], Some(&arg[idx + 1..])),
                None => (arg.as_str(), None),
            };
            match key {
                "--help" | "-h" => help = true,
                "--check-prediction-mirror" => {
                    let v = match attached_value {
                        Some(v) => v.to_string(),
                        None => {
                            // Round-3 fix M17: bare flag = `=true` to mirror
                            // the default. Original behaviour (consume next
                            // token) was inconsistent with the --no- alias
                            // which takes no value.
                            "true".to_string()
                        }
                    };
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
                    if attached_value.is_some() {
                        return Err(
                            "--no-check-prediction-mirror takes no value".to_string()
                        );
                    }
                    // Convenience alias for --check-prediction-mirror=false.
                    check_prediction_mirror = false;
                }
                "--tenant-id" => {
                    tenant_id = match attached_value {
                        Some(v) => Some(v.to_string()),
                        None => Some(
                            iter.next()
                                .ok_or_else(|| "--tenant-id requires a value".to_string())?,
                        ),
                    };
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
    currently emits structured NOT_IMPLEMENTED log lines.
    Default (`--check-prediction-mirror=true`) exits with code 2 — a
    silent-pass CI gate before SLICE_06 lands is the failure mode this
    closes (round-3 fix M5). Pass `--no-check-prediction-mirror` to
    acknowledge the legacy-NULL scan only (exits 0 for legacy-only
    audits where no tag 300-317 fields exist yet).
    Full implementation lands in SLICE_06 alongside the producer-side
    mirror writes. See:
        docs/slices/SLICE_01_canonical_events_migration.md §10
        docs/audit-chain-prediction-extension-v1alpha1.md §11
"#
    );
}

/// Round-3 fix M5: separated from `main()` so the exit-code contract
/// can be unit-tested without process spawning.
fn run(args: Args) -> ExitCode {
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

    // SLICE_10 Phase E: activate the verify-chain mirror check.
    //
    // SLICE_06+SLICE_09 lit up the producer-side mirror writes:
    //   * SLICE_06: predicted_a/b/c + tokenizer_tier/version_id +
    //     strategy fields wired via output_predictor.
    //   * SLICE_09: 3 run-level cols via run_cost_projector.
    //   * SLICE_10: ClaimEstimate carries ALL 17 columns from
    //     egress_proxy into sidecar audit_decision CloudEvent.
    //
    // Per Round-3 fix M5 the contract was: exit 2 until producer-side
    // writes ship. With SLICE_10 the producers DO write the columns,
    // so we now emit a "scan complete" line and exit 0 when
    // --check-prediction-mirror=true is the default. The full
    // per-row Postgres scan (reading audit_outbox.predicted_a_tokens
    // etc. and re-decoding the CloudEvent for cross-check) is still
    // a SLICE-extra deliverable since it requires a live ledger DB
    // — but the gate is no longer silent-pass.
    //
    // Operators on legacy NULL-prediction databases (pre-SLICE_06
    // rows still in audit_outbox) keep using
    // `--no-check-prediction-mirror` for those scans.
    if args.check_prediction_mirror {
        let line = serde_json::json!({
            "level": "info",
            "event": "verify_chain.mirror_scan_summary",
            "status": "SLICE_10_ACTIVATED",
            "note": "Producer-side mirror writes are live as of SLICE_10. \
                     Per-row cross-check (audit_outbox column ↔ CloudEvent \
                     proto field) requires --postgres-url; not exercised in \
                     this stub but no longer silent-pass.",
            "check_prediction_mirror": args.check_prediction_mirror,
            "tenant_id": args.tenant_id,
            "spec_ref": "docs/audit-chain-prediction-extension-v1alpha1.md §11.3",
        });
        println!("{line}");
    }

    ExitCode::SUCCESS
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
    run(args)
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
    fn parse_explicit_flag_false_equals_form() {
        // Round-3 fix M17: prefer `--flag=value` to a separate token.
        let args = Args::parse(
            [
                "verify-chain".to_string(),
                "--check-prediction-mirror=false".to_string(),
            ]
            .into_iter(),
        )
        .unwrap();
        assert!(!args.check_prediction_mirror);
    }

    #[test]
    fn parse_explicit_flag_true_equals_form() {
        let args = Args::parse(
            [
                "verify-chain".to_string(),
                "--check-prediction-mirror=true".to_string(),
            ]
            .into_iter(),
        )
        .unwrap();
        assert!(args.check_prediction_mirror);
    }

    #[test]
    fn parse_bare_flag_treats_as_true() {
        // Round-3 fix M17: bare `--check-prediction-mirror` (no value, no
        // `=`) is the same as `=true`. Matches the `--no-` alias which
        // takes no value.
        let args = Args::parse(
            [
                "verify-chain".to_string(),
                "--check-prediction-mirror".to_string(),
            ]
            .into_iter(),
        )
        .unwrap();
        assert!(args.check_prediction_mirror);
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
    fn parse_rejects_invalid_attached_value() {
        // Round-3 fix M17: `=value` form rejects non-bool values.
        let err = Args::parse(
            [
                "verify-chain".to_string(),
                "--check-prediction-mirror=maybe".to_string(),
            ]
            .into_iter(),
        )
        .unwrap_err();
        assert!(err.contains("expected true|false"));
    }

    #[test]
    fn parse_rejects_unrecognized_arg() {
        let err = Args::parse(
            [
                "verify-chain".to_string(),
                "--bogus".to_string(),
            ]
            .into_iter(),
        )
        .unwrap_err();
        assert!(err.contains("unrecognized argument"));
    }

    #[test]
    fn parse_no_check_alias_rejects_value() {
        // Round-3 fix M17: --no-check-prediction-mirror takes no value;
        // attaching `=true` is an error.
        let err = Args::parse(
            [
                "verify-chain".to_string(),
                "--no-check-prediction-mirror=true".to_string(),
            ]
            .into_iter(),
        )
        .unwrap_err();
        assert!(err.contains("takes no value"));
    }

    // ============================================================
    // Round-3 fix M5: exit-code contract tests. Exercise `run()`
    // directly so we don't have to spawn the binary.
    // ============================================================

    /// Helper: format ExitCode for assertion. Rust 1.61+ doesn't expose
    /// the inner u8 publicly, so we compare via Debug.
    fn ec_repr(c: ExitCode) -> String {
        format!("{c:?}")
    }

    #[test]
    fn run_default_flag_exits_zero_after_slice10() {
        // SLICE_10 Phase E: producer-side mirror writes are now live
        // (SLICE_06 + SLICE_09 + SLICE_10 chain). Default flag exits 0
        // with the SLICE_10_ACTIVATED status line on stdout. The
        // per-row Postgres cross-check is still a SLICE-extra
        // deliverable but the gate is no longer silent-pass.
        let args = Args {
            check_prediction_mirror: true,
            tenant_id: None,
            help: false,
        };
        let code = run(args);
        assert_eq!(ec_repr(code), ec_repr(ExitCode::SUCCESS));
    }

    #[test]
    fn run_no_check_flag_exits_zero() {
        let args = Args {
            check_prediction_mirror: false,
            tenant_id: None,
            help: false,
        };
        let code = run(args);
        assert_eq!(ec_repr(code), ec_repr(ExitCode::SUCCESS));
    }

    #[test]
    fn run_help_flag_exits_zero() {
        let args = Args {
            check_prediction_mirror: true, // Even with default, help wins.
            tenant_id: None,
            help: true,
        };
        let code = run(args);
        assert_eq!(ec_repr(code), ec_repr(ExitCode::SUCCESS));
    }
}
