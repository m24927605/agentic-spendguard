// SPDX-License-Identifier: Apache-2.0
// Copyright (c) SpendGuard Authors.

using System;
using System.Net.Sockets;
using System.Threading.Tasks;
using Microsoft.Extensions.AI;
using Microsoft.Extensions.Logging.Abstractions;
using Microsoft.Extensions.Options;
using Spendguard.AgentFramework.Middleware;
using Spendguard.AgentFramework.Options;
using Spendguard.AgentFramework.Tests.Helpers;
using Spendguard.SidecarAdapter.V1;
using Xunit;

namespace Spendguard.AgentFramework.Tests.Unit;

public sealed class MiddlewareTests
{
    private static SpendGuardOptions ValidOptions(OnSidecarUnavailable on = OnSidecarUnavailable.Deny)
    {
        return new SpendGuardOptions
        {
            TenantId = "tenant-1",
            BudgetId = "budget-1",
            SidecarSocketPath = "/tmp/spendguard.sock",
            OnSidecarUnavailable = on,
        };
    }

    private static SpendGuardChatMiddleware Build(
        FakeSidecarClient sidecar,
        IChatClient inner,
        SpendGuardOptions? opts = null)
    {
        return new SpendGuardChatMiddleware(
            inner,
            sidecar,
            Microsoft.Extensions.Options.Options.Create(opts ?? ValidOptions()),
            estimator: null,
            logger: NullLogger<SpendGuardChatMiddleware>.Instance);
    }

    [Fact]
    public async Task Allow_InvokesInner_AndStampsHandshake()
    {
        var sidecar = new FakeSidecarClient();
        sidecar.DecisionProducers.Add(_ => new DecisionResponse
        {
            DecisionId = "d-1",
            Decision = DecisionResponse.Types.Decision.Continue,
        });
        var inner = FakeChatClient.WithUsage(50, 25, "hello");
        var mw = Build(sidecar, inner);

        var resp = await mw.GetResponseAsync(new[] { new ChatMessage(ChatRole.User, "hi") });

        Assert.NotNull(resp);
        Assert.Equal(1, inner.InvocationCount);
        Assert.Equal(1, sidecar.HandshakeCallCount);
        Assert.Single(sidecar.DecisionCalls);
        Assert.Equal(DecisionRequest.Types.Trigger.LlmCallPre, sidecar.DecisionCalls[0].Trigger);
    }

    [Fact]
    public async Task Deny_ShortCircuits_InnerNeverCalled()
    {
        var sidecar = new FakeSidecarClient();
        sidecar.DecisionProducers.Add(_ => new DecisionResponse
        {
            DecisionId = "d-deny",
            Decision = DecisionResponse.Types.Decision.Stop,
            Terminal = true,
        });
        var inner = FakeChatClient.WithUsage(0, 0);
        var mw = Build(sidecar, inner);

        var ex = await Assert.ThrowsAsync<SpendGuardDecisionDeniedException>(() =>
            mw.GetResponseAsync(new[] { new ChatMessage(ChatRole.User, "hi") }));

        Assert.Equal("d-deny", ex.DecisionId);
        Assert.Equal(0, inner.InvocationCount);
    }

    [Fact]
    public async Task StopRunProjection_ShortCircuits_LikeStop()
    {
        var sidecar = new FakeSidecarClient();
        sidecar.DecisionProducers.Add(_ => new DecisionResponse
        {
            DecisionId = "d-runproj",
            Decision = DecisionResponse.Types.Decision.StopRunProjection,
            Terminal = true,
            RunCodeTriggered = "RUN_BUDGET_PROJECTION_EXCEEDED",
        });
        var inner = FakeChatClient.WithUsage(0, 0);
        var mw = Build(sidecar, inner);

        var ex = await Assert.ThrowsAsync<SpendGuardDecisionDeniedException>(() =>
            mw.GetResponseAsync(new[] { new ChatMessage(ChatRole.User, "hi") }));

        Assert.Equal("RUN_BUDGET_PROJECTION_EXCEEDED", ex.RunCodeTriggered);
        Assert.Equal(0, inner.InvocationCount);
    }

    [Fact]
    public async Task RequireApproval_RaisesPendingApprovalException()
    {
        var sidecar = new FakeSidecarClient();
        sidecar.DecisionProducers.Add(_ => new DecisionResponse
        {
            DecisionId = "d-appr",
            Decision = DecisionResponse.Types.Decision.RequireApproval,
            ApprovalRequestId = "appr-1",
        });
        var inner = FakeChatClient.WithUsage(0, 0);
        var mw = Build(sidecar, inner);

        var ex = await Assert.ThrowsAsync<PendingApprovalRequiredException>(() =>
            mw.GetResponseAsync(new[] { new ChatMessage(ChatRole.User, "hi") }));
        Assert.Equal("appr-1", ex.ApprovalRequestId);
        Assert.Equal(0, inner.InvocationCount);
    }

