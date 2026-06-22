// SPDX-License-Identifier: Apache-2.0
// Copyright (c) SpendGuard Authors.

using System;
using System.Collections.Generic;
using System.Globalization;
using System.Net.Sockets;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using Google.Protobuf;
using Google.Protobuf.WellKnownTypes;
using Microsoft.Extensions.AI;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Logging.Abstractions;
using Microsoft.Extensions.Options;
using Spendguard.AgentFramework.Estimators;
using Spendguard.AgentFramework.Ids;
using Spendguard.AgentFramework.Options;
using Spendguard.AgentFramework.Sidecar;
using Spendguard.Common.V1;
using Spendguard.SidecarAdapter.V1;

namespace Spendguard.AgentFramework.Middleware;

/// <summary>
/// Microsoft.Extensions.AI <see cref="DelegatingChatClient"/> implementation
/// that gates every <see cref="IChatClient.GetResponseAsync(IEnumerable{ChatMessage},ChatOptions,CancellationToken)"/>
/// call through the SpendGuard sidecar.
///
/// Lifecycle per design.md §3.1:
///   1. BEFORE next.GetResponseAsync — call <c>RequestDecision</c> with
///      <c>LLM_CALL_PRE</c>.
///   2. On <c>STOP</c> / <c>STOP_RUN_PROJECTION</c> — throw
///      <see cref="SpendGuardDecisionDeniedException"/>.
///   3. On <c>REQUIRE_APPROVAL</c> — throw
///      <see cref="PendingApprovalRequiredException"/>.
///   4. On <c>CONTINUE</c> / <c>DEGRADE</c> — call <c>next.GetResponseAsync</c>.
///   5. If the inner call throws — release the reservation.
///   6. AFTER next.GetResponseAsync — (Stage 2; stubbed in SLICE_04) emit
///      <c>LLM_CALL_POST</c> with real usage.
/// </summary>
public sealed class SpendGuardChatMiddleware : DelegatingChatClient
{
    private readonly ISidecarClient _sidecar;
    private readonly SpendGuardOptions _options;
    private readonly ITokenEstimator _estimator;
    private readonly ILogger<SpendGuardChatMiddleware> _logger;
    private long _llmCallCounter;

    /// <summary>
    /// DI-friendly constructor. <see cref="DelegatingChatClient"/> wraps the
    /// inner <paramref name="innerClient"/>; the middleware delegates to it
    /// only after a CONTINUE / DEGRADE decision.
    /// </summary>
    public SpendGuardChatMiddleware(
        IChatClient innerClient,
        ISidecarClient sidecar,
        IOptions<SpendGuardOptions> options,
        ITokenEstimator? estimator = null,
        ILogger<SpendGuardChatMiddleware>? logger = null)
        : base(innerClient)
    {
        if (options is null) throw new ArgumentNullException(nameof(options));
        _sidecar = sidecar ?? throw new ArgumentNullException(nameof(sidecar));
        _options = options.Value ?? throw new ArgumentException("Options.Value is null", nameof(options));
        _options.Validate();
        _estimator = estimator ?? new SimpleTokenEstimator();
        _logger = logger ?? NullLogger<SpendGuardChatMiddleware>.Instance;
    }

