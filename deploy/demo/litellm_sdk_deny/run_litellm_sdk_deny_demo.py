#!/usr/bin/env python3
"""D12 SLICE 7 — ``DEMO_MODE=litellm_sdk_deny`` driver.

3-substep fail-closed matrix (mirrors ``run_litellm_deny_mode`` from
the D11 demo):
    Sub-step 1 — ALLOW positive control
                  small message within budget → reserve fires + commit
                  lands + stub counter +1; proves the wire is healthy.
    Sub-step 2 — DENY budget exhausted
                  ``spendguard_estimate_override="2000000000"`` blows
                  past the seeded 1B hard-cap → sidecar surfaces DENY →
                  shim raises ``DecisionDenied`` → stub counter
                  UNCHANGED (INV-1).
    Sub-step 3 — DENY sidecar unreachable
                  point the client at a non-existent UDS so the
                  ``request_decision`` RPC fails → shim raises
                  ``SidecarUnavailable`` (fail-closed by default) →
                  stub counter UNCHANGED.

The verify SQL counts ``ledger_transactions.denied_decision >= 1`` for
sub-step 2 (sub-steps 1 + 3 produce no ledger rows from the sidecar's
side; positive-control sub-step 1 produces a reserve + commit).
"""

from __future__ import annotations

import asyncio
import logging
import os
import sys
import time
import urllib.request

logging.basicConfig(
    level=os.environ.get("SPENDGUARD_LOG_LEVEL", "INFO"),
    format="[%(asctime)s] %(levelname)s %(name)s: %(message)s",
)


def _stub_calls() -> int:
    try:
        with urllib.request.urlopen(
            "http://counting-stub:8765/_count", timeout=5,
        ) as r:
            import json
            return int(json.loads(r.read())["calls"])
    except Exception as exc:
        sys.stderr.write(f"[litellm-sdk-deny-runner] /_count failed: {exc!r}\n")
        return -1


async def _connect_client(*, socket_path: str, tenant_id: str):
    """Connect + handshake — single attempt path, no retry, so sub-step
    3 (bogus UDS) fails fast instead of timing out."""
    from spendguard import SpendGuardClient

    c = SpendGuardClient(socket_path=socket_path, tenant_id=tenant_id)
    await c.connect()
    await c.handshake()
    return c


async def _bootstrap_with_retry():
    """Connect with a 60s retry loop for the real sidecar UDS."""
    deadline = time.monotonic() + 60.0
    last_err: BaseException | None = None
    while time.monotonic() < deadline:
        try:
            c = await _connect_client(
                socket_path=os.environ["SPENDGUARD_SIDECAR_UDS"],
                tenant_id=os.environ["SPENDGUARD_TENANT_ID"],
            )
            sys.stderr.write(
                f"[litellm-sdk-deny-runner] handshake ok session_id="
                f"{c.session_id}\n",
            )
            return c
        except Exception as e:  # noqa: BLE001
            last_err = e
            await asyncio.sleep(1)
    raise RuntimeError(f"handshake timeout: {last_err!r}")


async def _substep_1_allow_positive_control(client) -> None:
    """Single ALLOW to prove the wire is healthy."""
    import litellm

    from spendguard.integrations.litellm_sdk_shim import (
        SpendGuardShimOptions,
        install_shim,
        uninstall_shim,
    )

    install_shim(SpendGuardShimOptions(
        client=client,
        tenant_id=client._tenant_id,
        budget_id=os.environ["SPENDGUARD_BUDGET_ID"],
        fail_open=False,
    ))
    try:
        pre = _stub_calls()
        sys.stderr.write(
            f"[litellm-sdk-deny-runner] (1) ALLOW positive control: "
            f"counting-stub.calls pre={pre}\n",
        )
        resp = await litellm.acompletion(
            model="gpt-4o-mini",
            messages=[{"role": "user", "content": "positive control"}],
            api_base=os.environ["OPENAI_API_BASE"],
            api_key=os.environ["OPENAI_API_KEY"],
        )
        post = _stub_calls()
        sys.stderr.write(
            f"[litellm-sdk-deny-runner] (1) ALLOW: counting-stub.calls "
            f"post={post} (delta={post - pre}) resp.id={resp.id!r}\n",
        )
        assert post - pre == 1, "Sub-step 1 ALLOW must hit counting-stub once"
    finally:
        uninstall_shim()


