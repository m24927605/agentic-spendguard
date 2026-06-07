// SPDX-License-Identifier: Apache-2.0
// Copyright (c) SpendGuard Authors.
//
// COV_d07 SLICE 8 — both-language MAF demo (the .NET half).
//
// Two modes, dispatched via the first arg:
//   --mock   In-process SpendGuardChatMiddleware against a stub
//            ISidecarClient + in-memory IChatClient. No sidecar, no
//            counting-stub required. Drives 3 calls (ALLOW + DENY +
//            ALLOW2) and exits 0 on PASS / 7 on FAIL.
//
//   --real   Connect to the SpendGuard sidecar UDS, register the
//            real `SpendGuardChatMiddleware` via the DI extension, and
//            drive 3 chat-client calls against a counting-stub-backed
//            IChatClient. The DEMO_MODE=maf_dotnet_real Makefile target
//            wires this up.
//
// 3-step matrix (mirrors D04 / D06 / D08 composite demos):
//   step 1 ALLOW   — small message within budget → counter +1.
//   step 2 DENY    — message tagged "trigger-deny" so the local stub
//                    ISidecarClient (in --mock) or the contract
//                    evaluator (in --real) emits SPENDGUARD_DENY → the
//                    middleware throws SpendGuardDecisionDeniedException
//                    BEFORE the inner IChatClient HTTP fires → counter
//                    unchanged.
//   step 3 ALLOW2  — second ALLOW call exercising cross-call
//                    determinism (streaming-per-chunk gating is v0.1.x
//                    non-goal — same shape as openai-agents-ts D08).
//
// Success line (LOCKED — CI grep depends on the exact spelling, mirrors
// the openai_agents_ts / inngest_agent_kit composite convention):
//
//     `[demo] maf_dotnet ALL 3 steps PASS (ALLOW + DENY + ALLOW2)`
//
// Launched by:
//   - direct `dotnet run --project examples/maf-dotnet -- --mock` for
//     laptop iteration.
//   - deploy/demo/demo/run_demo.py::run_maf_dotnet_mode in the
//     `DEMO_MODE=maf_dotnet_real` Makefile target.

using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Net.Http;
using System.Net.Http.Json;
using System.Text.Json;
using System.Text.Json.Serialization;
using System.Threading;
using System.Threading.Tasks;

using Microsoft.Extensions.AI;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Options;

using Spendguard.AgentFramework.Extensions;
using Spendguard.AgentFramework.Middleware;
using Spendguard.AgentFramework.Options;
using Spendguard.AgentFramework.Sidecar;
using Spendguard.Common.V1;
using Spendguard.SidecarAdapter.V1;

namespace Spendguard.Examples.MafDotnet;

internal static class Program
{
    private static readonly string SocketPath =
        Environment.GetEnvironmentVariable("SPENDGUARD_SIDECAR_UDS")
        ?? "/var/run/spendguard/adapter.sock";

    private static readonly string TenantId =
        Environment.GetEnvironmentVariable("SPENDGUARD_TENANT_ID")
        ?? "00000000-0000-4000-8000-000000000001";

    private static readonly string BudgetId =
        Environment.GetEnvironmentVariable("SPENDGUARD_BUDGET_ID")
        ?? "44444444-4444-4444-8444-444444444444";

    private static readonly string WindowInstanceId =
        Environment.GetEnvironmentVariable("SPENDGUARD_WINDOW_INSTANCE_ID")
        ?? "55555555-5555-4555-8555-555555555555";

    private static readonly string CountingStubUrl =
        Environment.GetEnvironmentVariable("SPENDGUARD_COUNTING_STUB_URL")
        ?? "http://counting-stub:8765";

    private static readonly int HandshakeTimeoutMs =
        int.TryParse(Environment.GetEnvironmentVariable("SPENDGUARD_HANDSHAKE_TIMEOUT_MS"), out var v)
            ? v
            : 30_000;

    public static async Task<int> Main(string[] args)
    {
        bool useReal = args.Any(a => a == "--real");
        bool useMock = args.Any(a => a == "--mock") || !useReal;

        try
        {
            return useReal ? await RealMainAsync() : await MockMainAsync();
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine($"[demo] FAIL: {ex.GetType().Name}: {ex.Message}");
            Console.Error.WriteLine(ex.StackTrace);
            return 7;
        }
    }