    [Fact]
    public async Task SidecarDown_FailClosed_ByDefault()
    {
        var sidecar = new FakeSidecarClient
        {
            DecisionThrow = new SocketException(),
        };
        // Need handshake to succeed so we get into RequestDecision.
        var inner = FakeChatClient.WithUsage(0, 0);
        var mw = Build(sidecar, inner);

        await Assert.ThrowsAsync<SidecarUnavailableException>(() =>
            mw.GetResponseAsync(new[] { new ChatMessage(ChatRole.User, "hi") }));
        Assert.Equal(0, inner.InvocationCount);
    }

    [Fact]
    public async Task SidecarDown_FailOpen_AllowsCall()
    {
        var sidecar = new FakeSidecarClient
        {
            DecisionThrow = new SocketException(),
        };
        var inner = FakeChatClient.WithUsage(10, 10);
        var mw = Build(sidecar, inner, ValidOptions(OnSidecarUnavailable.Allow));

        var resp = await mw.GetResponseAsync(new[] { new ChatMessage(ChatRole.User, "hi") });
        Assert.NotNull(resp);
        Assert.Equal(1, inner.InvocationCount);
    }

    [Fact]
    public async Task Handshake_RunsOnlyOnce_AcrossCalls()
    {
        var sidecar = new FakeSidecarClient();
        var inner = FakeChatClient.WithUsage(1, 1);
        var mw = Build(sidecar, inner);

        await mw.GetResponseAsync(new[] { new ChatMessage(ChatRole.User, "hi") });
        await mw.GetResponseAsync(new[] { new ChatMessage(ChatRole.User, "hi again") });

        Assert.Equal(1, sidecar.HandshakeCallCount);
        Assert.Equal(2, sidecar.DecisionCalls.Count);
        Assert.Equal(2, inner.InvocationCount);
    }

    [Fact]
    public async Task InnerThrows_TriggersReservationRelease()
    {
        var sidecar = new FakeSidecarClient();
        sidecar.DecisionProducers.Add(_ =>
        {
            var resp = new DecisionResponse
            {
                DecisionId = "d-r",
                Decision = DecisionResponse.Types.Decision.Continue,
            };
            resp.ReservationIds.Add("res-1");
            return resp;
        });
        var inner = FakeChatClient.WithUsage(0, 0);
        inner.ThrowOnNext = new InvalidOperationException("provider blew up");

        var mw = Build(sidecar, inner);

        await Assert.ThrowsAsync<InvalidOperationException>(() =>
            mw.GetResponseAsync(new[] { new ChatMessage(ChatRole.User, "hi") }));

        Assert.Single(sidecar.ReleaseCalls);
        Assert.Equal("res-1", sidecar.ReleaseCalls[0].ReservationId);
        Assert.Equal("tenant-1", sidecar.ReleaseCalls[0].TenantId);
        Assert.Contains("runtime_error", sidecar.ReleaseCalls[0].ReasonCodes);
    }

    [Fact]
    public async Task IdempotencyKey_Stable_AcrossRepeatedCalls()
    {
        var sidecar = new FakeSidecarClient();
        var inner = FakeChatClient.WithUsage(1, 1);
        var mw = Build(sidecar, inner);

        await mw.GetResponseAsync(new[] { new ChatMessage(ChatRole.User, "hi") });
        await mw.GetResponseAsync(new[] { new ChatMessage(ChatRole.User, "hi") });
        await mw.GetResponseAsync(new[] { new ChatMessage(ChatRole.User, "hi") });

        var keys = new[]
        {
            sidecar.DecisionCalls[0].Idempotency.Key,
            sidecar.DecisionCalls[1].Idempotency.Key,
            sidecar.DecisionCalls[2].Idempotency.Key,
        };
        Assert.NotEqual(keys[0], keys[1]);
        Assert.NotEqual(keys[1], keys[2]);
        Assert.All(keys, k => Assert.False(string.IsNullOrEmpty(k)));
    }

    [Fact]
    public async Task Trigger_AlwaysLlmCallPre_OnLlmRoute()
    {
        var sidecar = new FakeSidecarClient();
        var inner = FakeChatClient.WithUsage(1, 1);
        var mw = Build(sidecar, inner);

        await mw.GetResponseAsync(new[] { new ChatMessage(ChatRole.User, "hi") }, new ChatOptions { ModelId = "gpt-4o" });

        Assert.Equal(
            DecisionRequest.Types.Trigger.LlmCallPre,
            sidecar.DecisionCalls[0].Trigger);
        Assert.Equal("gpt-4o", sidecar.DecisionCalls[0].Route);
    }

    [Fact]
    public async Task UnknownDecisionEnum_FailsClosed()
    {
        var sidecar = new FakeSidecarClient();
        sidecar.DecisionProducers.Add(_ => new DecisionResponse
        {
            DecisionId = "d-unknown",
            Decision = DecisionResponse.Types.Decision.Unspecified,
        });
        var inner = FakeChatClient.WithUsage(0, 0);
        var mw = Build(sidecar, inner);

        await Assert.ThrowsAsync<SpendGuardDecisionDeniedException>(() =>
            mw.GetResponseAsync(new[] { new ChatMessage(ChatRole.User, "hi") }));
        Assert.Equal(0, inner.InvocationCount);
    }
}
