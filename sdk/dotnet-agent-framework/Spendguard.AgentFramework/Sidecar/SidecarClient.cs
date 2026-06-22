// SPDX-License-Identifier: Apache-2.0
// Copyright (c) SpendGuard Authors.

using System;
using System.Threading;
using System.Threading.Tasks;
using Grpc.Net.Client;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Logging.Abstractions;
using Spendguard.SidecarAdapter.V1;
using static Spendguard.SidecarAdapter.V1.SidecarAdapter;

namespace Spendguard.AgentFramework.Sidecar;

/// <summary>
/// gRPC-over-UDS sidecar client. Owns a single <see cref="GrpcChannel"/>
/// and the generated stub. Thread-safe: a single instance is intended to
/// serve all concurrent middleware calls in a process.
/// </summary>
public sealed class SidecarClient : ISidecarClient
{
    private readonly GrpcChannel _channel;
    private readonly SidecarAdapterClient _stub;
    private readonly ILogger<SidecarClient> _logger;
    private readonly SemaphoreSlim _handshakeGate = new(1, 1);

    private volatile bool _handshakeComplete;
    private string _sessionId = string.Empty;

    /// <summary>
    /// Build a client around a pre-constructed channel. Caller owns the
    /// channel only if it passes <paramref name="ownsChannel"/>=false.
    /// </summary>
    public SidecarClient(GrpcChannel channel, bool ownsChannel = true, ILogger<SidecarClient>? logger = null)
    {
        _channel = channel ?? throw new ArgumentNullException(nameof(channel));
        _stub = new SidecarAdapterClient(_channel);
        _logger = logger ?? NullLogger<SidecarClient>.Instance;
        OwnsChannel = ownsChannel;
    }

    /// <summary>
    /// Convenience factory: open a UDS channel at <paramref name="socketPath"/>
    /// and wrap it in a SidecarClient.
    /// </summary>
    public static SidecarClient ForSocketPath(string socketPath, ILoggerFactory? loggerFactory = null)
    {
        var channel = SidecarChannelFactory.Create(socketPath, loggerFactory: loggerFactory);
        var logger = loggerFactory?.CreateLogger<SidecarClient>();
        return new SidecarClient(channel, ownsChannel: true, logger: logger);
    }

    /// <summary>Set when this client owns the underlying gRPC channel.</summary>
    public bool OwnsChannel { get; }

    /// <inheritdoc/>
    public bool IsHandshakeComplete => _handshakeComplete;

    /// <inheritdoc/>
    public string SessionId => _sessionId;

    /// <inheritdoc/>
    public async Task<HandshakeResponse> HandshakeAsync(
        string tenantIdAssertion,
        string sdkVersion,
        string runtimeKind,
        CancellationToken ct = default)
    {
        if (string.IsNullOrWhiteSpace(tenantIdAssertion))
        {
            throw new ArgumentException("tenant_id_assertion must be non-empty.", nameof(tenantIdAssertion));
        }

        await _handshakeGate.WaitAsync(ct).ConfigureAwait(false);
        try
        {
            if (_handshakeComplete)
            {
                _logger.LogDebug("Handshake already complete; reusing session id {SessionId}.", _sessionId);
                return new HandshakeResponse { SessionId = _sessionId };
            }

            var req = new HandshakeRequest
            {
                SdkVersion = sdkVersion ?? string.Empty,
                RuntimeKind = runtimeKind ?? string.Empty,
                RuntimeVersion = Environment.Version.ToString(),
                CapabilityLevel = HandshakeRequest.Types.CapabilityLevel.L1LlmCall,
                TenantIdAssertion = tenantIdAssertion,
                WorkloadInstanceId = string.Empty,
                ProtocolVersion = 1u,
            };

            // Review-standards Sec5: log only metadata, not assertion content.
            _logger.LogInformation(
                "spendguard handshake: sdk={SdkVersion} runtime={RuntimeKind}",
                sdkVersion, runtimeKind);

            HandshakeResponse resp = await _stub.HandshakeAsync(req, cancellationToken: ct);
            _sessionId = resp.SessionId ?? string.Empty;
            _handshakeComplete = true;
            return resp;
        }
        finally
        {
            _handshakeGate.Release();
        }
    }

