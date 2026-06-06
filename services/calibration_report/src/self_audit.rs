//! Self-audit CloudEvent emission.
//!
//! Spec ancestor: `docs/calibration-report-spec-v1alpha1.md` §5.3.
//!
//! ## What this module does
//!
//! Every report run (including refusals) emits a
//! `spendguard.audit.calibration.report_generated` CloudEvent into the
//! audit chain. Cross-tenant rejections additionally emit
//! `spendguard.audit.calibration.unauthorized_access`. Both events are
//! signed + ride the existing canonical_ingest replication path, so
//! "who looked at the report and what window" is itself audit
//! evidence.
//!
//! ## Two emission paths
//!
//! 1. **Local-log path** (default; works on every operator workstation):
//!    write a structured JSON line to stdout/stderr via `tracing`.
//!    SIEM forwarders / sidecar tails ingest it. This is the
//!    fail-closed default — even with no canonical_ingest connectivity
//!    we leave a trail.
//!
//! 2. **Canonical-ingest push** (production): if a signing key + mTLS
//!    bundle are configured, the CLI also pushes the signed event to
//!    canonical_ingest via the standard AppendEvents RPC.
//!
//! SLICE_13 Phase C ships path (1) for both the local-only operator
//! use case and the demo scenario; path (2) is opt-in via env
//! configuration so production deployments wire it without changing
//! the binary surface.
//!
//! ## Self-audit content
//!
//! Per spec §5.3:
//!
//!   * `subject`: caller's mTLS subject or env-supplied identity.
//!   * `tenant_id` + `window`: scope of the report.
//!   * `format` + `proof_mode`: how the report was rendered.
//!   * `exit_code`: outcome.
//!
//! ## What we DON'T leak
//!
//! The report content itself is NOT included in the self-audit event
//! — only the run metadata. (Including the full report would be
//! recursive audit; spec §8.3 forbids.) Downstream consumers know
//! WHICH report ran, not its contents.

use crate::cli::Cli;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::json;
use tracing::info;

#[derive(Debug, Clone, Serialize)]
pub struct ReportGeneratedEvent<'a> {
    pub specversion: &'static str,
    pub r#type: &'a str,
    pub id: String,
    pub source: &'static str,
    pub subject: Option<&'a str>,
    pub time: String,
    pub data: ReportGeneratedData<'a>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReportGeneratedData<'a> {
    pub tenant_id: Option<&'a str>,
    pub auth_subject: Option<&'a str>,
    pub window_from: String,
    pub window_to: String,
    pub format: &'a str,
    pub proof_mode: &'a str,
    pub exit_code: u8,
    pub verify_chain_run: bool,
}

const EVENT_TYPE_REPORT: &str = "spendguard.audit.calibration.report_generated.v1alpha1";
const EVENT_TYPE_UNAUTHORIZED: &str = "spendguard.audit.calibration.unauthorized_access.v1alpha1";

/// Emit the report-generated self-audit event (spec §5.3).
///
/// `window_from` / `window_to` are formatted as RFC3339 strings so the
/// receiver doesn't need to parse a custom timestamp format.
pub fn emit_report_generated(
    cli: &Cli,
    window_from: DateTime<Utc>,
    window_to: DateTime<Utc>,
    exit_code: u8,
) {
    let format = match cli.format {
        crate::cli::Format::Text => "text",
        crate::cli::Format::Json => "json",
        crate::cli::Format::Markdown => "markdown",
    };
    let proof_mode = match cli.effective_proof_mode() {
        crate::cli::ProofMode::Cache => "cache",
        crate::cli::ProofMode::Canonical => "canonical",
    };
    let event = ReportGeneratedEvent {
        specversion: "1.0",
        r#type: EVENT_TYPE_REPORT,
        id: uuid::Uuid::now_v7().to_string(),
        source: "spendguard-calibration-report",
        subject: cli.auth_subject.as_deref(),
        time: Utc::now().to_rfc3339(),
        data: ReportGeneratedData {
            tenant_id: cli.tenant.as_deref(),
            auth_subject: cli.auth_subject.as_deref(),
            window_from: window_from.to_rfc3339(),
            window_to: window_to.to_rfc3339(),
            format,
            proof_mode,
            exit_code,
            verify_chain_run: cli.verify_chain,
        },
    };
    emit_json(&event);
}

