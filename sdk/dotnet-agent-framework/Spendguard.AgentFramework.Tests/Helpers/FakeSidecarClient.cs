// SPDX-License-Identifier: Apache-2.0
// Copyright (c) SpendGuard Authors.

using System;
using System.Collections.Generic;
using System.Threading;
using System.Threading.Tasks;
using Spendguard.AgentFramework.Sidecar;
using Spendguard.SidecarAdapter.V1;

namespace Spendguard.AgentFramework.Tests.Helpers;

/// <summary>
/// In-memory <see cref="ISidecarClient"/> for unit-testing the middleware
/// without spinning up a real gRPC server. Mirrors the Python fixture shape.
/// </summary>
public sealed class FakeSidecarClient : ISidecarClient
{
    /// <summary>Configurable handshake response.</summary>
    public HandshakeResponse HandshakeResult { get; set; } = new()
    {
        SidecarVersion = "fake-0.0.0",
        ProtocolVersion = 1u,
        SessionId = "session-fake-0001",
    };

    /// <summary>Sequence of decision producers; index advances each call.</summary>
    public List<Func<DecisionRequest, DecisionResponse>> DecisionProducers { get; } = new();

    /// <summary>Sequence of release producers; index advances each call.</summary>
    public List<Func<ReleaseReservationRequest, ReleaseReservationResponse>> ReleaseProducers { get; } = new();

    /// <summary>Number of times handshake was invoked.</summary>
    public int HandshakeCallCount;

    /// <summary>Captured decision requests in arrival order.</summary>
    public List<DecisionRequest> DecisionCalls { get; } = new();

    /// <summary>Captured release requests in arrival order.</summary>
    public List<ReleaseReservationRequest> ReleaseCalls { get; } = new();

    /// <summary>Captured handshake requests in arrival order.</summary>
    public List<(string Tenant, string Sdk, string Runtime)> HandshakeCalls { get; } = new();

    /// <summary>If non-null, handshake throws this exception.</summary>
    public Exception? HandshakeThrow;

    /// <summary>If non-null, request-decision throws this exception.</summary>
    public Exception? DecisionThrow;

    private int _producerIdx;
    private int _releaseIdx;

    /// <inheritdoc/>
    public bool IsHandshakeComplete { get; private set; }

    /// <inheritdoc/>
    public string SessionId { get; private set; } = string.Empty;

    /// <inheritdoc/>
    public Task<HandshakeResponse> HandshakeAsync(
        string tenantIdAssertion,
        string sdkVersion,
        string runtimeKind,
        CancellationToken ct = default)
    {
        HandshakeCallCount++;
        HandshakeCalls.Add((tenantIdAssertion, sdkVersion, runtimeKind));
        if (HandshakeThrow is not null)
        {
            throw HandshakeThrow;
        }
        IsHandshakeComplete = true;
        SessionId = HandshakeResult.SessionId ?? string.Empty;
        return Task.FromResult(HandshakeResult);
    }

    /// <inheritdoc/>
    public Task<DecisionResponse> RequestDecisionAsync(
        DecisionRequest request,
        CancellationToken ct = default)
    {
        DecisionCalls.Add(request);
        if (DecisionThrow is not null)
        {
            throw DecisionThrow;
        }

        if (DecisionProducers.Count == 0)
        {
            return Task.FromResult(new DecisionResponse
            {
                DecisionId = "decision-default",
                Decision = DecisionResponse.Types.Decision.Continue,
            });
        }
        int idx = Interlocked.Increment(ref _producerIdx) - 1;
        var fn = DecisionProducers[idx % DecisionProducers.Count];
        return Task.FromResult(fn(request));
    }

    /// <inheritdoc/>
    public Task<ReleaseReservationResponse> ReleaseReservationAsync(
        ReleaseReservationRequest request,
        CancellationToken ct = default)
    {
        ReleaseCalls.Add(request);
        if (ReleaseProducers.Count == 0)
        {
            return Task.FromResult(new ReleaseReservationResponse
            {
                LedgerTransactionId = $"release-{request.ReservationId}",
            });
        }
        int idx = Interlocked.Increment(ref _releaseIdx) - 1;
        var fn = ReleaseProducers[idx % ReleaseProducers.Count];
        return Task.FromResult(fn(request));
    }

    /// <inheritdoc/>
    public void Dispose() { }
}
