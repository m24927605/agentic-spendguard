// SPDX-License-Identifier: Apache-2.0
// Copyright (c) SpendGuard Authors.

using System.Threading.Tasks;
using Microsoft.Extensions.AI;
using Microsoft.Extensions.Logging.Abstractions;
using Microsoft.Extensions.Options;
using Spendguard.AgentFramework.Middleware;
using Spendguard.AgentFramework.Options;
using Spendguard.AgentFramework.Sidecar;
using Spendguard.AgentFramework.Tests.Helpers;
using Spendguard.SidecarAdapter.V1;
using Xunit;

namespace Spendguard.AgentFramework.Tests.Integration;

/// <summary>
/// Integration tests gated behind <c>#[Trait("Category","Integration")]</c>
/// — they spin up the in-process gRPC server, exercise the generated wire
/// shape, and validate Handshake + RequestDecision behavior end-to-end.
/// </summary>
public sealed class InProcessSidecarIntegrationTests
{
    [Fact]
    [Trait("Category", "Integration")]
    public async Task RealWire_AllowFlow_RoundTrips()
    {
        await using var server = await InProcessSidecarServer.StartAsync(s =>
        {
            s.DecisionPlanner.Add(_ => new DecisionResponse
            {
                DecisionId = "wire-1",
                Decision = DecisionResponse.Types.Decision.Continue,
            });
        });

        using var sidecar = new SidecarClient(server.Channel, ownsChannel: false);
        var inner = FakeChatClient.WithUsage(7, 13, "wire-ok");
        var opts = new SpendGuardOptions
        {
            TenantId = "tenant-wire",
            BudgetId = "budget-wire",
            SidecarSocketPath = "/dev/null",
        };

        var mw = new SpendGuardChatMiddleware(
            inner,
            sidecar,
            Microsoft.Extensions.Options.Options.Create(opts),
            estimator: null,
            logger: NullLogger<SpendGuardChatMiddleware>.Instance);

        var resp = await mw.GetResponseAsync(new[] { new ChatMessage(ChatRole.User, "ping") });

        Assert.NotNull(resp);
        Assert.Single(server.CapturedHandshakes);
        Assert.Single(server.CapturedDecisionRequests);
        Assert.Equal(
            DecisionRequest.Types.Trigger.LlmCallPre,
            server.CapturedDecisionRequests[0].Trigger);
        Assert.Equal(
            "tenant-wire",
            server.CapturedHandshakes[0].TenantIdAssertion);
    }

    [Fact]
    [Trait("Category", "Integration")]
    public async Task RealWire_DenyFlow_Throws()
    {
        await using var server = await InProcessSidecarServer.StartAsync(s =>
        {
            s.DecisionPlanner.Add(_ => new DecisionResponse
            {
                DecisionId = "wire-deny",
                Decision = DecisionResponse.Types.Decision.Stop,
                Terminal = true,
            });
        });

        using var sidecar = new SidecarClient(server.Channel, ownsChannel: false);
        var inner = FakeChatClient.WithUsage(0, 0);
        var opts = new SpendGuardOptions
        {
            TenantId = "tenant-wire",
            BudgetId = "budget-wire",
            SidecarSocketPath = "/dev/null",
        };
        var mw = new SpendGuardChatMiddleware(
            inner,
            sidecar,
            Microsoft.Extensions.Options.Options.Create(opts),
            estimator: null,
            logger: NullLogger<SpendGuardChatMiddleware>.Instance);

        await Assert.ThrowsAsync<SpendGuardDecisionDeniedException>(() =>
            mw.GetResponseAsync(new[] { new ChatMessage(ChatRole.User, "ping") }));

        // Provider was NOT called.
        Assert.Equal(0, inner.InvocationCount);
    }

    [Fact]
    [Trait("Category", "Integration")]
    public async Task RealWire_HandshakeNegotiatesCapability()
    {
        await using var server = await InProcessSidecarServer.StartAsync();
        using var sidecar = new SidecarClient(server.Channel, ownsChannel: false);

        var resp = await sidecar.HandshakeAsync(
            tenantIdAssertion: "tenant-wire",
            sdkVersion: "0.1.0-pre",
            runtimeKind: "microsoft-agent-framework-dotnet");

        Assert.Equal("session-stub-0001", resp.SessionId);
        Assert.True(sidecar.IsHandshakeComplete);
        Assert.Single(server.CapturedHandshakes);
        Assert.Equal(
            HandshakeRequest.Types.CapabilityLevel.L1LlmCall,
            server.CapturedHandshakes[0].CapabilityLevel);
    }
}
