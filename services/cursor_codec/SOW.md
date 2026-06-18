# Statement of Work — SpendGuard Cursor MITM Codec

> **STATUS: EXPERIMENTAL — SOW only.**
>
> # DO NOT SHIP AS A GA FEATURE.
>
> This document is the customer-facing Statement of Work (SOW)
> addendum for the SpendGuard Cursor IDE MITM codec. It is the third
> of the three loud experimental markers required by
> [`design.md`](../../docs/specs/coverage/D17_cursor_mitm/design.md)
> §6.
>
> The codec reverse-engineers the Cursor IDE Agent's outbound wire
> protocol against `api.cursor.sh`. It WILL break whenever Cursor
> changes their wire protocol. Customer signs this SOW addendum to
> acknowledge break-window risk, legal posture, and operator threat
> model. The codec is **off by default**; opt-in requires the workspace
> feature flag `cursor-mitm-experimental` plus the customer's SOW
> credential bundle.

<!--
noindex: true
robots: noindex,nofollow
-->

## Status

* **Document version:** SOW Draft v1 (D17 SLICE 9)
* **Status:** EXPERIMENTAL — SOW only
* **Companion docs:**
  [`design.md`](../../docs/specs/coverage/D17_cursor_mitm/design.md),
  [`review-standards.md`](../../docs/specs/coverage/D17_cursor_mitm/review-standards.md),
  [`PROTOCOL.md`](PROTOCOL.md), [`RECON.md`](RECON.md),
  [`README.md`](README.md).
* **Generated from:** this file is the source of truth. There is **no
  separate template** — the customer signs a copy of this file with
  the Break-Window SLA section filled in.

## 1. Scope

The SOW authorises SpendGuard to provide the Customer with a
build of the SpendGuard egress proxy that includes the experimental
Cursor IDE MITM codec under the workspace feature flag
`cursor-mitm-experimental`. The build:

1. Replaces the default egress-proxy pass-through for the
   `api.cursor.sh` host with a per-request budget reservation +
   release-on-error pipeline as described in
   [`design.md`](../../docs/specs/coverage/D17_cursor_mitm/design.md)
   §5.
2. Surfaces a Cursor session's per-call cost projection to the
   SpendGuard ledger so the Customer's existing budget rules
   apply unchanged.
3. Emits the same KMS-signed audit chain (decision + outcome) the
   Customer receives for the Customer's other adapter integrations.

Out of scope:

* Real-time chat-message intercept on Cursor IDE non-Agent surfaces
  (Tab autocomplete, Cmd-K inline edits).
* Cursor authentication / login flow changes.
* New CA roots — the existing D02 trust store covers `api.cursor.sh`
  once the leaf cert SAN is extended for the SOW deployment.

## 2. Break-Window SLA

This section is filled in at SOW signature time. Reviewer / customer-
success rejects any signed SOW that leaves these fields blank.

| Field | Value |
|-------|-------|
| **Customer legal entity** | _(fill in at signature)_ |
| **SOW number** | _(fill in at signature)_ |
| **Codec-break detection window** | _(typically: ≤24 hours after Cursor release)_ |
| **Codec-fix turnaround target** | _(typically: ≤5 business days)_ |
| **Customer escalation contact** | _(operator name + on-call channel)_ |
| **SpendGuard escalation contact** | _(SpendGuard customer-success on-call)_ |
| **Codec build version** | _(commit SHA of the SpendGuard tree at delivery)_ |
| **Cursor client version range tested** | _(min/max Cursor IDE version at delivery)_ |

**Break detection.** The Customer agrees to deploy the SpendGuard
egress proxy with the `cursor-mitm-experimental` feature in a mode
that surfaces decode-error metrics to the Customer's on-call channel.
A SpendGuard-supplied PrometheusRule alerts when the percentage of
Cursor sessions failing envelope decode exceeds the agreed-upon
threshold over a rolling 1-hour window; the alert routes to the
Customer's escalation contact.

**Fix turnaround.** Upon codec-break alert, SpendGuard re-captures
the latest Cursor wire bytes (per the customer's on-host capture
workflow), refreshes the protobuf description under
[`src/proto/cursor.proto`](src/proto/cursor.proto) + the corpus
under [`fixtures/`](fixtures/), republishes a feature-flagged build,
and notifies the Customer. The Customer is responsible for redeploying
the refreshed build within their own change-management window.

## 3. Sealed-secret credential model

The Customer's Cursor session bearer tokens are end-to-end opaque to
the SpendGuard codec. They flow through unmodified in the
Connect-RPC Authorization header from the Cursor IDE binary to
`api.cursor.sh`; the codec does NOT decrypt, log, or persist them.

* **Authentication forwarding.** The egress proxy preserves the
  full request header set on the upstream forward; only the Host
  header is replaced (per the SAN-extension leaf cert).
* **Credential storage.** The Customer's CA root used to terminate
  TLS in front of the SpendGuard egress proxy is sealed in the
  Customer's existing secret-manager backing the D02 trust store.
  No new secret material is introduced by D17.
* **Audit redaction.** Per
  [POST_GA_03](../../docs/internal/slices/POST_GA_03_tokenizer_runtime_hardening.md)
  the SpendGuard sidecar redacts message-body fields beyond
  `model` / `max_tokens` / `temperature` in audit events when the
  tenant has the redaction policy bound. Cursor traffic is no
  exception.

