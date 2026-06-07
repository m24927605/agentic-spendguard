// SPDX-License-Identifier: Apache-2.0
// Copyright (c) SpendGuard Authors.

using System;
using System.Threading;
using System.Threading.Tasks;
using Spendguard.SidecarAdapter.V1;

namespace Spendguard.AgentFramework.Sidecar;

/// <summary>
/// Thin transport surface the middleware actually depends on. Defined as
/// an interface so the middleware can be unit-tested with an in-memory
/// fake (review-standards T1 / T2 / T3) without spinning up a real gRPC
/// server or UDS endpoint.
/// </summary>
public interface ISidecarClient : IDisposable
{
    /// <summary>
    /// Has <see cref="HandshakeAsync"/> been called successfully? Used by
    /// the middleware to gate <see cref="RequestDecisionAsync"/>
    /// (review-standards S3).
    /// </summary>
    bool IsHandshakeComplete { get; }

    /// <summary>Session id returned by the sidecar handshake. Empty until handshake completes.</summary>
    string SessionId { get; }

    /// <summary>
    /// Mandatory per adapter.proto §SidecarAdapter and review-standards S3.
    /// Negotiates SDK version, runtime kind, and capability mask. Must be
    /// called before any other RPC.
    /// </summary>
    Task<HandshakeResponse> HandshakeAsync(
        string tenantIdAssertion,
        string sdkVersion,
        string runtimeKind,
        CancellationToken ct = default);

    /// <summary>
    /// LLM-pre-call / tool-pre-call decision boundary. Reviewers (S1) verify
    /// the trigger enum and (S3) verify handshake has succeeded first.
    /// </summary>
    Task<DecisionResponse> RequestDecisionAsync(
        DecisionRequest request,
        CancellationToken ct = default);

    /// <summary>
    /// Explicit reservation release per ASP Draft-01 §4 (adapter.proto
    /// <c>ReleaseReservation</c> RPC).
    /// </summary>
    Task<ReleaseReservationResponse> ReleaseReservationAsync(
        ReleaseReservationRequest request,
        CancellationToken ct = default);
}