    /// <inheritdoc/>
    public override async Task<ChatResponse> GetResponseAsync(
        IEnumerable<ChatMessage> messages,
        ChatOptions? options = null,
        CancellationToken cancellationToken = default)
    {
        // 1) Make sure handshake has happened (review-standards S3).
        await EnsureHandshakeAsync(cancellationToken).ConfigureAwait(false);

        // 2) Stamp this call with stable identifiers.
        long callOrdinal = Interlocked.Increment(ref _llmCallCounter);
        string runId = _sidecar.SessionId; // until run scoping lands; SLICE_05+ wires real run ctx
        string stepId = $"step-{callOrdinal:D8}";
        string llmCallId = $"llm-{callOrdinal:D8}";

        // 3) Build the DecisionRequest.
        DecisionRequest req = BuildDecisionRequest(
            messages,
            options,
            runId,
            stepId,
            llmCallId);

        // 4) Call RequestDecision with fail-closed semantics.
        DecisionResponse decision;
        try
        {
            decision = await _sidecar
                .RequestDecisionAsync(req, cancellationToken)
                .ConfigureAwait(false);
        }
        catch (SocketException sx)
        {
            return await HandleSidecarUnavailableAsync(messages, options, cancellationToken, sx).ConfigureAwait(false);
        }
        catch (Grpc.Core.RpcException rx) when (
            rx.StatusCode == Grpc.Core.StatusCode.Unavailable ||
            rx.StatusCode == Grpc.Core.StatusCode.DeadlineExceeded)
        {
            return await HandleSidecarUnavailableAsync(messages, options, cancellationToken, rx).ConfigureAwait(false);
        }

        // 5) Branch on the decision.
        switch (decision.Decision)
        {
            case DecisionResponse.Types.Decision.Stop:
            case DecisionResponse.Types.Decision.StopRunProjection:
                _logger.LogInformation(
                    "spendguard deny decision={Decision} decision_id={DecisionId} run_code={RunCode}",
                    decision.Decision, decision.DecisionId, decision.RunCodeTriggered);
                throw new SpendGuardDecisionDeniedException(decision);

            case DecisionResponse.Types.Decision.RequireApproval:
                throw new PendingApprovalRequiredException(decision.ApprovalRequestId);

            case DecisionResponse.Types.Decision.Continue:
            case DecisionResponse.Types.Decision.Degrade:
            case DecisionResponse.Types.Decision.Skip:
                // Continue past — DEGRADE / SKIP currently treated like CONTINUE
                // until DEGRADE wiring lands in SLICE_05.
                break;

            default:
                // Fail-closed on unknown decision codes (review-standards D5).
                _logger.LogWarning(
                    "spendguard unknown decision={Decision}; failing closed.",
                    decision.Decision);
                throw new SpendGuardDecisionDeniedException(decision);
        }

        // 6) Hand off to the inner chat client. Release on inner exception.
        ChatResponse response;
        try
        {
            response = await base
                .GetResponseAsync(messages, options, cancellationToken)
                .ConfigureAwait(false);
        }
        catch (Exception inner)
        {
            await TryReleaseAsync(decision, cancellationToken, inner).ConfigureAwait(false);
            throw;
        }

        // 7) Emit LLM_CALL_POST — commit the reservation with the real usage
        //    over the EmitTraceEvents bidi wire.
        await EmitLlmCallPostAsync(
            decision, response, runId, stepId, llmCallId, cancellationToken)
            .ConfigureAwait(false);

        return response;
    }

    private DecisionRequest BuildDecisionRequest(
        IEnumerable<ChatMessage> messages,
        ChatOptions? options,
        string runId,
        string stepId,
        string llmCallId)
    {
        int inputTokens = EstimateInputTokens(messages);
        string idempotencyKey = IdempotencyKeyDerivation.DeriveHex(
            _options.TenantId,
            _sidecar.SessionId,
            runId,
            stepId,
            llmCallId,
            "LLM_CALL_PRE");

        var req = new DecisionRequest
        {
            SessionId = _sidecar.SessionId,
            Trigger = DecisionRequest.Types.Trigger.LlmCallPre,
            Trace = new TraceContext(),
            Ids = new SpendGuardIds
            {
                RunId = runId,
                StepId = stepId,
                LlmCallId = llmCallId,
            },
            Route = options?.ModelId ?? string.Empty,
            Inputs = BuildDecisionInputs(inputTokens, options),
            Idempotency = new Idempotency
            {
                Key = idempotencyKey,
            },
        };

        return req;
    }

