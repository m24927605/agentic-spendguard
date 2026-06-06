// Minimal mock sidecar UDS server for SLICE 3 lifecycle tests.
//
// SLICE 3 only verifies the connect → close lifecycle and `Symbol.asyncDispose`
// semantics — there is no need to implement the full SidecarAdapter service
// here. We bind a real `@grpc/grpc-js` server to a UDS path and let the client
// connect, then verify the channel is created and torn down cleanly.
//
// SLICE 9 ships the full mock with handshake / reserve / commit / release
// behaviors per `tests.md` §4.2.

import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { Server, ServerCredentials } from "@grpc/grpc-js";

/**
 * Mock UDS sidecar. Binds an empty gRPC server to a tempfile socket; the
 * server accepts the client's connection attempts but does NOT register any
 * services. SLICE 3 tests check only the channel-open + close path; SLICE 4+
 * extends this mock with handshake + RPC handlers.
 */
export class MockSidecar {
  /** The UDS path the server is bound to. Stable across the mock's lifetime. */
  readonly socketPath: string;
  private readonly server: Server;
  private readonly socketDir: string;
  private bound = false;

  private constructor(socketPath: string, socketDir: string) {
    this.socketPath = socketPath;
    this.socketDir = socketDir;
    this.server = new Server();
  }

  /**
   * Start a fresh mock instance on a random UDS path under the system tempdir.
   * Each test should `await using mock = await MockSidecar.start()` so the
   * cleanup runs on scope exit.
   */
  static async start(): Promise<MockSidecar> {
    const dir = mkdtempSync(join(tmpdir(), "spendguard-mock-"));
    const path = join(dir, "adapter.sock");
    const mock = new MockSidecar(path, dir);
    await mock.bind();
    return mock;
  }

  private async bind(): Promise<void> {
    return new Promise<void>((resolve, reject) => {
      // `unix:` prefix is the @grpc/grpc-js convention for UDS bind targets.
      this.server.bindAsync(
        `unix:${this.socketPath}`,
        ServerCredentials.createInsecure(),
        (err) => {
          if (err) {
            reject(err);
            return;
          }
          this.bound = true;
          resolve();
        },
      );
    });
  }

  /** Whether the server is currently bound. */
  get isBound(): boolean {
    return this.bound;
  }

  /**
   * Stop the server and clean up the temp socket / dir. Idempotent.
   *
   * Implements `[Symbol.asyncDispose]` so callers can write
   * `await using mock = await MockSidecar.start()` and rely on cleanup.
   */
  async close(): Promise<void> {
    if (this.bound) {
      await new Promise<void>((resolve) => {
        // `tryShutdown` waits for in-flight RPCs; for SLICE 3 there are none,
        // but we use it anyway to match the production graceful-close path.
        this.server.tryShutdown((err) => {
          if (err) {
            // forceShutdown ensures the test doesn't hang if shutdown wedges.
            this.server.forceShutdown();
          }
          resolve();
        });
      });
      this.bound = false;
    }
    // Best-effort socket file cleanup. On macOS the kernel may have already
    // removed it once the server closed the listening fd; on Linux we have to
    // unlink it ourselves.
    try {
      if (existsSync(this.socketPath)) {
        rmSync(this.socketPath, { force: true });
      }
      rmSync(this.socketDir, { recursive: true, force: true });
    } catch {
      // ignore — cleanup is best-effort.
    }
  }

  async [Symbol.asyncDispose](): Promise<void> {
    await this.close();
  }
}
