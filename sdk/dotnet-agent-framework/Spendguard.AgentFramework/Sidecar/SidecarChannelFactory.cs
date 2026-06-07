// SPDX-License-Identifier: Apache-2.0
// Copyright (c) SpendGuard Authors.

using System;
using System.IO;
using System.Net.Http;
using System.Net.Sockets;
using System.Threading;
using System.Threading.Tasks;
using Grpc.Net.Client;

namespace Spendguard.AgentFramework.Sidecar;

/// <summary>
/// Builds a <see cref="GrpcChannel"/> wired over a Unix Domain Socket.
/// Reviewer notes:
///   * Sec2 — does not chmod/chown the socket; permissions are sidecar-owned.
///   * Sec1 — never reads credentials; UDS auth is via SO_PEERCRED at the
///     sidecar end per adapter.proto §Handshake.
/// </summary>
public static class SidecarChannelFactory
{
    /// <summary>
    /// Build a UDS-backed gRPC channel for the given socket file path.
    /// Caller owns the returned channel and must dispose it.
    /// </summary>
    /// <param name="socketPath">Filesystem path to the sidecar UDS endpoint.</param>
    /// <param name="address">
    /// Optional dummy address. gRPC.NET needs a non-empty URI even on UDS;
    /// the default <c>http://localhost</c> is overridden by the connect
    /// callback. Override only when stitching multiple sidecars in a test.
    /// </param>
    /// <param name="loggerFactory">Optional logger factory.</param>
    /// <param name="maxReceiveMessageSize">
    /// Tighten the default gRPC receive cap. Decisions and handshakes are
    /// small (under 16 KiB); we cap at 1 MiB to bound memory if the sidecar
    /// returns an oversized payload.
    /// </param>
    public static GrpcChannel Create(
        string socketPath,
        string address = "http://localhost",
        Microsoft.Extensions.Logging.ILoggerFactory? loggerFactory = null,
        int maxReceiveMessageSize = 1 * 1024 * 1024)
    {
        if (string.IsNullOrWhiteSpace(socketPath))
        {
            throw new ArgumentException("Socket path must be non-empty.", nameof(socketPath));
        }

        var connectionFactory = new UnixDomainSocketConnectionFactory(socketPath);
        var socketsHttpHandler = new SocketsHttpHandler
        {
            ConnectCallback = connectionFactory.ConnectAsync,
            // Reuse one inner connection; gRPC multiplexes over HTTP/2.
            PooledConnectionIdleTimeout = TimeSpan.FromMinutes(5),
        };

        var options = new GrpcChannelOptions
        {
            HttpHandler = socketsHttpHandler,
            DisposeHttpClient = true,
            MaxReceiveMessageSize = maxReceiveMessageSize,
            // The UDS auth model gives us peer credentials, not TLS — so
            // we ride plain HTTP/2 over the socket (per adapter.proto top
            // comment: "No mTLS over UDS").
            Credentials = Grpc.Core.ChannelCredentials.Insecure,
            LoggerFactory = loggerFactory,
        };

        return GrpcChannel.ForAddress(address, options);
    }

    /// <summary>
    /// Connect-callback helper that wraps a <see cref="Socket"/> in a Stream
    /// usable by <see cref="SocketsHttpHandler"/>.
    /// </summary>
    internal sealed class UnixDomainSocketConnectionFactory
    {
        private readonly string _socketPath;

        public UnixDomainSocketConnectionFactory(string socketPath)
        {
            _socketPath = socketPath;
        }

        public async ValueTask<Stream> ConnectAsync(
            SocketsHttpConnectionContext _,
            CancellationToken cancellationToken)
        {
            var socket = new Socket(
                AddressFamily.Unix,
                SocketType.Stream,
                ProtocolType.Unspecified);

            try
            {
                var endpoint = new UnixDomainSocketEndPoint(_socketPath);
                await socket.ConnectAsync(endpoint, cancellationToken).ConfigureAwait(false);
                return new NetworkStream(socket, ownsSocket: true);
            }
            catch
            {
                socket.Dispose();
                throw;
            }
        }
    }
}