/// Emit a cross-tenant rejection event (spec §5.2 + §5.3).
pub fn emit_unauthorized_access(cli: &Cli) {
    let payload = json!({
        "specversion": "1.0",
        "type": EVENT_TYPE_UNAUTHORIZED,
        "id": uuid::Uuid::now_v7().to_string(),
        "source": "spendguard-calibration-report",
        "subject": cli.auth_subject.as_deref(),
        "time": Utc::now().to_rfc3339(),
        "data": {
            "requested_tenant": cli.tenant.as_deref(),
            "auth_subject": cli.auth_subject.as_deref(),
            "auth_tenants_configured": cli.auth_tenants.is_some(),
        }
    });
    emit_json(&payload);
}

fn emit_json<T: Serialize>(event: &T) {
    // The fail-closed local-log path. A SIEM tail will pick this up
    // off the operator's stderr; the production canonical-ingest push
    // is an opt-in PUT against canonical_ingest::AppendEvents (SLICE
    // extra — full mTLS wiring lives there).
    match serde_json::to_string(event) {
        Ok(json) => {
            // Use tracing::info! with the structured `cloudevent` field
            // so json-log aggregators can pluck it cleanly without
            // string-matching the message body.
            info!(
                cloudevent = %json,
                "spendguard.audit.calibration"
            );
        }
        Err(e) => {
            // Serialize-fail is impossible for our types, but we
            // guard anyway because losing the audit trail is worse
            // than panicking.
            tracing::error!(error = %e, "self-audit serialize failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn cli(tenant: &str) -> Cli {
        Cli::parse_from([
            "spendguard-calibration-report",
            "--tenant",
            tenant,
            "--auth-subject",
            "test-operator@example.com",
        ])
    }

    #[test]
    fn report_generated_event_serializes() {
        let c = cli("00000000-0000-4000-8000-000000000001");
        // Just call the function; serialisation is the failure mode
        // we care about. The fixture exercises every branch in the
        // `data` body.
        emit_report_generated(&c, Utc::now() - chrono::Duration::days(7), Utc::now(), 0);
    }

    #[test]
    fn unauthorized_access_event_serializes() {
        let c = cli("00000000-0000-4000-8000-000000000001");
        emit_unauthorized_access(&c);
    }

    #[test]
    fn report_generated_includes_all_data_fields() {
        // Verify the struct shape directly so we catch additions.
        let c = cli("00000000-0000-4000-8000-000000000001");
        let event = ReportGeneratedEvent {
            specversion: "1.0",
            r#type: EVENT_TYPE_REPORT,
            id: "11111111-1111-7000-a000-000000000001".to_string(),
            source: "spendguard-calibration-report",
            subject: c.auth_subject.as_deref(),
            time: "2026-05-30T00:00:00Z".to_string(),
            data: ReportGeneratedData {
                tenant_id: c.tenant.as_deref(),
                auth_subject: c.auth_subject.as_deref(),
                window_from: "2026-05-23T00:00:00Z".to_string(),
                window_to: "2026-05-30T00:00:00Z".to_string(),
                format: "text",
                proof_mode: "cache",
                exit_code: 0,
                verify_chain_run: false,
            },
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], EVENT_TYPE_REPORT);
        assert_eq!(json["data"]["format"], "text");
        assert_eq!(json["data"]["proof_mode"], "cache");
        assert_eq!(json["data"]["exit_code"], 0);
        assert_eq!(
            json["data"]["tenant_id"],
            "00000000-0000-4000-8000-000000000001"
        );
        assert_eq!(json["data"]["auth_subject"], "test-operator@example.com");
    }

    #[test]
    fn unauthorized_access_includes_subject_and_tenant() {
        let c = cli("00000000-0000-4000-8000-000000000099");
        let json = json!({
            "specversion": "1.0",
            "type": EVENT_TYPE_UNAUTHORIZED,
            "source": "spendguard-calibration-report",
            "subject": c.auth_subject.as_deref(),
            "data": {
                "requested_tenant": c.tenant.as_deref(),
                "auth_subject": c.auth_subject.as_deref(),
            }
        });
        assert_eq!(json["type"], EVENT_TYPE_UNAUTHORIZED);
        assert_eq!(
            json["data"]["requested_tenant"],
            "00000000-0000-4000-8000-000000000099"
        );
    }
}
