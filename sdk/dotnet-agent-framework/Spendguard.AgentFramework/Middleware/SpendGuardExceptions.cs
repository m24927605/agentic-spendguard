// SPDX-License-Identifier: Apache-2.0
// Copyright (c) SpendGuard Authors.

using System;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using Spendguard.SidecarAdapter.V1;

namespace Spendguard.AgentFramework.Middleware;

/// <summary>
/// Raised by <c>SpendGuardChatMiddleware</c> when the sidecar returns a
/// <c>STOP</c> or <c>STOP_RUN_PROJECTION</c> decision. Cross-language parity
/// per review-standards P3 with Python's <c>DecisionDenied</c>.
/// </summary>
public sealed class SpendGuardDecisionDeniedException : Exception
{
    /// <summary>Initialises a new instance with the sidecar decision payload.</summary>
    public SpendGuardDecisionDeniedException(DecisionResponse decision)
        : base(BuildMessage(decision))
    {
        if (decision is null) throw new ArgumentNullException(nameof(decision));

        DecisionId = decision.DecisionId;
        AuditDecisionEventId = decision.AuditDecisionEventId;
        ReasonCodes = new ReadOnlyCollection<string>(new List<string>(decision.ReasonCodes));
        MatchedRuleIds = new ReadOnlyCollection<string>(new List<string>(decision.MatchedRuleIds));
        RunCodeTriggered = decision.RunCodeTriggered ?? string.Empty;
        IsTerminal = decision.Terminal;
        DecisionEnum = decision.Decision;
    }

    /// <summary>Sidecar-issued decision id (UUID v7).</summary>
    public string DecisionId { get; }

    /// <summary>Audit canonical_event id for the deny row.</summary>
    public string AuditDecisionEventId { get; }

    /// <summary>Reason codes emitted by the contract evaluation.</summary>
    public IReadOnlyList<string> ReasonCodes { get; }

    /// <summary>Matched rule ids from the contract bundle.</summary>
    public IReadOnlyList<string> MatchedRuleIds { get; }

    /// <summary>RUN_* code if the deny was driven by run-cost projection.</summary>
    public string RunCodeTriggered { get; }

    /// <summary>True if the contract marked the decision terminal.</summary>
    public bool IsTerminal { get; }

    /// <summary>Underlying decision enum (STOP or STOP_RUN_PROJECTION).</summary>
    public DecisionResponse.Types.Decision DecisionEnum { get; }

    private static string BuildMessage(DecisionResponse d)
    {
        // Review-standards Sec5: emit metadata only, never prompt content.
        return $"SpendGuard denied LLM call (decision={d.Decision}, decision_id={d.DecisionId}, terminal={d.Terminal})";
    }
}

/// <summary>
/// Raised when the sidecar is unreachable and
/// <see cref="Options.OnSidecarUnavailable.Deny"/> is the configured behavior.
/// Cross-language parity per review-standards P3 with Python's
/// <c>SidecarUnavailable</c>.
/// </summary>
public sealed class SidecarUnavailableException : Exception
{
    /// <summary>Creates a new instance.</summary>
    public SidecarUnavailableException(string message, Exception? inner = null)
        : base(message, inner)
    {
    }
}

/// <summary>
/// Raised when REQUIRE_APPROVAL flows back through the middleware without a
/// connected approval pump. Caller is expected to catch and route to its own
/// approval UI. Parity with Python <c>PendingApprovalRequired</c>.
/// </summary>
public sealed class PendingApprovalRequiredException : Exception
{
    /// <summary>Approval request id from the sidecar.</summary>
    public string ApprovalRequestId { get; }

    /// <summary>Initialises with the sidecar's approval id.</summary>
    public PendingApprovalRequiredException(string approvalRequestId)
        : base($"SpendGuard returned REQUIRE_APPROVAL (approval_request_id={approvalRequestId})")
    {
        ApprovalRequestId = approvalRequestId ?? string.Empty;
    }
}
