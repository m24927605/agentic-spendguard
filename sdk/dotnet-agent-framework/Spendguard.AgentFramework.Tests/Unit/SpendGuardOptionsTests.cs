// SPDX-License-Identifier: Apache-2.0
// Copyright (c) SpendGuard Authors.

using System;
using Spendguard.AgentFramework.Options;
using Xunit;

namespace Spendguard.AgentFramework.Tests.Unit;

public sealed class SpendGuardOptionsTests
{
    [Fact]
    public void Options_FailClosed_IsDefault()
    {
        var opts = new SpendGuardOptions();
        Assert.Equal(OnSidecarUnavailable.Deny, opts.OnSidecarUnavailable);
    }

    [Fact]
    public void Options_DefaultSocketPath_IsSetToSpendguardConvention()
    {
        var opts = new SpendGuardOptions();
        Assert.Equal("/var/run/spendguard/adapter.sock", opts.SidecarSocketPath);
    }

    [Fact]
    public void Validate_Rejects_EmptyBudgetId()
    {
        var opts = new SpendGuardOptions { TenantId = "t", BudgetId = "" };
        var ex = Assert.Throws<ArgumentException>(() => opts.Validate());
        Assert.Contains("BudgetId", ex.Message);
    }

    [Fact]
    public void Validate_Rejects_EmptySocketPath()
    {
        var opts = new SpendGuardOptions { TenantId = "t", BudgetId = "b", SidecarSocketPath = "" };
        var ex = Assert.Throws<ArgumentException>(() => opts.Validate());
        Assert.Contains("SidecarSocketPath", ex.Message);
    }

    [Fact]
    public void Validate_Rejects_EmptyTenantId()
    {
        var opts = new SpendGuardOptions { BudgetId = "b", TenantId = "" };
        var ex = Assert.Throws<ArgumentException>(() => opts.Validate());
        Assert.Contains("TenantId", ex.Message);
    }

    [Fact]
    public void Validate_Accepts_FullyPopulatedOptions()
    {
        var opts = new SpendGuardOptions
        {
            TenantId = "t",
            BudgetId = "b",
            SidecarSocketPath = "/tmp/x.sock",
        };
        opts.Validate(); // no throw
    }
}
