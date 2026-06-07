// Module-level SpendGuardClient cache keyed by `(tenantId, sidecarUds)`.
//
// Locked at design.md §5: re-running `init()` per Flowise invocation
// MUST NOT re-open the UDS — the Flowise runtime instantiates an INode
// per chatflow execution, so without this cache every chat call would
// pay the gRPC handshake cost.
//
// The cache is intentionally process-global. Flowise runs a single Node
// process per server, so collision between tenants is impossible (the
// composite key ensures distinct entries) and lifecycle teardown is
// handled by process exit.

/**
 * Structural shape of the SpendGuardClient the cache returns. The real
 * type lives in `@spendguard/sdk` (a peerDep) and exposes more methods;
 * we only need `connect()` + `handshake()` here, and downstream consumers
 * (the D04 handler) cast to the full type.
 */
export interface CachedClient {
  // biome-ignore lint/suspicious/noExplicitAny: peer-dep type is opaque
  readonly raw: any;
}

interface CacheKey {
  sidecarUds: string;
  tenantId: string;
}

interface CacheEntry {
  client: CachedClient;
  /** Module path used to dynamically import the SDK; isolated for testability. */
  sdkModuleSpecifier: string;
}

const cache = new Map<string, CacheEntry>();

function keyOf(k: CacheKey): string {
  return `${k.tenantId}::${k.sidecarUds}`;
}

/**
 * Lazy SDK import — keeps `@spendguard/sdk` strictly a peerDep at the
 * package boundary and lets the unit tests inject a mock factory.
 */
let factoryOverride: ((k: CacheKey) => Promise<CachedClient>) | undefined;

/**
 * Test-only hook — install a mock factory in place of the dynamic SDK
 * import. Cleared by `_resetCacheForTests`.
 */
export function _setClientFactoryForTests(
  factory: ((k: CacheKey) => Promise<CachedClient>) | undefined,
): void {
  factoryOverride = factory;
}

async function defaultFactory(k: CacheKey): Promise<CachedClient> {
  // Dynamic import keeps the peer-dep contract honest: the package
  // doesn't `require('@spendguard/sdk')` until `getOrCreateClient` is
  // actually called, so a Flowise installation that never lands a
  // wrapper node won't fail to load even without the peer installed.
  const sdkModule = (await import("@spendguard/sdk")) as {
    SpendGuardClient: new (opts: {
      socketPath: string;
      tenantId: string;
      runtimeKind: string;
      runtimeVersion: string;
    }) => {
      connect: () => Promise<void>;
      handshake: () => Promise<void>;
    };
  };
  const client = new sdkModule.SpendGuardClient({
    socketPath: k.sidecarUds,
    tenantId: k.tenantId,
    runtimeKind: "flowise",
    runtimeVersion: "0.1.0",
  });
  await client.connect();
  await client.handshake();
  return { raw: client };
}

export async function getOrCreateClient(k: CacheKey): Promise<CachedClient> {
  const key = keyOf(k);
  const existing = cache.get(key);
  if (existing) return existing.client;

  const factory = factoryOverride ?? defaultFactory;
  const client = await factory(k);
  cache.set(key, { client, sdkModuleSpecifier: "@spendguard/sdk" });
  return client;
}

/** Test-only: clear the cache between cases. */
export function _resetCacheForTests(): void {
  cache.clear();
  factoryOverride = undefined;
}

/** Inspector for tests — number of cached entries. */
export function _cacheSize(): number {
  return cache.size;
}
