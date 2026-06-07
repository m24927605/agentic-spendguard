// SPDX-License-Identifier: Apache-2.0
// Copyright (c) SpendGuard Authors.

using Spendguard.AgentFramework.Ids;
using Xunit;

namespace Spendguard.AgentFramework.Tests.Unit;

public sealed class IdempotencyKeyTests
{
    private const string Tenant = "tenant-abc";
    private const string Session = "session-001";
    private const string Run = "run-1";
    private const string Step = "step-1";
    private const string LlmCall = "llm-1";

    [Fact]
    public void CanonicalPreImage_IsStable_AndOrdered()
    {
        string a = IdempotencyKeyDerivation.CanonicalPreImage(Tenant, Session, Run, Step, LlmCall, "LLM_CALL_PRE");
        string b = IdempotencyKeyDerivation.CanonicalPreImage(Tenant, Session, Run, Step, LlmCall, "LLM_CALL_PRE");
        Assert.Equal(a, b);
        Assert.Equal("tenant-abc\nsession-001\nrun-1\nstep-1\nllm-1\nLLM_CALL_PRE", a);
    }

    [Fact]
    public void Derive_IsStable_AcrossInvocations()
    {
        byte[] a = IdempotencyKeyDerivation.Derive(Tenant, Session, Run, Step, LlmCall, "LLM_CALL_PRE");
        byte[] b = IdempotencyKeyDerivation.Derive(Tenant, Session, Run, Step, LlmCall, "LLM_CALL_PRE");
        Assert.Equal(16, a.Length);
        Assert.Equal(a, b);
    }

    [Fact]
    public void Derive_Differs_ByTrigger()
    {
        byte[] pre = IdempotencyKeyDerivation.Derive(Tenant, Session, Run, Step, LlmCall, "LLM_CALL_PRE");
        byte[] tool = IdempotencyKeyDerivation.Derive(Tenant, Session, Run, Step, LlmCall, "TOOL_CALL_PRE");
        Assert.NotEqual(pre, tool);
    }

    [Fact]
    public void DeriveHex_IsLowercase_AndHexOnly()
    {
        string hex = IdempotencyKeyDerivation.DeriveHex(Tenant, Session, Run, Step, LlmCall, "LLM_CALL_PRE");
        Assert.Equal(32, hex.Length);
        foreach (char c in hex)
        {
            Assert.True(
                (c >= '0' && c <= '9') || (c >= 'a' && c <= 'f'),
                $"non-hex char '{c}' in idempotency key");
        }
    }
}
