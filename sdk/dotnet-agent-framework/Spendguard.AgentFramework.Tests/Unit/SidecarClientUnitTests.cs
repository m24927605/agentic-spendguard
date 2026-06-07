// SPDX-License-Identifier: Apache-2.0
// Copyright (c) SpendGuard Authors.

using System;
using System.Threading.Tasks;
using Spendguard.AgentFramework.Sidecar;
using Spendguard.SidecarAdapter.V1;
using Xunit;

namespace Spendguard.AgentFramework.Tests.Unit;

public sealed class SidecarClientUnitTests
{
    [Fact]
    public void ChannelFactory_Rejects_EmptySocketPath()
    {
        Assert.Throws<ArgumentException>(() => SidecarChannelFactory.Create(""));
        Assert.Throws<ArgumentException>(() => SidecarChannelFactory.Create("   "));
    }

    [Fact]
    public async Task RequestDecision_BeforeHandshake_Throws()
    {
        // Build the channel but never call Handshake.
        var channel = SidecarChannelFactory.Create("/tmp/spendguard-not-real.sock");
        using var client = new SidecarClient(channel);
        await Assert.ThrowsAsync<HandshakeRequiredException>(() =>
            client.RequestDecisionAsync(new DecisionRequest()));
    }

    [Fact]
    public async Task ReleaseReservation_BeforeHandshake_Throws()
    {
        var channel = SidecarChannelFactory.Create("/tmp/spendguard-not-real.sock");
        using var client = new SidecarClient(channel);
        await Assert.ThrowsAsync<HandshakeRequiredException>(() =>
            client.ReleaseReservationAsync(new ReleaseReservationRequest()));
    }

    [Fact]
    public async Task HandshakeAsync_RejectsEmptyTenantAssertion()
    {
        var channel = SidecarChannelFactory.Create("/tmp/spendguard-not-real.sock");
        using var client = new SidecarClient(channel);
        await Assert.ThrowsAsync<ArgumentException>(() =>
            client.HandshakeAsync(string.Empty, "0.1.0", "test"));
    }
}