    // ─── --mock implementation ─────────────────────────────────────────────

    private static async Task<int> MockMainAsync()
    {
        Console.WriteLine(
            "[demo] maf_dotnet driver: --mock mode (no sidecar, in-process stubs)");

        using var loggerFactory = LoggerFactory.Create(b => b.AddConsole());

        // 1. Build a stub ISidecarClient. We want CONTINUE on every call
        //    except when the prompt contains "trigger-deny" — in which case
        //    we return STOP. That maps directly to the contract-evaluator
        //    rule the --real path tests against (BUDGET_EXCEEDED on
        //    spendguard_estimate_override=2000000000), without needing a
        //    real sidecar.
        var stubSidecar = new MockSidecarClient();

        // 2. Build the options bag.
        var opts = new SpendGuardOptions
        {
            TenantId = TenantId,
            BudgetId = BudgetId,
            WindowInstanceId = WindowInstanceId,
            SidecarSocketPath = SocketPath,
            OnSidecarUnavailable = OnSidecarUnavailable.Deny,
        };
        opts.Validate();

        // 3. Inner stub IChatClient counts every invocation. The middleware
        //    sits in front; we assert the inner is NEVER called when the
        //    sidecar emits STOP.
        var inner = new CountingChatClient();
        var middleware = new SpendGuardChatMiddleware(
            innerClient: inner,
            sidecar: stubSidecar,
            options: new OptionsWrapper<SpendGuardOptions>(opts),
            estimator: null,
            logger: loggerFactory.CreateLogger<SpendGuardChatMiddleware>());

        // ALLOW
        Console.WriteLine("[demo] (1) ALLOW step — small message within budget");
        var r1 = await middleware.GetResponseAsync(new[]
        {
            new ChatMessage(ChatRole.User, "hi from .NET"),
        });
        if (inner.CallCount != 1)
        {
            throw new InvalidOperationException(
                $"FATAL ALLOW: inner.CallCount={inner.CallCount} (expected 1)");
        }
        if (r1 is null)
        {
            throw new InvalidOperationException("FATAL ALLOW: response was null");
        }

        // DENY
        Console.WriteLine("[demo] (2) DENY step — forcing SpendGuard STOP");
        bool denied = false;
        try
        {
            // The mock sidecar sniffs ChatOptions.ModelId for the
            // "trigger-deny" marker since the .NET middleware forwards
            // that field into DecisionRequest.Inputs.ClaimEstimate.Model.
            // In --real mode the contract evaluator picks up
            // `spendguard_estimate_override` from the inner HTTP body
            // instead; we simulate that path here without round-tripping
            // through the inner client (the inner is what we're proving
            // never fires on DENY).
            await middleware.GetResponseAsync(
                new[]
                {
                    new ChatMessage(ChatRole.User, "trigger-deny: please block me"),
                },
                new ChatOptions { ModelId = "gpt-4o-mini-trigger-deny" });
        }
        catch (SpendGuardDecisionDeniedException)
        {
            denied = true;
        }
        if (!denied)
        {
            throw new InvalidOperationException(
                "FATAL DENY: middleware did NOT throw SpendGuardDecisionDeniedException");
        }
        if (inner.CallCount != 1)
        {
            throw new InvalidOperationException(
                $"FATAL DENY INV-1.6: inner was called; CallCount={inner.CallCount} (expected 1)");
        }

        // ALLOW2 (instead of STREAM — design.md §3 non-goal)
        Console.WriteLine("[demo] (3) ALLOW2 step — second small message within budget");
        var r3 = await middleware.GetResponseAsync(new[]
        {
            new ChatMessage(ChatRole.User, "another hi"),
        });
        if (inner.CallCount != 2)
        {
            throw new InvalidOperationException(
                $"FATAL ALLOW2: inner.CallCount={inner.CallCount} (expected 2)");
        }
        if (r3 is null)
        {
            throw new InvalidOperationException("FATAL ALLOW2: response was null");
        }

        Console.WriteLine("[demo] maf_dotnet ALL 3 steps PASS (ALLOW + DENY + ALLOW2)");
        Console.WriteLine(
            $"[demo] summary: reserve={stubSidecar.RequestDecisionCount} inner.CallCount={inner.CallCount}");
        return 0;
    }

    // ─── --real implementation ─────────────────────────────────────────────

