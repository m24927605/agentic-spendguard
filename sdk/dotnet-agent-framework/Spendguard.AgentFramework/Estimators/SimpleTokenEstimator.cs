// SPDX-License-Identifier: Apache-2.0
// Copyright (c) SpendGuard Authors.

using System;
using System.Collections.Generic;
using System.Text;

namespace Spendguard.AgentFramework.Estimators;

/// <summary>
/// Abstraction over a tokenizer used to estimate prompt-side token counts
/// before the provider call is made (driving <c>LLM_CALL_PRE</c> reservation
/// amounts).
/// </summary>
public interface ITokenEstimator
{
    /// <summary>
    /// Rough estimate of the number of input tokens used by the given prompt
    /// fragments. Implementations MUST be bounded: O(n) over prompt characters,
    /// per review-standards Sec4.
    /// </summary>
    /// <param name="prompts">Prompt strings, in caller-supplied order.</param>
    /// <returns>Estimated input token count (always &gt;= 0).</returns>
    int EstimateInputTokens(IEnumerable<string> prompts);

    /// <summary>
    /// Convenience overload over a single text blob. Implementations should
    /// preserve the same semantics as <see cref="EstimateInputTokens"/>.
    /// </summary>
    int EstimateInputTokensForText(string? text);
}

/// <summary>
/// chars/4 heuristic estimator (Tier 3 fallback per
/// <c>egress_proxy/src/decision.rs</c> historical heuristic).
///
/// Intentionally simple: this is the default pre-Tier-2 estimator. Production
/// adopters should swap in SharpToken (for OpenAI models) or the sidecar's
/// tokenizer UDS service (for non-OpenAI models). Both are tracked as
/// follow-up work in design.md §3.3 ADR-004.
/// </summary>
public sealed class SimpleTokenEstimator : ITokenEstimator
{
    private readonly int _divisor;

    /// <summary>
    /// Build a chars/<paramref name="divisor"/> estimator. Default divisor is 4.
    /// Reviewers (Sec4) bound the divisor so a misconfigured 0 cannot loop or
    /// throw at runtime.
    /// </summary>
    /// <param name="divisor">Positive integer divisor (default 4).</param>
    public SimpleTokenEstimator(int divisor = 4)
    {
        if (divisor < 1)
        {
            throw new ArgumentOutOfRangeException(
                nameof(divisor),
                "SimpleTokenEstimator divisor must be >= 1.");
        }

        _divisor = divisor;
    }

    /// <inheritdoc/>
    public int EstimateInputTokens(IEnumerable<string> prompts)
    {
        if (prompts is null)
        {
            return 0;
        }

        long chars = 0;
        foreach (string p in prompts)
        {
            if (p is null)
            {
                continue;
            }

            chars += p.Length;
        }

        // Bounded division; clamp to int.MaxValue for downstream NUMERIC(38,0)
        // safety (Sec5: prompt content never logged, only counts).
        long estimated = chars / _divisor;
        if (estimated > int.MaxValue)
        {
            return int.MaxValue;
        }

        return (int)estimated;
    }

    /// <summary>
    /// Helper that joins string content from chat messages and routes through
    /// <see cref="EstimateInputTokens"/>. Returns 0 on empty / null input.
    /// </summary>
    public int EstimateInputTokensForText(string? text)
    {
        if (string.IsNullOrEmpty(text))
        {
            return 0;
        }

        return EstimateInputTokens(new[] { text! });
    }

    /// <summary>
    /// Encoding-aware variant: counts bytes / divisor instead of chars / divisor
    /// for non-UTF-16 strings. Reserved for future Tier-2 wiring; currently
    /// behaves identically to <see cref="EstimateInputTokens"/> because all
    /// callers feed UTF-16 strings.
    /// </summary>
    public int EstimateInputTokensUtf8(IEnumerable<string> prompts)
    {
        if (prompts is null)
        {
            return 0;
        }

        long bytes = 0;
        foreach (string p in prompts)
        {
            if (p is null)
            {
                continue;
            }

            bytes += Encoding.UTF8.GetByteCount(p);
        }

        long estimated = bytes / _divisor;
        if (estimated > int.MaxValue)
        {
            return int.MaxValue;
        }

        return (int)estimated;
    }
}
