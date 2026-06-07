// `clientPool` — process-wide singleton cache of SpendGuardClient
// instances, keyed by `(tenantId, socketPath)`.
//
// LOCKED per design.md §5 / review-standards.md §5:
//   - One client per credential per process; survives across executions.
//   - Bounded at 16 entries with FIFO eviction. Evicted clients have
//     `close()` called so handshake-side resources drain.
//   - Concurrent first-call requests share a single in-flight Promise so
//     duplicate `connect` + `handshake` round-trips never fire.
//   - Failed handshake → pool entry is deleted; the next call retries
//     from scratch (no permanent poisoning).
//   - `beforeExit` registered ONCE at module load, closes everything.
//
// `runtimeKind` is intentionally NOT in the key — review-standards §5.9 —
// because the same n8n install commonly attributes to different telemetry
// kinds without wanting to multiply client connections.

import { createHash } from "node:crypto";
import { SpendGuardClient, type SpendGuardClientOptions } from "@spendguard/sdk";

export const MAX_POOL_ENTRIES = 16;

interface PoolEntry {
  promise: Promise<SpendGuardClient>;
  insertedAt: number;
}

const POOL = new Map<string, PoolEntry>();
let exitHandlerInstalled = false;

/** Stable key from credential fields the substrate cares about. */
export function key(creds: Record<string, unknown>): string {
  const h = createHash("sha256");
  h.update(String(creds.tenantId ?? ""));
  h.update("|");
  h.update(String(creds.socketPath ?? ""));
  return h.digest("hex").slice(0, 16);
}

/** Test-only: clear the pool. NOT exported via barrel. */
export function _resetPoolForTests(): void {
  for (const [, entry] of POOL) {
    entry.promise.then((c) => c.close?.()).catch(() => {});
  }
  POOL.clear();
}

/**
 * Constructor injection point so tests can stub the SpendGuardClient
 * without monkey-patching the SDK package. Production code never sets
 * this — the default builds a real client via `new SpendGuardClient(...)`.
 */
type ClientFactory = (
  options: SpendGuardClientOptions,
) => SpendGuardClient | Promise<SpendGuardClient>;

let clientFactory: ClientFactory = (options) => new SpendGuardClient(options);

/** Test-only setter for the factory; resets on `_resetPoolForTests`. */
export function _setClientFactoryForTests(factory: ClientFactory | null): void {
  clientFactory = factory ?? ((options) => new SpendGuardClient(options));
}

function installBeforeExitHandlerOnce(): void {
  if (exitHandlerInstalled) return;
  exitHandlerInstalled = true;
  process.on("beforeExit", () => {
    for (const [, entry] of POOL) {
      entry.promise.then((c) => c.close?.()).catch(() => {});
    }
  });
}

/**
 * Acquire (or create) a `SpendGuardClient` for the given credential.
 * Concurrent callers for the same credential observe a single in-flight
 * Promise.
 */
export async function acquireClient(creds: Record<string, unknown>): Promise<SpendGuardClient> {
  installBeforeExitHandlerOnce();
  const k = key(creds);
  const existing = POOL.get(k);
  if (existing !== undefined) {
    return existing.promise;
  }

  if (POOL.size >= MAX_POOL_ENTRIES) {
    // FIFO eviction — oldest entry leaves first.
    let oldestKey: string | undefined;
    let oldestEntry: PoolEntry | undefined;
    for (const [candKey, candEntry] of POOL) {
      if (oldestEntry === undefined || candEntry.insertedAt < oldestEntry.insertedAt) {
        oldestKey = candKey;
        oldestEntry = candEntry;
      }
    }
    if (oldestKey !== undefined && oldestEntry !== undefined) {
      POOL.delete(oldestKey);
      oldestEntry.promise.then((c) => c.close?.()).catch(() => {});
    }
  }

  const promise = (async () => {
    const built = await clientFactory({
      socketPath: String(creds.socketPath),
      tenantId: String(creds.tenantId),
      runtimeKind: String(creds.runtimeKind ?? "n8n"),
    } as SpendGuardClientOptions);
    await built.connect();
    await built.handshake();
    return built;
  })();

  const entry: PoolEntry = { promise, insertedAt: Date.now() };
  POOL.set(k, entry);
  promise.catch(() => {
    // Failed handshake — purge so the next call retries from scratch.
    POOL.delete(k);
  });
  return promise;
}

/** Test-only inspection. */
export function _poolSizeForTests(): number {
  return POOL.size;
}