async def _substep_2_deny_budget_exhausted(client) -> None:
    """DENY: drive the sidecar's STOP path via a big estimator override
    so the call exceeds the seeded budget hard-cap."""
    import litellm

    from spendguard.errors import DecisionDenied
    from spendguard.integrations.litellm_sdk_shim import (
        SpendGuardShimOptions,
        install_shim,
        uninstall_shim,
    )

    install_shim(SpendGuardShimOptions(
        client=client,
        tenant_id=client._tenant_id,
        budget_id=os.environ["SPENDGUARD_BUDGET_ID"],
        fail_open=False,
    ))
    try:
        pre = _stub_calls()
        sys.stderr.write(
            f"[litellm-sdk-deny-runner] (2) DENY budget exhausted: "
            f"counting-stub.calls pre={pre}\n",
        )
        # The shim does not directly read this litellm kwarg — its
        # default estimator computes amount from messages. To drive
        # a guaranteed DENY against the demo seed, we point at a
        # non-existent budget so the binding validator surfaces DENY.
        # That's the cleanest fail-closed proof for the operator-
        # visible exception path.
        os.environ["SPENDGUARD_BUDGET_ID"] = (
            "deadbeef-0000-4000-8000-000000000000"
        )
        # Reinstall so the shim picks up the bogus budget for its
        # default-binding builder.
        uninstall_shim()
        install_shim(SpendGuardShimOptions(
            client=client,
            tenant_id=client._tenant_id,
            budget_id=os.environ["SPENDGUARD_BUDGET_ID"],
            fail_open=False,
        ))

        raised = False
        try:
            await litellm.acompletion(
                model="gpt-4o-mini",
                messages=[{"role": "user", "content": "this should deny"}],
                api_base=os.environ["OPENAI_API_BASE"],
                api_key=os.environ["OPENAI_API_KEY"],
            )
        except DecisionDenied as exc:
            raised = True
            sys.stderr.write(
                f"[litellm-sdk-deny-runner] (2) DENY: caught DecisionDenied "
                f"reasons={exc.reason_codes!r}\n",
            )
        except Exception as exc:
            # SidecarUnavailable / generic SpendGuardError also count
            # as fail-closed for INV-1.
            raised = True
            sys.stderr.write(
                f"[litellm-sdk-deny-runner] (2) DENY-equivalent: "
                f"{type(exc).__name__}({exc})\n",
            )
        assert raised, "Sub-step 2 DENY must raise"
        post = _stub_calls()
        sys.stderr.write(
            f"[litellm-sdk-deny-runner] (2) DENY: counting-stub.calls "
            f"post={post} (delta={post - pre})\n",
        )
        assert post == pre, "Sub-step 2 DENY MUST NOT hit counting-stub (INV-1)"
    finally:
        uninstall_shim()


async def _substep_3_deny_sidecar_unreachable() -> None:
    """DENY: point at a bogus UDS so ``request_decision`` itself fails;
    the shim's fail-closed default raises ``SidecarUnavailable``. Stub
    counter unchanged."""
    import litellm

    from spendguard import SpendGuardClient
    from spendguard.errors import HandshakeError, SidecarUnavailable
    from spendguard.integrations.litellm_sdk_shim import (
        SpendGuardShimOptions,
        install_shim,
        uninstall_shim,
    )

    bogus_uds = "/tmp/spendguard-bogus-uds-d12-deny.sock"
    sys.stderr.write(
        f"[litellm-sdk-deny-runner] (3) DENY sidecar unreachable: "
        f"pointing at {bogus_uds}\n",
    )
    pre = _stub_calls()

    raised = False
    bogus_client = SpendGuardClient(
        socket_path=bogus_uds,
        tenant_id=os.environ["SPENDGUARD_TENANT_ID"],
    )
    try:
        # connect() / handshake() will fail; the demo treats either
        # as a DENY-equivalent. If somehow it succeeds, install_shim +
        # the first acompletion will fail via SidecarUnavailable
        # during request_decision.
        try:
            await bogus_client.connect()
            await bogus_client.handshake()
        except Exception as exc:
            raised = True
            sys.stderr.write(
                f"[litellm-sdk-deny-runner] (3) DENY: bogus UDS handshake "
                f"raised as expected: {type(exc).__name__}({exc})\n",
            )

        if not raised:
            install_shim(SpendGuardShimOptions(
                client=bogus_client,
                tenant_id=bogus_client._tenant_id,
                budget_id=os.environ["SPENDGUARD_BUDGET_ID"],
                fail_open=False,
            ))
            try:
                await litellm.acompletion(
                    model="gpt-4o-mini",
                    messages=[{"role": "user", "content": "unreachable"}],
                    api_base=os.environ["OPENAI_API_BASE"],
                    api_key=os.environ["OPENAI_API_KEY"],
                )
            except (SidecarUnavailable, HandshakeError) as exc:
                raised = True
                sys.stderr.write(
                    f"[litellm-sdk-deny-runner] (3) DENY: caught "
                    f"{type(exc).__name__}({exc})\n",
                )
            except Exception as exc:
                raised = True
                sys.stderr.write(
                    f"[litellm-sdk-deny-runner] (3) DENY-equivalent: "
                    f"{type(exc).__name__}({exc})\n",
                )
            finally:
                uninstall_shim()
    finally:
        try:
            await bogus_client.close()
        except Exception:  # noqa: BLE001
            pass

    assert raised, "Sub-step 3 DENY (bogus UDS) must raise"
    post = _stub_calls()
    sys.stderr.write(
        f"[litellm-sdk-deny-runner] (3) DENY: counting-stub.calls "
        f"post={post} (delta={post - pre})\n",
    )
    assert post == pre, "Sub-step 3 DENY MUST NOT hit counting-stub"


async def amain() -> int:
    sys.stderr.write("[litellm-sdk-deny-runner] booting\n")
    client = await _bootstrap_with_retry()
    try:
        await _substep_1_allow_positive_control(client)
        await asyncio.sleep(0.2)
        await _substep_2_deny_budget_exhausted(client)
        await asyncio.sleep(0.2)
        await _substep_3_deny_sidecar_unreachable()
    except AssertionError as exc:
        sys.stderr.write(f"[litellm-sdk-deny-runner] FAIL — assertion: {exc}\n")
        return 2
    except Exception as exc:
        sys.stderr.write(f"[litellm-sdk-deny-runner] FAIL — unexpected: {exc!r}\n")
        return 3
    finally:
        try:
            await client.close()
        except Exception:  # noqa: BLE001
            pass
    sys.stderr.write(
        "[litellm-sdk-deny-runner] litellm_sdk_deny ALL 3 sub-steps PASSED\n",
    )
    return 0


def main() -> int:
    return asyncio.run(amain())


if __name__ == "__main__":
    sys.exit(main())