    private static async Task<int> RealMainAsync()
    {
        Console.WriteLine(
            $"[demo] maf_dotnet driver: --real mode socket={SocketPath} " +
            $"tenant={TenantId} counting_stub={CountingStubUrl}");

        using var loggerFactory = LoggerFactory.Create(b => b.AddConsole());

        // 1. Wait for the sidecar UDS to be visible (Docker volume race).
        await WaitForSocketAsync(SocketPath, TimeSpan.FromMilliseconds(HandshakeTimeoutMs));

        // 2. Wire the DI container.
        var services = new ServiceCollection();
        services.AddSingleton<ILoggerFactory>(loggerFactory);
        services.AddLogging();
        services.AddSpendGuard(o =>
        {
            o.TenantId = TenantId;
            o.BudgetId = BudgetId;
            o.WindowInstanceId = WindowInstanceId;
            o.SidecarSocketPath = SocketPath;
            o.OnSidecarUnavailable = OnSidecarUnavailable.Deny;
        });
        await using var sp = services.BuildServiceProvider();

        // 3. Drive a handshake before middleware fires (S3 reviewer gate).
        var sidecar = sp.GetRequiredService<ISidecarClient>();
        await sidecar.HandshakeAsync(
            TenantId,
            sdkVersion: "0.1.0-pre",
            runtimeKind: "microsoft-agent-framework-dotnet");
        Console.WriteLine($"[demo] handshake ok session_id={sidecar.SessionId}");

        // 4. Build the inner IChatClient: a HTTP-backed adapter that hits
        //    the demo counting-stub. We wrap it in the SpendGuard middleware
        //    via the IChatClient.UseSpendGuard extension — the same wire
        //    shape documented in docs/integrations/microsoft-agent-framework.mdx.
        using var http = new HttpClient { BaseAddress = new Uri(CountingStubUrl) };
        IChatClient inner = new CountingStubChatClient(http);
        IChatClient gated = inner.UseSpendGuard(sp);

        var preAllow = await ReadCountingStubHitsAsync(http);

        // ALLOW
        Console.WriteLine("[demo] (1) ALLOW step — small message within budget");
        var r1 = await gated.GetResponseAsync(new[]
        {
            new ChatMessage(ChatRole.User, "hi from .NET"),
        });
        if (r1 is null)
        {
            throw new InvalidOperationException("FATAL ALLOW: response was null");
        }
        var postAllow = await ReadCountingStubHitsAsync(http);
        if (postAllow != preAllow + 1)
        {
            throw new InvalidOperationException(
                $"FATAL ALLOW: counting-stub pre={preAllow} post={postAllow} (expected +1)");
        }

        // DENY
        Console.WriteLine("[demo] (2) DENY step — forcing hard-cap overflow");
        var preDeny = postAllow;
        bool denied = false;
        try
        {
            await gated.GetResponseAsync(new[]
            {
                new ChatMessage(ChatRole.User, "trigger-deny: please block me"),
            });
        }
        catch (SpendGuardDecisionDeniedException ex)
        {
            denied = true;
            Console.WriteLine(
                $"[demo] (2) DENY caught SpendGuardDecisionDeniedException: {ex.Message}");
        }
        if (!denied)
        {
            throw new InvalidOperationException(
                "FATAL DENY: middleware did NOT throw SpendGuardDecisionDeniedException");
        }
        var postDeny = await ReadCountingStubHitsAsync(http);
        if (postDeny != preDeny)
        {
            throw new InvalidOperationException(
                $"FATAL DENY INV-1.6: counting-stub pre={preDeny} post={postDeny} (expected 0)");
        }

        // ALLOW2 (STREAM replacement — design.md §3 non-goal)
        Console.WriteLine("[demo] (3) ALLOW2 step — second small message within budget");
        var preAllow2 = postDeny;
        var r3 = await gated.GetResponseAsync(new[]
        {
            new ChatMessage(ChatRole.User, "another hi"),
        });
        if (r3 is null)
        {
            throw new InvalidOperationException("FATAL ALLOW2: response was null");
        }
        var postAllow2 = await ReadCountingStubHitsAsync(http);
        if (postAllow2 != preAllow2 + 1)
        {
            throw new InvalidOperationException(
                $"FATAL ALLOW2: counting-stub pre={preAllow2} post={postAllow2} (expected +1)");
        }

        Console.WriteLine("[demo] maf_dotnet ALL 3 steps PASS (ALLOW + DENY + ALLOW2)");
        return 0;
    }

