// SPDX-License-Identifier: Apache-2.0
// Copyright (c) SpendGuard Authors.

using Spendguard.AgentFramework.Middleware;
using Spendguard.SidecarAdapter.V1;
using Xunit;

namespace Spendguard.AgentFramework.Tests.Unit;

public sealed class ExceptionTests
{
    [Fact]
    public void DecisionDenied_Carries_AllReasonFields()
    {
        var decision = new DecisionResponse
        {
            DecisionId = "decision-1",
            AuditDecisionEventId = "audit-1",
            Decision = DecisionResponse.Types.Decision.Stop,
            Terminal = true,
            RunCodeTriggered = "RUN_BUDGET_PROJECTION_EXCEEDED",
        };
        decision.ReasonCodes.Add("BUDGET_EXHAUSTED");
        decision.MatchedRuleIds.Add("rule-001");

        var ex = new SpendGuardDecisionDeniedException(decision);

        Assert.Equal("decision-1", ex.DecisionId);
        Assert.Equal("audit-1", ex.AuditDecisionEventId);
        Assert.True(ex.IsTerminal);
        Assert.Equal("RUN_BUDGET_PROJECTION_EXCEEDED", ex.RunCodeTriggered);
        Assert.Contains("BUDGET_EXHAUSTED", ex.ReasonCodes);
        Assert.Contains("rule-001", ex.MatchedRuleIds);
        Assert.Equal(DecisionResponse.Types.Decision.Stop, ex.DecisionEnum);
    }

    [Fact]
    public void DecisionDenied_Message_DoesNotLeakPromptContent()
    {
        var d = new DecisionResponse
        {
            DecisionId = "d-1",
            Decision = DecisionResponse.Types.Decision.Stop,
        };
        var ex = new SpendGuardDecisionDeniedException(d);

        // Reviewer Sec5: messages must contain only metadata, never inputs.
        Assert.DoesNotContain("prompt", ex.Message);
        Assert.Contains("d-1", ex.Message);
    }

    [Fact]
    public void SidecarUnavailable_PreservesInner()
    {
        var inner = new System.Net.Sockets.SocketException();
        var ex = new SidecarUnavailableException("down", inner);
        Assert.Same(inner, ex.InnerException);
    }

    [Fact]
    public void PendingApproval_RecordsApprovalId()
    {
        var ex = new PendingApprovalRequiredException("appr-1");
        Assert.Equal("appr-1", ex.ApprovalRequestId);
        Assert.Contains("appr-1", ex.Message);
    }
}
