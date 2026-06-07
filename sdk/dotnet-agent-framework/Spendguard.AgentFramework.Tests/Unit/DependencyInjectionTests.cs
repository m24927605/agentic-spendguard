// SPDX-License-Identifier: Apache-2.0
// Copyright (c) SpendGuard Authors.

using System;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Options;
using Spendguard.AgentFramework.Estimators;
using Spendguard.AgentFramework.Extensions;
using Spendguard.AgentFramework.Options;
using Xunit;

namespace Spendguard.AgentFramework.Tests.Unit;

public sealed class DependencyInjectionTests
{
    [Fact]
    public void AddSpendGuard_RegistersOptions_AndDefaultEstimator()
    {
        var services = new ServiceCollection();
        services.AddSpendGuard(o =>
        {
            o.TenantId = "t";
            o.BudgetId = "b";
            o.SidecarSocketPath = "/tmp/x.sock";
        });
        var sp = services.BuildServiceProvider();
        var options = sp.GetRequiredService<IOptions<SpendGuardOptions>>();
        Assert.Equal("t", options.Value.TenantId);

        var estimator = sp.GetRequiredService<ITokenEstimator>();
        Assert.NotNull(estimator);
    }

    [Fact]
    public void AddSpendGuard_RejectsNullConfigure()
    {
        var services = new ServiceCollection();
        Assert.Throws<ArgumentNullException>(() =>
            services.AddSpendGuard(null!));
    }

    [Fact]
    public void AddSpendGuard_SurfacesInvalidConfigAtResolve()
    {
        var services = new ServiceCollection();
        services.AddSpendGuard(_ => { /* deliberately leave all required fields blank */ });
        var sp = services.BuildServiceProvider();
        var options = sp.GetRequiredService<IOptions<SpendGuardOptions>>();
        Assert.Throws<ArgumentException>(() => options.Value.Validate());
    }
}
