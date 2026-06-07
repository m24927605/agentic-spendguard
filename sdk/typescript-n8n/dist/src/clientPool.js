"use strict";
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
Object.defineProperty(exports, "__esModule", { value: true });
exports.MAX_POOL_ENTRIES = void 0;
exports.key = key;
exports._resetPoolForTests = _resetPoolForTests;
exports._setClientFactoryForTests = _setClientFactoryForTests;
exports.acquireClient = acquireClient;
exports._poolSizeForTests = _poolSizeForTests;
const node_crypto_1 = require("node:crypto");
const sdk_1 = require("@spendguard/sdk");
exports.MAX_POOL_ENTRIES = 16;
const POOL = new Map();
let exitHandlerInstalled = false;
/** Stable key from credential fields the substrate cares about. */
function key(creds) {
    const h = (0, node_crypto_1.createHash)("sha256");
    h.update(String(creds.tenantId ?? ""));
    h.update("|");
    h.update(String(creds.socketPath ?? ""));
    return h.digest("hex").slice(0, 16);
}
/** Test-only: clear the pool. NOT exported via barrel. */
function _resetPoolForTests() {
    for (const [, entry] of POOL) {
        entry.promise.then((c) => c.close?.()).catch(() => { });
    }
    POOL.clear();
}
let clientFactory = (options) => new sdk_1.SpendGuardClient(options);
/** Test-only setter for the factory; resets on `_resetPoolForTests`. */
function _setClientFactoryForTests(factory) {
    clientFactory = factory ?? ((options) => new sdk_1.SpendGuardClient(options));
}
function installBeforeExitHandlerOnce() {
    if (exitHandlerInstalled)
        return;
    exitHandlerInstalled = true;
    process.on("beforeExit", () => {
        for (const [, entry] of POOL) {
            entry.promise.then((c) => c.close?.()).catch(() => { });
        }
    });
}
/**
 * Acquire (or create) a `SpendGuardClient` for the given credential.
 * Concurrent callers for the same credential observe a single in-flight
 * Promise.
 */
async function acquireClient(creds) {
    installBeforeExitHandlerOnce();
    const k = key(creds);
    const existing = POOL.get(k);
    if (existing !== undefined) {
        return existing.promise;
    }
    if (POOL.size >= exports.MAX_POOL_ENTRIES) {
        // FIFO eviction — oldest entry leaves first.
        let oldestKey;
        let oldestEntry;
        for (const [candKey, candEntry] of POOL) {
            if (oldestEntry === undefined || candEntry.insertedAt < oldestEntry.insertedAt) {
                oldestKey = candKey;
                oldestEntry = candEntry;
            }
        }
        if (oldestKey !== undefined && oldestEntry !== undefined) {
            POOL.delete(oldestKey);
            oldestEntry.promise.then((c) => c.close?.()).catch(() => { });
        }
    }
    const promise = (async () => {
        const built = await clientFactory({
            socketPath: String(creds.socketPath),
            tenantId: String(creds.tenantId),
            runtimeKind: String(creds.runtimeKind ?? "n8n"),
        });
        await built.connect();
        await built.handshake();
        return built;
    })();
    const entry = { promise, insertedAt: Date.now() };
    POOL.set(k, entry);
    promise.catch(() => {
        // Failed handshake — purge so the next call retries from scratch.
        POOL.delete(k);
    });
    return promise;
}
/** Test-only inspection. */
function _poolSizeForTests() {
    return POOL.size;
}
