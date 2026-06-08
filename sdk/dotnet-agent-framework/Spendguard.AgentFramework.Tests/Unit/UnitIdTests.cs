// SPDX-License-Identifier: Apache-2.0
// Copyright (c) SpendGuard Authors.
//
// HARDEN_D05_UR_S02 — `UnitId` option plumbing tests for the .NET adapter.
// Mirrors the LOCKED per-adapter TN-01 / TN-02 / TN-03 pattern from
// docs/specs/harden_d05_unit_ref/tests.md §2.3.
//
// SLICE 2 contract (additive only):
//   - TN-01: SpendGuardOptions.UnitId is nullable Guid? present on options.
//   - TN-02: SidecarClient.RequestDecision plumbs UnitId through to
//     DecisionRequest.Inputs.ProjectedUnit.UnitId on the wire.
//   - TN-03: Backward compat — null UnitId leaves ProjectedUnit unset
//     (preserves the pre-HARDEN_D05_UR wire shape).

using System;
using System.Threading.Tasks;
using Microsoft.Extensions.AI;
using Microsoft.Extensions.Logging.Abstractions;
using Microsoft.Extensions.Options;
using Spendguard.AgentFramework.Middleware;
using Spendguard.AgentFramework.Options;
using Spendguard.AgentFramework.Tests.Helpers;
using Spendguard.SidecarAdapter.V1;
using Xunit;

namespace Spendguard.AgentFramework.Tests.Unit;

public sealed class UnitIdTests
{
    private static readonly Guid UnitIdFixture =
        Guid.Parse("550e8400-e29b-41d4-a716-446655440000");

    private static SpendGuardOptions OptsWithUnitId(Guid? unitId)
    {
        return new SpendGuardOptions
        {
            TenantId = "tenant-unitid-test",
            BudgetId = "budget-unitid-test",
            SidecarSocketPath = "/tmp/spendguard.sock",
            OnSidecarUnavailable = OnSidecarUnavailable.Deny,
            UnitId = unitId,
        };
    }

    private static SpendGuardChatMiddleware Build(
        FakeSidecarClient sidecar,
        IChatClient inner,
        SpendGuardOptions opts)
    {
        return new SpendGuardChatMiddleware(
            inner,
            sidecar,
            Microsoft.Extensions.Options.Options.Create(opts),
            estimator: null,
            logger: NullLogger<SpendGuardChatMiddleware>.Instance);
    }

    [Fact]
    public void TN_01_UnitId_IsNullableGuid_DefaultNull()
    {
        var opts = new SpendGuardOptions();
        Assert.Null(opts.UnitId);

        opts.UnitId = UnitIdFixture;
        Assert.Equal(UnitIdFixture, opts.UnitId);

        opts.UnitId = null;
        Assert.Null(opts.UnitId);
    }

    [Fact]
    public async Task TN_02_RequestDecision_ThreadsUnitIdToProjectedUnit()
    {
        var sidecar = new FakeSidecarClient();
        sidecar.DecisionProducers.Add(_ => new DecisionResponse
        {
            DecisionId = "d-unitid",
            Decision = DecisionResponse.Types.Decision.Continue,
        });
        var inner = FakeChatClient.WithUsage(10, 5, "ok");
        var mw = Build(sidecar, inner, OptsWithUnitId(UnitIdFixture));

        await mw.GetResponseAsync(new[] { new ChatMessage(ChatRole.User, "hi") });

        Assert.Single(sidecar.DecisionCalls);
        DecisionRequest req = sidecar.DecisionCalls[0];
        Assert.NotNull(req.Inputs);
        Assert.NotNull(req.Inputs.ProjectedUnit);
        Assert.Equal(UnitIdFixture.ToString(), req.Inputs.ProjectedUnit.UnitId);
    }

    [Fact]
    public async Task TN_03_BackwardCompat_NullUnitId_LeavesProjectedUnitUnset()
    {
        var sidecar = new FakeSidecarClient();
        sidecar.DecisionProducers.Add(_ => new DecisionResponse
        {
            DecisionId = "d-no-unitid",
            Decision = DecisionResponse.Types.Decision.Continue,
        });
        var inner = FakeChatClient.WithUsage(10, 5, "ok");
        var mw = Build(sidecar, inner, OptsWithUnitId(null));

        await mw.GetResponseAsync(new[] { new ChatMessage(ChatRole.User, "hi") });

        Assert.Single(sidecar.DecisionCalls);
        DecisionRequest req = sidecar.DecisionCalls[0];
        Assert.NotNull(req.Inputs);
        // ProjectedUnit is a proto message field — when unset the proto stub
        // exposes it as null (not a default-constructed UnitRef). This
        // preserves the pre-HARDEN_D05_UR wire shape exactly.
        Assert.Null(req.Inputs.ProjectedUnit);
    }
}
