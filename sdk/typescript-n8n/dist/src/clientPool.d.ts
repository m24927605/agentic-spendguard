import { SpendGuardClient, type SpendGuardClientOptions } from "@spendguard/sdk";
export declare const MAX_POOL_ENTRIES = 16;
/** Stable key from credential fields the substrate cares about. */
export declare function key(creds: Record<string, unknown>): string;
/** Test-only: clear the pool. NOT exported via barrel. */
export declare function _resetPoolForTests(): void;
/**
 * Constructor injection point so tests can stub the SpendGuardClient
 * without monkey-patching the SDK package. Production code never sets
 * this — the default builds a real client via `new SpendGuardClient(...)`.
 */
type ClientFactory = (options: SpendGuardClientOptions) => SpendGuardClient | Promise<SpendGuardClient>;
/** Test-only setter for the factory; resets on `_resetPoolForTests`. */
export declare function _setClientFactoryForTests(factory: ClientFactory | null): void;
/**
 * Acquire (or create) a `SpendGuardClient` for the given credential.
 * Concurrent callers for the same credential observe a single in-flight
 * Promise.
 */
export declare function acquireClient(creds: Record<string, unknown>): Promise<SpendGuardClient>;
/** Test-only inspection. */
export declare function _poolSizeForTests(): number;
export {};
