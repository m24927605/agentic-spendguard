// SPDX-License-Identifier: Apache-2.0
// Copyright (c) SpendGuard Authors.

using System;
using Spendguard.AgentFramework.Estimators;
using Xunit;

namespace Spendguard.AgentFramework.Tests.Unit;

public sealed class SimpleTokenEstimatorTests
{
    [Fact]
    public void Estimator_ReturnsZero_OnEmptyInput()
    {
        var est = new SimpleTokenEstimator();
        Assert.Equal(0, est.EstimateInputTokens(Array.Empty<string>()));
        Assert.Equal(0, est.EstimateInputTokens(null!));
        Assert.Equal(0, est.EstimateInputTokensForText(string.Empty));
        Assert.Equal(0, est.EstimateInputTokensForText(null));
    }

    [Fact]
    public void Estimator_Counts_CharsOverFour_ByDefault()
    {
        var est = new SimpleTokenEstimator();
        // 16 chars => 4 tokens at default divisor.
        int tokens = est.EstimateInputTokens(new[] { "1234567890123456" });
        Assert.Equal(4, tokens);
    }

    [Fact]
    public void Estimator_CustomDivisor_IsRespected()
    {
        var est = new SimpleTokenEstimator(divisor: 2);
        int tokens = est.EstimateInputTokens(new[] { "1234567890" }); // 10 chars / 2 = 5
        Assert.Equal(5, tokens);
    }

    [Fact]
    public void Estimator_Rejects_ZeroOrNegativeDivisor()
    {
        Assert.Throws<ArgumentOutOfRangeException>(() => new SimpleTokenEstimator(divisor: 0));
        Assert.Throws<ArgumentOutOfRangeException>(() => new SimpleTokenEstimator(divisor: -1));
    }
}
