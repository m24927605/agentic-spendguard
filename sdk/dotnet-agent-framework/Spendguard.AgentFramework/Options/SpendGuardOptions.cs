// SPDX-License-Identifier: Apache-2.0
// Copyright (c) SpendGuard Authors.

using System;

namespace Spendguard.AgentFramework.Options;

/// <summary>
/// Behavior selector for the sidecar-unavailable case.
/// Default is <see cref="Deny"/> per design.md ADR-005 (fail-closed).
/// </summary>
public enum OnSidecarUnavailable
{
    /// <summary>
    /// Fail closed (default). When the sidecar UDS endpoint is unreachable or
    /// the handshake fails, raise an exception instead of allowing the LLM call
    /// through unaudited.
    /// </summary>
    Deny = 0,

    /// <summary>
    /// Explicit opt-in only. Allow the LLM call through unaudited when the
    /// sidecar is unreachable, after emitting a logged warning. Reviewers
    /// (Sec3) flag every use site.
    /// </summary>
    Allow = 1,
}

/// <summary>
/// Caller-visible options for <c>SpendGuardChatMiddleware</c>.
/// Mirrors the Python <c>SpendGuardMiddleware</c> options surface
/// per review-standards.md §2.3 P2 (cross-language naming parity).
/// </summary>
public sealed class SpendGuardOptions
{
    /// <summary>
    /// Filesystem path to the sidecar Unix Domain Socket.
    /// Matches the default ASP sidecar bind in
    /// <c>deploy/sidecar/spendguard-sidecar.yaml</c>.
    /// </summary>
    public string SidecarSocketPath { get; set; } = "/var/run/spendguard/adapter.sock";

    /// <summary>
    /// Tenant identifier asserted at handshake. Sidecar verifies against
    /// <c>SO_PEERCRED</c> + signed manifest per Sidecar Architecture spec §5.
    /// </summary>
    public string TenantId { get; set; } = string.Empty;

    /// <summary>
    /// Budget identifier. Required at registration time.
    /// </summary>
    public string BudgetId { get; set; } = string.Empty;

    /// <summary>
    /// Workload instance identifier (sidecar's own). Echoed at handshake for
    /// sanity check (Handshake.workload_instance_id, adapter.proto §Handshake).
    /// </summary>
    public string WindowInstanceId { get; set; } = string.Empty;

    /// <summary>
    /// Behavior when the sidecar is unreachable. Default is
    /// <see cref="OnSidecarUnavailable.Deny"/> (fail-closed).
    /// </summary>
    public OnSidecarUnavailable OnSidecarUnavailable { get; set; }
        = OnSidecarUnavailable.Deny;

    /// <summary>
    /// SDK version string sent at handshake. Set automatically to the package
    /// version; consumers should not override unless running an internal fork.
    /// </summary>
    public string SdkVersion { get; set; } = "0.1.0-pre";

    /// <summary>
    /// Runtime kind sent at handshake (per <c>HandshakeRequest.runtime_kind</c>).
    /// Default identifies this adapter so the sidecar can route capability
    /// negotiation correctly.
    /// </summary>
    public string RuntimeKind { get; set; } = "microsoft-agent-framework-dotnet";

    /// <summary>
    /// Canonical-truth UUID of the ledger unit row (FK to
    /// <c>ledger_units.unit_id</c>). When set, the adapter threads it
    /// through to <c>DecisionRequest.Inputs.ProjectedUnit.UnitId</c> on the
    /// wire so the sidecar ledger can resolve the budget claim. Most
    /// operators source this from the <c>SPENDGUARD_UNIT_ID</c> env var at
    /// adapter construction time.
    ///
    /// Omitting leaves the wire field empty and the ledger will reject the
    /// reserve with <c>INVALID_REQUEST: claim[N].unit.unit_id empty</c> —
    /// recipe-style integrations (no ledger reserve) MAY omit. NB: this is
    /// the ledger UUID, distinct from any free-form unit slug — they are
    /// NOT interchangeable.
    ///
    /// Additive optional field shipped under HARDEN_D05_UR.
    /// </summary>
    public Guid? UnitId { get; set; }

    /// <summary>
    /// Convenience <c>token_kind</c> for the projected unit (e.g.
    /// <c>output_token</c>). Canonical truth is <see cref="UnitId"/>.
    /// </summary>
    public string UnitTokenKind { get; set; } = "output_token";

    /// <summary>Convenience <c>model_family</c> for the projected unit.</summary>
    public string UnitModelFamily { get; set; } = "gpt-4";

    /// <summary>
    /// Per-call projected claim amount (atomic units of <see cref="UnitId"/>).
    /// Drives the LLM_CALL_PRE reservation. When null the adapter falls back to
    /// the token estimate. The demo sets it per turn (tiny ALLOW claim, a
    /// contract-cap-busting DENY claim).
    /// </summary>
    public long? ProjectedClaimAmountAtomic { get; set; }

    /// <summary>
    /// PricingFreeze tuple for the commit (LLM_CALL_POST). The sidecar
    /// validates equality against <c>ledger.pricing_snapshots</c>, so these
    /// MUST match the contract bundle the sidecar loaded (operators source
    /// them from the bundles <c>runtime.env</c>). Required for a real commit.
    /// </summary>
    public string PricingVersion { get; set; } = string.Empty;

    /// <summary>Hex-encoded sha256 of the price snapshot (PricingFreeze.price_snapshot_hash).</summary>
    public string PriceSnapshotHashHex { get; set; } = string.Empty;

    /// <summary>PricingFreeze.fx_rate_version.</summary>
    public string FxRateVersion { get; set; } = string.Empty;

    /// <summary>PricingFreeze.unit_conversion_version.</summary>
    public string UnitConversionVersion { get; set; } = string.Empty;

    /// <summary>
    /// Validates this option bag. Throws <see cref="ArgumentException"/>
    /// on any disallowed combination. Called by DI registration so misconfig
    /// surfaces at startup, not on the hot path.
    /// </summary>
    public void Validate()
    {
        if (string.IsNullOrWhiteSpace(BudgetId))
        {
            throw new ArgumentException(
                "SpendGuardOptions.BudgetId is required (review-standards N1).",
                nameof(BudgetId));
        }

        if (string.IsNullOrWhiteSpace(SidecarSocketPath))
        {
            throw new ArgumentException(
                "SpendGuardOptions.SidecarSocketPath is required (review-standards N2).",
                nameof(SidecarSocketPath));
        }

        if (string.IsNullOrWhiteSpace(TenantId))
        {
            throw new ArgumentException(
                "SpendGuardOptions.TenantId is required for sidecar handshake.",
                nameof(TenantId));
        }
    }
}
