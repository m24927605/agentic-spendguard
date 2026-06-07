"use strict";
// `n8n-nodes-spendguard` — public package barrel.
//
// The n8n loader reads `package.json` `n8n.nodes[]` + `n8n.credentials[]`
// to discover the runtime classes; this barrel exists primarily so the
// package's `main` (CJS-only per design.md §6.6) loads cleanly when
// vendored as a dependency. Re-exports the helper modules for tests and
// downstream tooling.
Object.defineProperty(exports, "__esModule", { value: true });
exports.VERSION = exports.mapToNodeApiError = exports.acquireClient = exports.resolveRunIdentity = void 0;
var runIdentity_1 = require("./runIdentity");
Object.defineProperty(exports, "resolveRunIdentity", { enumerable: true, get: function () { return runIdentity_1.resolveRunIdentity; } });
var clientPool_1 = require("./clientPool");
Object.defineProperty(exports, "acquireClient", { enumerable: true, get: function () { return clientPool_1.acquireClient; } });
var errors_1 = require("./errors");
Object.defineProperty(exports, "mapToNodeApiError", { enumerable: true, get: function () { return errors_1.mapToNodeApiError; } });
var version_1 = require("./version");
Object.defineProperty(exports, "VERSION", { enumerable: true, get: function () { return version_1.VERSION; } });