    // ─── Helpers ────────────────────────────────────────────────────────────

    private static async Task WaitForSocketAsync(string path, TimeSpan timeout)
    {
        var deadline = DateTime.UtcNow + timeout;
        while (DateTime.UtcNow < deadline)
        {
            // UDS path is a UNIX file; we don't need IsConnected — File.Exists
            // is enough to know the sidecar created the socket.
            if (File.Exists(path))
            {
                Console.WriteLine($"[demo] sidecar UDS visible at {path}");
                return;
            }
            await Task.Delay(1_000);
        }
        throw new TimeoutException(
            $"sidecar UDS at {path} did not appear within {timeout.TotalSeconds:F0}s");
    }

    private static async Task<int> ReadCountingStubHitsAsync(HttpClient http)
    {
        var r = await http.GetAsync("/_count");
        r.EnsureSuccessStatusCode();
        var body = await r.Content.ReadFromJsonAsync<CountingStubCount>();
        return body?.Calls ?? 0;
    }

    private sealed class CountingStubCount
    {
        [JsonPropertyName("calls")] public int Calls { get; set; }
    }

    // ─── Mock SidecarClient (mock mode only) ───────────────────────────────

    /// <summary>
    /// Minimal in-process ISidecarClient stub. CONTINUE on every call
    /// unless the incoming claim_estimate's model field or any input
    /// signal carries "trigger-deny" — then STOP. Mirrors the contract
    /// evaluator's BUDGET_EXCEEDED path the --real demo drives.
    /// </summary>
    private sealed class MockSidecarClient : ISidecarClient
    {
        private bool _handshook;
        public bool IsHandshakeComplete => _handshook;
        public string SessionId { get; private set; } = "session-mock-1";
        public int RequestDecisionCount { get; private set; }

        public Task<HandshakeResponse> HandshakeAsync(
            string tenantIdAssertion, string sdkVersion, string runtimeKind,
            CancellationToken ct = default)
        {
            _handshook = true;
            return Task.FromResult(new HandshakeResponse { SessionId = SessionId });
        }

        public Task<DecisionResponse> RequestDecisionAsync(
            DecisionRequest request, CancellationToken ct = default)
        {
            RequestDecisionCount += 1;
            // Sniff the projected claim's model field for a "trigger-deny" tag
            // — that's the marker the demo driver writes when issuing the DENY
            // call. In production the sidecar's contract evaluator does this
            // upstream; we simulate the path for the mock-only smoke test.
            string sniff = request.Inputs?.ClaimEstimate?.Model ?? string.Empty;
            if (sniff.Contains("trigger-deny", StringComparison.Ordinal))
            {
                return Task.FromResult(new DecisionResponse
                {
                    Decision = DecisionResponse.Types.Decision.Stop,
                    DecisionId = $"dec-{RequestDecisionCount}",
                    RunCodeTriggered = "BUDGET_EXCEEDED",
                });
            }
            var resp = new DecisionResponse
            {
                Decision = DecisionResponse.Types.Decision.Continue,
                DecisionId = $"dec-{RequestDecisionCount}",
                LedgerTransactionId = $"lgr-{RequestDecisionCount}",
            };
            resp.ReservationIds.Add($"res-{RequestDecisionCount}");
            return Task.FromResult(resp);
        }

        public Task<ReleaseReservationResponse> ReleaseReservationAsync(
            ReleaseReservationRequest request, CancellationToken ct = default)
            => Task.FromResult(new ReleaseReservationResponse());

        public void Dispose() { }
    }

    // ─── Counting IChatClient (mock mode only) ─────────────────────────────

    private sealed class CountingChatClient : IChatClient
    {
        public int CallCount { get; private set; }

        public ChatClientMetadata Metadata { get; } = new("mock-counting", new Uri("http://localhost"));

        public Task<ChatResponse> GetResponseAsync(
            IEnumerable<ChatMessage> messages,
            ChatOptions? options = null,
            CancellationToken cancellationToken = default)
        {
            CallCount += 1;
            var resp = new ChatResponse(new ChatMessage(
                ChatRole.Assistant, $"hi from mock #{CallCount}"));
            resp.Usage = new UsageDetails
            {
                InputTokenCount = 5,
                OutputTokenCount = 7,
                TotalTokenCount = 12,
            };
            return Task.FromResult(resp);
        }