    /// <summary>
    /// Build the <see cref="DecisionRequest.Types.Inputs"/> payload. When
    /// <see cref="SpendGuardOptions.UnitId"/> is set (HARDEN_D05_UR), the
    /// canonical-truth ledger UUID is threaded onto <c>ProjectedUnit.UnitId</c>
    /// so the sidecar ledger can resolve the budget claim. Omitting leaves
    /// the wire field empty — same as the pre-HARDEN_D05_UR behaviour.
    /// </summary>
    private DecisionRequest.Types.Inputs BuildDecisionInputs(
        int inputTokens,
        ChatOptions? options)
    {
        var inputs = new DecisionRequest.Types.Inputs
        {
            ClaimEstimate = new ClaimEstimate
            {
                TokenizerTier = "T3",
                InputTokens = inputTokens,
                PredictedATokens = Math.Max(inputTokens / 2, 1),
                ReservedStrategy = "A",
                PredictionStrategyUsed = "A",
                PredictionPolicyUsed = "STRICT_CEILING",
                Model = options?.ModelId ?? string.Empty,
            },
        };

        if (_options.UnitId is Guid unitId)
        {
            UnitRef unitRef = BuildUnitRef(unitId);
            inputs.ProjectedUnit = unitRef;

            // Populate the repeated projected_claims the ledger requires for a
            // reserve (the sidecar rejects an empty list). The amount is the
            // operator-supplied per-call override (the demo flips it tiny for
            // ALLOW / 2B for DENY) or the token estimate.
            long amount = _options.ProjectedClaimAmountAtomic ?? Math.Max(inputTokens, 1);
            inputs.ProjectedClaims.Add(new BudgetClaim
            {
                BudgetId = _options.BudgetId,
                Unit = unitRef.Clone(),
                AmountAtomic = amount.ToString(CultureInfo.InvariantCulture),
                Direction = BudgetClaim.Types.Direction.Debit,
                WindowInstanceId = _options.WindowInstanceId,
            });
        }

        return inputs;
    }

    private int EstimateInputTokens(IEnumerable<ChatMessage> messages)
    {
        if (messages is null) return 0;

        // Hot-path bounded loop (review-standards Sec4).
        var sb = new StringBuilder();
        foreach (ChatMessage m in messages)
        {
            if (m?.Text is { Length: > 0 } text)
            {
                sb.Append(text).Append('\n');
            }
        }

        return _estimator.EstimateInputTokensForText(sb.ToString());
    }

    private async Task EnsureHandshakeAsync(CancellationToken ct)
    {
        if (_sidecar.IsHandshakeComplete)
        {
            return;
        }

        try
        {
            await _sidecar.HandshakeAsync(
                _options.TenantId,
                _options.SdkVersion,
                _options.RuntimeKind,
                ct).ConfigureAwait(false);
        }
        catch (SocketException sx)
        {
            // Same fail-closed path as decision-time unavailability.
            throw new SidecarUnavailableException(
                "Sidecar UDS not reachable during handshake.", sx);
        }
    }

    private async Task<ChatResponse> HandleSidecarUnavailableAsync(
        IEnumerable<ChatMessage> messages,
        ChatOptions? options,
        CancellationToken ct,
        Exception cause)
    {
        if (_options.OnSidecarUnavailable == Options.OnSidecarUnavailable.Allow)
        {
            // Reviewer Sec3: explicit opt-in path must log a warning.
            _logger.LogWarning(cause,
                "spendguard sidecar unavailable; OnSidecarUnavailable=Allow. " +
                "Proceeding without audit row.");
            return await base.GetResponseAsync(messages, options, ct).ConfigureAwait(false);
        }

        throw new SidecarUnavailableException(
            "SpendGuard sidecar unreachable and OnSidecarUnavailable=Deny.",
            cause);
    }

