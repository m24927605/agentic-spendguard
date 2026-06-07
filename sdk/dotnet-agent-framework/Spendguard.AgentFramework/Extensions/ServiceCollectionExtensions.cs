// SPDX-License-Identifier: Apache-2.0
// Copyright (c) SpendGuard Authors.

using System;
using Microsoft.Extensions.AI;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.DependencyInjection.Extensions;
using Spendguard.AgentFramework.Estimators;
using Spendguard.AgentFramework.Middleware;
using Spendguard.AgentFramework.Options;
using Spendguard.AgentFramework.Sidecar;

namespace Spendguard.AgentFramework.Extensions;

/// <summary>
/// DI wiring for the SpendGuard MAF middleware. Cross-language parity per
/// review-standards P2 (matches Python <c>spendguard.integrations.agent_framework</c>
/// registration shape).
/// </summary>
public static class ServiceCollectionExtensions
{
    /// <summary>
    /// Register the SpendGuard middleware against the current DI container.
    /// Wires up:
    ///   * <see cref="SpendGuardOptions"/> (validated at registration).
    ///   * <see cref="ISidecarClient"/> (singleton; UDS-backed gRPC).
    ///   * <see cref="ITokenEstimator"/> (default <see cref="SimpleTokenEstimator"/>).
    /// Consumers wrap their own <see cref="IChatClient"/> with
    /// <see cref="UseSpendGuard"/> when constructing the chat pipeline.
    /// </summary>
    /// <param name="services">DI service collection.</param>
    /// <param name="configure">Caller-supplied configuration delegate.</param>
    public static IServiceCollection AddSpendGuard(
        this IServiceCollection services,
        Action<SpendGuardOptions> configure)
    {
        if (services is null) throw new ArgumentNullException(nameof(services));
        if (configure is null) throw new ArgumentNullException(nameof(configure));

        // Bind & validate immediately. Surfacing config errors at startup is
        // a review-standards N1 / N2 requirement.
        var optionsBuilder = services
            .AddOptions<SpendGuardOptions>()
            .Configure(configure)
            .Validate(o =>
            {
                try
                {
                    o.Validate();
                    return true;
                }
                catch (ArgumentException)
                {
                    return false;
                }
            }, "SpendGuardOptions validation failed (BudgetId / SidecarSocketPath / TenantId required).");

        // Eager-run validation so misconfig surfaces at startup, not on first hit.
        services.AddOptions<SpendGuardOptions>()
            .Configure(configure)
            .PostConfigure(o => o.Validate());

        services.TryAddSingleton<ITokenEstimator>(_ => new SimpleTokenEstimator());

        services.TryAddSingleton<ISidecarClient>(sp =>
        {
            var opts = (Microsoft.Extensions.Options.IOptions<SpendGuardOptions>)
                sp.GetService(typeof(Microsoft.Extensions.Options.IOptions<SpendGuardOptions>))!;
            var loggerFactory = sp.GetService(typeof(Microsoft.Extensions.Logging.ILoggerFactory))
                as Microsoft.Extensions.Logging.ILoggerFactory;
            return SidecarClient.ForSocketPath(opts.Value.SidecarSocketPath, loggerFactory);
        });

        return services;
    }

    /// <summary>
    /// Wrap an <see cref="IChatClient"/> with the SpendGuard middleware. Use
    /// inside an <c>IChatClientBuilder</c> pipeline:
    /// <code>
    /// services.AddChatClient(...).Use((inner, sp) =&gt; inner.UseSpendGuard(sp));
    /// </code>
    /// </summary>
    public static IChatClient UseSpendGuard(this IChatClient inner, IServiceProvider sp)
    {
        if (inner is null) throw new ArgumentNullException(nameof(inner));
        if (sp is null) throw new ArgumentNullException(nameof(sp));

        var sidecar = (ISidecarClient)sp.GetService(typeof(ISidecarClient))!;
        var options = (Microsoft.Extensions.Options.IOptions<SpendGuardOptions>)
            sp.GetService(typeof(Microsoft.Extensions.Options.IOptions<SpendGuardOptions>))!;
        var estimator = sp.GetService(typeof(ITokenEstimator)) as ITokenEstimator;
        var logger = sp.GetService(typeof(Microsoft.Extensions.Logging.ILogger<SpendGuardChatMiddleware>))
            as Microsoft.Extensions.Logging.ILogger<SpendGuardChatMiddleware>;

        return new SpendGuardChatMiddleware(inner, sidecar, options, estimator, logger);
    }
}
