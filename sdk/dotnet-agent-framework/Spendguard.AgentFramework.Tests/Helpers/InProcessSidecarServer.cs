// SPDX-License-Identifier: Apache-2.0
// Copyright (c) SpendGuard Authors.

using System;
using System.Collections.Generic;
using System.Threading;
using System.Threading.Tasks;
using Grpc.Core;
using Grpc.Net.Client;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Hosting;
using Microsoft.AspNetCore.TestHost;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Hosting;
using Spendguard.SidecarAdapter.V1;

namespace Spendguard.AgentFramework.Tests.Helpers;

/// <summary>
/// In-process sidecar gRPC server backed by TestServer. Lets the integration
/// tests exercise the real generated stubs end-to-end without a real UDS
/// endpoint. Mirrors review-standards T2's "sidecar stub" fixture.
/// </summary>
public sealed class InProcessSidecarServer : IAsyncDisposable
{
    private IHost? _host;

    /// <summary>Reachable channel for the test client.</summary>
    public GrpcChannel Channel { get; private set; } = default!;

    /// <summary>Captured decision requests in arrival order.</summary>
    public List<DecisionRequest> CapturedDecisionRequests { get; } = new();

    /// <summary>Captured handshake requests in arrival order.</summary>
    public List<HandshakeRequest> CapturedHandshakes { get; } = new();

    /// <summary>Decisions to play back in arrival order; cycles after the last entry.</summary>
    public List<Func<DecisionRequest, DecisionResponse>> DecisionPlanner { get; } = new();

    /// <summary>Spin up the in-process server.</summary>
    public static async Task<InProcessSidecarServer> StartAsync(
        Action<InProcessSidecarServer>? configure = null)
    {
        var server = new InProcessSidecarServer();
        configure?.Invoke(server);

        var builder = Host.CreateDefaultBuilder()
            .ConfigureWebHostDefaults(webBuilder =>
            {
                webBuilder
                    .UseTestServer()
                    .ConfigureServices(services =>
                    {
                        services.AddGrpc();
                        services.AddSingleton(server);
                    })
                    .Configure(app =>
                    {
                        app.UseRouting();
                        app.UseEndpoints(endpoints =>
                        {
                            endpoints.MapGrpcService<StubSidecarService>();
                        });
                    });
            });

        IHost host = await builder.StartAsync();
        var testServer = host.GetTestServer();
        var channel = GrpcChannel.ForAddress(testServer.BaseAddress, new GrpcChannelOptions
        {
            HttpHandler = testServer.CreateHandler(),
            Credentials = ChannelCredentials.Insecure,
        });

        server._host = host;
        server.Channel = channel;
        return server;
    }

    /// <inheritdoc/>
    public async ValueTask DisposeAsync()
    {
        Channel?.Dispose();
        if (_host is not null)
        {
            await _host.StopAsync();
            _host.Dispose();
        }
    }
}

/// <summary>gRPC stub service used by the in-process server.</summary>
internal sealed class StubSidecarService : Spendguard.SidecarAdapter.V1.SidecarAdapter.SidecarAdapterBase
{
    private readonly InProcessSidecarServer _state;
    private int _planIdx;

    public StubSidecarService(InProcessSidecarServer state)
    {
        _state = state;
    }

    public override Task<HandshakeResponse> Handshake(HandshakeRequest request, ServerCallContext context)
    {
        _state.CapturedHandshakes.Add(request);
        return Task.FromResult(new HandshakeResponse
        {
            SidecarVersion = "stub-0.0.0",
            ProtocolVersion = 1u,
            SessionId = "session-stub-0001",
        });
    }

    public override Task<DecisionResponse> RequestDecision(DecisionRequest request, ServerCallContext context)
    {
        _state.CapturedDecisionRequests.Add(request);

        if (_state.DecisionPlanner.Count == 0)
        {
            return Task.FromResult(new DecisionResponse
            {
                DecisionId = $"decision-{Guid.NewGuid()}",
                Decision = DecisionResponse.Types.Decision.Continue,
            });
        }

        int idx = Interlocked.Increment(ref _planIdx) - 1;
        Func<DecisionRequest, DecisionResponse> planFn = _state.DecisionPlanner[idx % _state.DecisionPlanner.Count];
        return Task.FromResult(planFn(request));
    }

    public override Task<ReleaseReservationResponse> ReleaseReservation(
        ReleaseReservationRequest request,
        ServerCallContext context)
    {
        return Task.FromResult(new ReleaseReservationResponse
        {
            LedgerTransactionId = $"release-{request.ReservationId}",
        });
    }
}
