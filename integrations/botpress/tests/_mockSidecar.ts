// In-process HTTP mock for the D09 SLICE 1 sidecar companion (`/v1/decision`
// + `/v1/trace`). Used by the unit tier for wire-shape parity without msw
// (msw is heavyweight + adds an ESM/CJS interop wrinkle in vitest 2.x — a
// bare Node `http.createServer` keeps the harness self-contained and works
// in both the unit and integration tiers).
//
// review-standards.md §3.14 / §4.3 / B03 / I02 / I04 — the mock records a
// timestamped event log so callers can assert strict-ordering between the
// reserve POST and any subsequent trace / upstream POST.

import { type IncomingMessage, type Server, type ServerResponse, createServer } from "node:http";
import type { AddressInfo } from "node:net";

export type DecisionVerdict = "ALLOW" | "DENY" | "DEGRADE";

export interface MockDecisionRequest {
  tenant_id: string;
  claim_estimate_atomic: string;
  prompt_class: string;
  model_class: string;
  idempotency_key: string;
  budget_id: string;
  decision_context?: Record<string, string>;
}

export interface MockTraceRequest {
  reservation_id: string;
  outcome: "ACCEPTED" | "REJECTED";
  provider_event_id?: string;
  input_tokens?: number;
  output_tokens?: number;
  actual_amount_atomic?: string;
}

export interface MockEvent {
  readonly kind: "decision" | "trace" | "decision-error" | "trace-error";
  readonly path: string;
  readonly body: MockDecisionRequest | MockTraceRequest;
  readonly timestamp: number;
}

export interface MockSidecarOptions {
  /** Verdict to return for `/v1/decision`. Defaults to `ALLOW`. */
  verdict?: DecisionVerdict;
  /** Force `/v1/decision` to return HTTP 500. */
  failDecisionWith?: { status: number; body?: string };
  /** Force `/v1/trace` to return HTTP 500. */
  failTraceWith?: { status: number; body?: string };
  /** Refuse to even respond (close the socket). Simulates DEGRADE on
   *  transport failure. */
  refuseAll?: boolean;
  /** Returned reason codes on a DENY. */
  denyReasonCodes?: string[];
}

export interface MockSidecarHandle {
  readonly url: string;
  readonly events: MockEvent[];
  readonly hits: { decision: number; trace: number };
  setVerdict(v: DecisionVerdict): void;
  setOptions(opts: MockSidecarOptions): void;
  reset(): void;
  close(): Promise<void>;
}

let counter = 0;

export async function setupMockSidecar(
  initial: MockSidecarOptions = {},
): Promise<MockSidecarHandle> {
  let options: MockSidecarOptions = { verdict: "ALLOW", ...initial };
  const events: MockEvent[] = [];
  const hits = { decision: 0, trace: 0 };

  const handler = (req: IncomingMessage, res: ServerResponse): void => {
    if (options.refuseAll === true) {
      req.socket.destroy();
      return;
    }
    let body = "";
    req.on("data", (chunk) => {
      body += chunk;
    });
    req.on("end", () => {
      const ts = performance.now();
      let parsed: unknown;
      try {
        parsed = JSON.parse(body);
      } catch {
        parsed = {};
      }
      if (req.url?.startsWith("/v1/decision")) {
        hits.decision += 1;
        if (options.failDecisionWith !== undefined) {
          events.push({
            kind: "decision-error",
            path: req.url ?? "",
            body: parsed as MockDecisionRequest,
            timestamp: ts,
          });
          res.statusCode = options.failDecisionWith.status;
          res.setHeader("content-type", "application/json");
          res.end(options.failDecisionWith.body ?? '{"error":"sidecar_failure"}');
          return;
        }
        events.push({
          kind: "decision",
          path: req.url,
          body: parsed as MockDecisionRequest,
          timestamp: ts,
        });
        const reservation = `res-${counter++}`;
        const decision = `dec-${counter++}`;
        const verdict = options.verdict ?? "ALLOW";
        res.statusCode = 200;
        res.setHeader("content-type", "application/json");
        res.end(
          JSON.stringify({
            verdict,
            reservation_id: verdict === "ALLOW" ? reservation : "",
            decision_id: decision,
            reason_codes:
              options.denyReasonCodes ?? (verdict === "DENY" ? ["BUDGET_EXCEEDED"] : []),
          }),
        );
        return;
      }
      if (req.url?.startsWith("/v1/trace")) {
        hits.trace += 1;
        if (options.failTraceWith !== undefined) {
          events.push({
            kind: "trace-error",
            path: req.url ?? "",
            body: parsed as MockTraceRequest,
            timestamp: ts,
          });
          res.statusCode = options.failTraceWith.status;
          res.setHeader("content-type", "application/json");
          res.end(options.failTraceWith.body ?? '{"error":"trace_failure"}');
          return;
        }
        events.push({
          kind: "trace",
          path: req.url,
          body: parsed as MockTraceRequest,
          timestamp: ts,
        });
        res.statusCode = 200;
        res.setHeader("content-type", "application/json");
        res.end(
          JSON.stringify({
            verdict:
              parsed && (parsed as MockTraceRequest).outcome === "ACCEPTED"
                ? "ACCEPTED"
                : "REJECTED",
            ledger_transaction_id: `tx-${counter++}`,
          }),
        );
        return;
      }
      res.statusCode = 404;
      res.end();
    });
  };

  const server: Server = createServer(handler);
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", () => resolve()));
  const addr = server.address() as AddressInfo;
  const url = `http://127.0.0.1:${addr.port}`;

  return {
    url,
    events,
    hits,
    setVerdict(v) {
      options.verdict = v;
    },
    setOptions(o) {
      options = { ...options, ...o };
    },
    reset() {
      events.length = 0;
      hits.decision = 0;
      hits.trace = 0;
      options = { verdict: "ALLOW" };
    },
    close() {
      return new Promise<void>((resolve) => server.close(() => resolve()));
    },
  };
}