    private async Task TryReleaseAsync(
        DecisionResponse decision,
        CancellationToken ct,
        Exception inner)
    {
        if (decision.ReservationIds.Count == 0)
        {
            return;
        }

        foreach (string reservationId in decision.ReservationIds)
        {
            try
            {
                var req = new ReleaseReservationRequest
                {
                    ReservationId = reservationId,
                    IdempotencyKey = $"release-{decision.DecisionId}-{reservationId}",
                    TenantId = _options.TenantId,
                    SessionId = _sidecar.SessionId,
                };
                req.ReasonCodes.Add("runtime_error");

                await _sidecar
                    .ReleaseReservationAsync(req, ct)
                    .ConfigureAwait(false);
            }
            catch (Exception releaseEx)
            {
                _logger.LogError(releaseEx,
                    "spendguard release failed for reservation {ReservationId} (inner: {InnerMessage})",
                    reservationId, inner.Message);
                // Swallow — surface the original exception. Reservation will
                // TTL-release server-side.
            }
        }
    }

    /// <summary>
    /// Commit the reservation by emitting an LLM_CALL_POST trace event with the
    /// real usage. estimated_amount_atomic + outcome=SUCCESS drives the
    /// sidecar's CommitEstimated path (reservation 'reserved' -> 'committed').
    /// </summary>
    private async Task EmitLlmCallPostAsync(
        DecisionResponse decision,
        ChatResponse response,
        string runId,
        string stepId,
        string llmCallId,
        CancellationToken ct)
    {
        // Only commit when a reservation was actually issued AND a unit is
        // wired (recipe-style integrations with no ledger reserve skip POST).
        if (decision.ReservationIds.Count == 0 || _options.UnitId is not Guid unitId)
        {
            _logger.LogDebug(
                "spendguard LLM_CALL_POST skipped (reservations={Count} unit_set={UnitSet})",
                decision.ReservationIds.Count,
                _options.UnitId.HasValue);
            return;
        }

        long totalTokens = response.Usage?.TotalTokenCount ?? 0L;
        if (totalTokens <= 0)
        {
            // CommitEstimated requires a positive estimate; floor to 1 when the
            // provider did not return usage.
            totalTokens = 1;
        }

        var payload = new LlmCallPostPayload
        {
            ReservationId = decision.ReservationIds[0],
            ProviderReportedAmountAtomic = string.Empty,
            EstimatedAmountAtomic = totalTokens.ToString(CultureInfo.InvariantCulture),
            Unit = BuildUnitRef(unitId),
            Pricing = BuildPricingFreeze(),
            ProviderEventId = response.ResponseId ?? string.Empty,
            Outcome = LlmCallPostPayload.Types.Outcome.Success,
        };

        var ev = new TraceEvent
        {
            SessionId = _sidecar.SessionId,
            Ids = new SpendGuardIds
            {
                RunId = runId,
                StepId = stepId,
                LlmCallId = llmCallId,
                DecisionId = decision.DecisionId,
            },
            Kind = TraceEvent.Types.EventKind.LlmCallPost,
            EventTime = Timestamp.FromDateTime(DateTime.UtcNow),
            LlmCallPost = payload,
        };

        await _sidecar.EmitTraceEventAsync(ev, ct).ConfigureAwait(false);
    }

    private UnitRef BuildUnitRef(Guid unitId) => new UnitRef
    {
        UnitId = unitId.ToString(),
        TokenKind = _options.UnitTokenKind ?? string.Empty,
        ModelFamily = _options.UnitModelFamily ?? string.Empty,
    };

    private PricingFreeze BuildPricingFreeze()
    {
        var pricing = new PricingFreeze
        {
            PricingVersion = _options.PricingVersion ?? string.Empty,
            FxRateVersion = _options.FxRateVersion ?? string.Empty,
            UnitConversionVersion = _options.UnitConversionVersion ?? string.Empty,
        };
        if (!string.IsNullOrEmpty(_options.PriceSnapshotHashHex))
        {
            pricing.PriceSnapshotHash = ByteString.CopyFrom(
                Convert.FromHexString(_options.PriceSnapshotHashHex));
        }
        return pricing;
    }
}