The codec carries NO long-lived secrets beyond what the D02 leaf
cert already requires. The codec build itself is unsigned (it ships
as part of the SOW build); the artifact integrity gate is the
SpendGuard delivery pipeline's existing cosign-on-release signature.

## 4. Operator threat model

This section enumerates the threats the operator MUST defend against,
and the codec-internal mitigations the Customer should NOT rely on as
a sole control.

| Threat | Mitigation | Customer responsibility |
|--------|------------|-------------------------|
| Cursor wire-protocol drift breaks decode | Version-gate check at request entry; release-and-pass-through on unknown wire shape per `W6`/`W7` | Maintain the SpendGuard escalation contact + redeployment workflow |
| Operator misconfigures the SAN extension | Default `sites.toml` does NOT list `api.cursor.sh`; opt-in is explicit per `F4` | Customer-side gate review before flipping the flag |
| Codec decode error leaks user prompt to logs | Decode errors log at `INFO` with envelope-level metadata only; payload bodies redacted (POST_GA_03) | Customer's log-pipeline policy MUST preserve the redaction |
| Cursor IDE binary tampered with on host | Out of scope for the codec; host-integrity is the Customer's MDM responsibility | Customer's existing endpoint-management posture |
| Cursor MITM detection fires on customer's deployment | The leaf cert is Customer-signed (D02); Cursor's own certificate pinning does NOT detect this provided the Customer's CA is trusted by the Cursor binary on each host | Confirm Cursor terms permit on-host trusted CA injection |
| SpendGuard decode mis-translates a Cursor field | `release-and-pass-through` on translator errors per `W6`; reservation released, upstream call proceeds with no budget gate that call | SpendGuard owns codec fix; Customer accepts a temporary gating gap |
| Adversary modifies a request mid-flight | TLS terminates at the SpendGuard egress proxy; same control path the customer's other adapters share | Customer's existing in-cluster trust model |

The codec is **best-effort gating**, not a hard policy gate. When the
codec cannot understand the wire shape (because Cursor changed it),
the egress proxy releases the held reservation and lets the call
pass through. This is the same posture the codec uses for unknown
Cursor model strings (`W6`). Failures are loud (decode-error metric
+ stderr banner + structured log) but they do NOT block the
Customer's Cursor session.

## 5. Legal posture

The Customer agrees that:

1. **Reverse-engineered interoperability.** The codec is SpendGuard's
   own observation of the Cursor IDE Agent's outbound wire format.
   No vendor source is included.
   [`PROTOCOL.md`](PROTOCOL.md) §5 documents field-by-field hex
   evidence for the synthetic corpus; real-capture evidence lives in
   Customer-side artifacts.
2. **Customer's Cursor terms responsibility.** The Customer is solely
   responsible for confirming that the Customer's organisation's
   Cursor IDE terms of service permit on-host MITM of outbound
   traffic.
   SpendGuard's standard SOW deliverable assumes the Customer has
   already obtained internal legal sign-off; SpendGuard does NOT
   review Cursor's terms on the Customer's behalf.
3. **No vendor endorsement.** Nothing in the codec, this SOW, or the
   SpendGuard egress-proxy build represents endorsement by Cursor of
   the SpendGuard product.
4. **No security disclosure asymmetry.** SpendGuard's published codec
   bug-fix changelog is the Customer's authoritative record of how
   the codec was changed. The Customer is welcome to disclose this
   to Cursor at the Customer's discretion; SpendGuard does NOT
   pre-negotiate a coordinated disclosure on the Customer's behalf.

## 6. Demo + acceptance gates

The Customer can validate a candidate SpendGuard codec build against
the synthetic fixture corpus before deploying it. The replay path is:

```sh
make demo-up DEMO_MODE=cursor_mitm_fixture
```

The demo:

* Runs the `cursor_mitm_fixture` driver against the SLICE 6 MITM
  session state machine using the synthetic corpus shipped under
  [`fixtures/synthetic/`](fixtures/synthetic/).
* Asserts:
  * Reserve-per-request (≥4 reserves across the 4 SLICE 8 fixtures).
  * Commit-on-success / no-commit-on-error.
  * Byte-for-byte preservation per `W5`.
* Verifies the ledger DB rows landed via
  [`deploy/demo/verify_step_cursor_mitm_fixture.sql`](../../deploy/demo/verify_step_cursor_mitm_fixture.sql).

The demo is **NOT** a real Cursor binary exercise — per the legal
posture in §5 we do not boot the Cursor IDE in CI. The fixture corpus
is the substitute. The Customer's deployment exercises live Cursor
traffic against the same codec.

## 7. Revocation

The Customer may revoke the SOW by:

1. Removing `cursor-mitm-experimental` from the SpendGuard egress
   proxy build configuration.
2. Removing `api.cursor.sh` from
   [`services/egress_proxy/config/sites.toml`](../egress_proxy/config/sites.toml).
3. Removing the SAN extension on the D02 leaf cert.

Once revoked, the codec is fully inert; all Cursor traffic falls
through to the default egress-proxy pass-through (which is also
off by default per `F4`). No data persists in the SpendGuard ledger
beyond the audit rows that were already written.

## 8. Signatures

By signing below, the Customer acknowledges the entire scope, the
Break-Window SLA, the credential model, the operator threat model,
and the legal posture documented above.

```
Customer
  Name:    ____________________________
  Title:   ____________________________
  Date:    ____________________________
  Signature: __________________________

SpendGuard
  Name:    ____________________________
  Title:   ____________________________
  Date:    ____________________________
  Signature: __________________________
```