    /// <inheritdoc/>
    public async Task<DecisionResponse> RequestDecisionAsync(
        DecisionRequest request,
        CancellationToken ct = default)
    {
        if (request is null) throw new ArgumentNullException(nameof(request));
        if (!_handshakeComplete)
        {
            // Review-standards S3: typed exception, not a silent no-op.
            throw new HandshakeRequiredException(
                "RequestDecision called before Handshake completed.");
        }

        // If the caller forgot the session id, fill it in from the handshake
        // for them — protects against subtle drift across copy-paste callers.
        if (string.IsNullOrEmpty(request.SessionId))
        {
            request.SessionId = _sessionId;
        }

        return await _stub.RequestDecisionAsync(request, cancellationToken: ct);
    }

    /// <inheritdoc/>
    public async Task<ReleaseReservationResponse> ReleaseReservationAsync(
        ReleaseReservationRequest request,
        CancellationToken ct = default)
    {
        if (request is null) throw new ArgumentNullException(nameof(request));
        if (!_handshakeComplete)
        {
            throw new HandshakeRequiredException(
                "ReleaseReservation called before Handshake completed.");
        }

        if (string.IsNullOrEmpty(request.SessionId))
        {
            request.SessionId = _sessionId;
        }

        return await _stub.ReleaseReservationAsync(request, cancellationToken: ct);
    }

    /// <inheritdoc/>
    public async Task EmitTraceEventAsync(
        TraceEvent traceEvent,
        CancellationToken ct = default)
    {
        if (traceEvent is null) throw new ArgumentNullException(nameof(traceEvent));
        if (!_handshakeComplete)
        {
            throw new HandshakeRequiredException(
                "EmitTraceEvent called before Handshake completed.");
        }

        if (string.IsNullOrEmpty(traceEvent.SessionId))
        {
            traceEvent.SessionId = _sessionId;
        }

        // One-shot bidi stream: write the single event, half-close, then drain
        // the ack stream. The sidecar emits exactly one ack per inbound event;
        // any status other than ACCEPTED means the commit lifecycle failed and
        // the caller MUST see it (mirrors the Python SDK's emit_llm_call_post).
        using var call = _stub.EmitTraceEvents(cancellationToken: ct);
        await call.RequestStream.WriteAsync(traceEvent).ConfigureAwait(false);
        await call.RequestStream.CompleteAsync().ConfigureAwait(false);
        while (await call.ResponseStream.MoveNext(ct).ConfigureAwait(false))
        {
            TraceEventAck ack = call.ResponseStream.Current;
            if (ack.Status != TraceEventAck.Types.Status.Accepted)
            {
                throw new SpendGuardCommitException(
                    $"EmitTraceEvents rejected: status={ack.Status} " +
                    $"code={(ack.Error is null ? 0 : ack.Error.Code)} " +
                    $"message={(ack.Error is null ? string.Empty : ack.Error.Message)}");
            }
        }
    }

    /// <inheritdoc/>
    public void Dispose()
    {
        if (OwnsChannel)
        {
            _channel.Dispose();
        }

        _handshakeGate.Dispose();
    }
}

/// <summary>Raised when an RPC is called before <see cref="ISidecarClient.HandshakeAsync"/>.</summary>
public sealed class HandshakeRequiredException : InvalidOperationException
{
    /// <summary>Creates a new <see cref="HandshakeRequiredException"/> with the supplied message.</summary>
    public HandshakeRequiredException(string message) : base(message) { }
}

/// <summary>
/// Raised when the sidecar does not ACCEPT an emitted trace event (the commit
/// lifecycle failed). Parity with the Python SDK surfacing
/// <c>EmitTraceEvents rejected</c>.
/// </summary>
public sealed class SpendGuardCommitException : Exception
{
    /// <summary>Creates a new instance with the supplied message.</summary>
    public SpendGuardCommitException(string message) : base(message) { }
}