        public IAsyncEnumerable<ChatResponseUpdate> GetStreamingResponseAsync(
            IEnumerable<ChatMessage> messages,
            ChatOptions? options = null,
            CancellationToken cancellationToken = default)
            => throw new NotSupportedException("mock streaming not implemented");

        public object? GetService(Type serviceType, object? serviceKey = null) => null;

        public void Dispose() { }
    }

    // ─── HTTP-backed IChatClient hitting the counting stub (real mode) ─────

    /// <summary>
    /// Minimal IChatClient that POSTs to the demo's counting-stub
    /// /v1/chat/completions endpoint and inflates the response back to a
    /// `ChatResponse`. The middleware sits in front; this client is the
    /// "inner" boundary the middleware delegates to on CONTINUE.
    /// </summary>
    private sealed class CountingStubChatClient : IChatClient
    {
        private readonly HttpClient _http;
        public ChatClientMetadata Metadata { get; } =
            new("counting-stub", new Uri("http://counting-stub:8765"));

        public CountingStubChatClient(HttpClient http)
        {
            _http = http;
        }

        public async Task<ChatResponse> GetResponseAsync(
            IEnumerable<ChatMessage> messages,
            ChatOptions? options = null,
            CancellationToken cancellationToken = default)
        {
            // Forward the message list to the counting stub. The stub
            // ignores the body shape; the call itself is what counts.
            var msgs = messages.Select(m => new
            {
                role = m.Role.Value,
                content = m.Text ?? string.Empty,
            }).ToList();
            var body = new Dictionary<string, object?>
            {
                ["model"] = options?.ModelId ?? "gpt-4o-mini",
                ["messages"] = msgs,
            };
            // The DENY case carries the spendguard_estimate_override marker
            // so the sidecar's contract evaluator emits SPENDGUARD_DENY.
            // This branch is exercised only when CONTINUE — the middleware
            // never delegates here on STOP — so it's effectively a no-op,
            // but we keep the marker forwarding here for parity with the
            // TS demos.
            if (msgs.Any(m => m.content.Contains("trigger-deny", StringComparison.Ordinal)))
            {
                body["spendguard_estimate_override"] = "2000000000";
            }
            using var req = new HttpRequestMessage(HttpMethod.Post, "/v1/chat/completions")
            {
                Content = JsonContent.Create(body),
            };
            using var res = await _http.SendAsync(req, cancellationToken);
            res.EnsureSuccessStatusCode();
            var payload = await res.Content.ReadFromJsonAsync<CountingStubResponse>(
                cancellationToken: cancellationToken);
            var resp = new ChatResponse(new ChatMessage(
                ChatRole.Assistant,
                payload?.Choices?[0]?.Message?.Content ?? "(empty)"));
            if (payload?.Usage is not null)
            {
                resp.Usage = new UsageDetails
                {
                    InputTokenCount = payload.Usage.PromptTokens,
                    OutputTokenCount = payload.Usage.CompletionTokens,
                    TotalTokenCount = payload.Usage.TotalTokens,
                };
            }
            return resp;
        }

        public IAsyncEnumerable<ChatResponseUpdate> GetStreamingResponseAsync(
            IEnumerable<ChatMessage> messages,
            ChatOptions? options = null,
            CancellationToken cancellationToken = default)
            => throw new NotSupportedException("counting-stub streaming not implemented");

        public object? GetService(Type serviceType, object? serviceKey = null) => null;

        public void Dispose() { }
    }

    private sealed class CountingStubResponse
    {
        [JsonPropertyName("choices")] public List<Choice>? Choices { get; set; }
        [JsonPropertyName("usage")] public Usage? Usage { get; set; }
    }

    private sealed class Choice
    {
        [JsonPropertyName("message")] public ChoiceMessage? Message { get; set; }
    }

    private sealed class ChoiceMessage
    {
        [JsonPropertyName("content")] public string? Content { get; set; }
    }

    private sealed class Usage
    {
        [JsonPropertyName("prompt_tokens")] public int PromptTokens { get; set; }
        [JsonPropertyName("completion_tokens")] public int CompletionTokens { get; set; }
        [JsonPropertyName("total_tokens")] public int TotalTokens { get; set; }
    }
}
