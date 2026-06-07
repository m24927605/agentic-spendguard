// SPDX-License-Identifier: Apache-2.0
// Copyright (c) SpendGuard Authors.

using System;
using System.Collections.Generic;
using System.Runtime.CompilerServices;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.Extensions.AI;

namespace Spendguard.AgentFramework.Tests.Helpers;

/// <summary>
/// Test double for the inner <see cref="IChatClient"/> the middleware wraps.
/// Counts invocations and lets the test set the canned response (including
/// usage tokens).
/// </summary>
public sealed class FakeChatClient : IChatClient
{
    private readonly ChatResponse _response;

    /// <summary>Number of times <see cref="GetResponseAsync"/> was invoked.</summary>
    public int InvocationCount;

    /// <summary>Last <see cref="ChatOptions"/> passed to the inner client.</summary>
    public ChatOptions? LastOptions;

    /// <summary>If non-null, the inner client throws this exception instead of returning.</summary>
    public Exception? ThrowOnNext;

    /// <summary>Constructs a fake that returns the supplied response.</summary>
    public FakeChatClient(ChatResponse response)
    {
        _response = response ?? throw new ArgumentNullException(nameof(response));
    }

    /// <summary>Convenience factory wrapping a response with a usage block.</summary>
    public static FakeChatClient WithUsage(int inputTokens, int outputTokens, string text = "ok")
    {
        var response = new ChatResponse(new ChatMessage(ChatRole.Assistant, text))
        {
            Usage = new UsageDetails
            {
                InputTokenCount = inputTokens,
                OutputTokenCount = outputTokens,
                TotalTokenCount = inputTokens + outputTokens,
            },
        };
        return new FakeChatClient(response);
    }

    /// <inheritdoc/>
    public Task<ChatResponse> GetResponseAsync(
        IEnumerable<ChatMessage> messages,
        ChatOptions? options = null,
        CancellationToken cancellationToken = default)
    {
        Interlocked.Increment(ref InvocationCount);
        LastOptions = options;
        if (ThrowOnNext is not null)
        {
            throw ThrowOnNext;
        }
        return Task.FromResult(_response);
    }

    /// <inheritdoc/>
    public async IAsyncEnumerable<ChatResponseUpdate> GetStreamingResponseAsync(
        IEnumerable<ChatMessage> messages,
        ChatOptions? options = null,
        [EnumeratorCancellation] CancellationToken cancellationToken = default)
    {
        Interlocked.Increment(ref InvocationCount);
        LastOptions = options;
        var update = new ChatResponseUpdate
        {
            Contents = { new TextContent(_response.Text) },
        };
        yield return update;
        await Task.CompletedTask;
    }

    /// <inheritdoc/>
    public object? GetService(Type serviceType, object? serviceKey = null) => null;

    /// <inheritdoc/>
    public void Dispose() { }
}
