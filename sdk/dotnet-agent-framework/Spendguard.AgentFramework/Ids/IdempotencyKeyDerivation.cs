// SPDX-License-Identifier: Apache-2.0
// Copyright (c) SpendGuard Authors.

using System;
using System.Globalization;
using System.IO.Hashing;
using System.Text;

namespace Spendguard.AgentFramework.Ids;

/// <summary>
/// Idempotency-key derivation for <c>LLM_CALL_PRE</c> /
/// <c>TOOL_CALL_PRE</c> decision boundaries, byte-compatible with the
/// Python SDK's <c>derive_idempotency_key</c> helper (review-standards P1).
/// </summary>
/// <remarks>
/// <para>
/// Per design.md ADR-007: idempotency_key = blake2b(canonical_concat) over
/// <c>(tenant_id, session_id, run_id, step_id, llm_call_id, trigger)</c>.
/// </para>
/// <para>
/// We use XxHash3 (built-in to <c>System.IO.Hashing</c>) as the runtime
/// hash here purely for deterministic, dependency-free derivation in
/// pre-release; the byte layout (canonical UTF-8 newline-delimited tuple
/// then hashed) matches the Python helper. The hash function itself is
/// scheduled to switch to BLAKE2b in a follow-up slice when the SDK
/// dependency lands (parity test stays valid because the canonical
/// pre-image is identical; only the digest function differs).
/// </para>
/// </remarks>
public static class IdempotencyKeyDerivation
{
    private const char Separator = '\n';

    /// <summary>
    /// Build the canonical pre-image string that gets hashed into the
    /// idempotency key. Exposed for the cross-language parity test
    /// (review-standards P1).
    /// </summary>
    public static string CanonicalPreImage(
        string tenantId,
        string sessionId,
        string runId,
        string stepId,
        string llmCallId,
        string trigger)
    {
        if (tenantId is null) throw new ArgumentNullException(nameof(tenantId));
        if (sessionId is null) throw new ArgumentNullException(nameof(sessionId));
        if (runId is null) throw new ArgumentNullException(nameof(runId));
        if (stepId is null) throw new ArgumentNullException(nameof(stepId));
        if (llmCallId is null) throw new ArgumentNullException(nameof(llmCallId));
        if (trigger is null) throw new ArgumentNullException(nameof(trigger));

        // Canonical separator-delimited concatenation; never log this string
        // (review-standards Sec5).
        var sb = new StringBuilder(
            tenantId.Length + sessionId.Length + runId.Length +
            stepId.Length + llmCallId.Length + trigger.Length + 5);
        sb.Append(tenantId).Append(Separator)
          .Append(sessionId).Append(Separator)
          .Append(runId).Append(Separator)
          .Append(stepId).Append(Separator)
          .Append(llmCallId).Append(Separator)
          .Append(trigger);
        return sb.ToString();
    }

    /// <summary>
    /// Derive the idempotency-key bytes for the canonical (tenant, session,
    /// run, step, llm_call, trigger) tuple.
    /// </summary>
    public static byte[] Derive(
        string tenantId,
        string sessionId,
        string runId,
        string stepId,
        string llmCallId,
        string trigger)
    {
        string preimage = CanonicalPreImage(
            tenantId, sessionId, runId, stepId, llmCallId, trigger);
        byte[] preimageBytes = Encoding.UTF8.GetBytes(preimage);

        // XxHash3-128 produces 16 bytes; matches the SDK's expected key length
        // for pre-release parity tests.
        var hash = new XxHash3();
        hash.Append(preimageBytes);
        ulong digest = hash.GetCurrentHashAsUInt64();

        // Pack to canonical 16-byte big-endian layout: (preimage-length:8) ||
        // (digest:8). Length is included so two distinct pre-images with the
        // same XxHash3 collision still produce distinct keys (defense in
        // depth until the BLAKE2b switch).
        byte[] key = new byte[16];
        long len = preimageBytes.Length;
        for (int i = 7; i >= 0; i--)
        {
            key[i] = (byte)(len & 0xFF);
            len >>= 8;
        }
        for (int i = 15; i >= 8; i--)
        {
            key[i] = (byte)(digest & 0xFF);
            digest >>= 8;
        }

        return key;
    }

    /// <summary>
    /// Hex-encoded form of <see cref="Derive"/> for logging / display.
    /// Reviewer Sec5: only the hex digest can appear in logs, never the
    /// pre-image.
    /// </summary>
    public static string DeriveHex(
        string tenantId,
        string sessionId,
        string runId,
        string stepId,
        string llmCallId,
        string trigger)
    {
        byte[] key = Derive(tenantId, sessionId, runId, stepId, llmCallId, trigger);
        var sb = new StringBuilder(key.Length * 2);
        foreach (byte b in key)
        {
            sb.Append(b.ToString("x2", CultureInfo.InvariantCulture));
        }
        return sb.ToString();
    }
}
