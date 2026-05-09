"""Pricing lookup helper — Phase 4 O4 USD-denominated budget support.

Adapters that want to charge an LLM call against a USD-denominated
budget need a way to convert (provider, model, token_counts) into
USD-micros. This module exposes a small `PricingLookup` that takes a
frozen pricing table (matching the seed YAML in
`deploy/demo/init/pricing/seed.yaml`) and computes µUSD per call.

For production, the pricing table comes from
`spendguard_canonical.pricing_table` keyed by the bundle's
`pricing_version`. POC sidecar adapters hardcode a subset; a future
slice will surface the full table via the sidecar handshake or a
control-plane API.

Usage::

    from spendguard.pricing import PricingLookup, USD_MICROS_PER_USD

    pricing = PricingLookup({
        ("openai", "gpt-4o-mini", "input"):  0.15,    # $/1M tokens
        ("openai", "gpt-4o-mini", "output"): 0.60,
        ("anthropic", "claude-haiku-4-5-20251001", "input"):  1.00,
        ("anthropic", "claude-haiku-4-5-20251001", "output"): 5.00,
    })

    micros = pricing.usd_micros_for_call(
        provider="openai", model="gpt-4o-mini",
        input_tokens=120, output_tokens=40,
    )
    # → math.ceil((120 * 0.15 + 40 * 0.60) / 1e6 * 1e6) → 42 µUSD = $0.000042
"""

from __future__ import annotations

import math
from collections.abc import Mapping

USD_MICROS_PER_USD = 1_000_000

PriceKey = tuple[str, str, str]  # (provider, model, token_kind)
PriceTable = Mapping[PriceKey, float]


class PricingLookup:
    """Frozen-at-construction pricing table → USD-micros computation.

    The lookup is *not* side-effecting — there is no DB query, no
    network call. Callers fetch / cache pricing once (e.g., at
    handshake time) and pass it in.
    """

    def __init__(self, table: PriceTable, *, default_kind: str = "output") -> None:
        self._table = dict(table)
        self._default_kind = default_kind

    def price_per_million(
        self, provider: str, model: str, token_kind: str
    ) -> float | None:
        """Return $/1M-tokens or None if (provider, model, kind) is missing."""
        return self._table.get((provider, model, token_kind))

    def usd_micros_for_call(
        self,
        *,
        provider: str,
        model: str,
        input_tokens: int = 0,
        output_tokens: int = 0,
        cached_input_tokens: int = 0,
    ) -> int:
        """Compute the µUSD cost of a single LLM call.

        Charges per-kind when prices are available; falls back to the
        default kind (typically `output`) for any token bucket without
        a configured price. Round up to the nearest µUSD so the
        customer is never under-charged due to floating-point
        truncation.
        """
        usd = 0.0
        for kind, count in (
            ("input", input_tokens),
            ("output", output_tokens),
            ("cached_input", cached_input_tokens),
        ):
            if count <= 0:
                continue
            price = self.price_per_million(provider, model, kind)
            if price is None:
                price = self.price_per_million(
                    provider, model, self._default_kind
                ) or 0.0
            usd += count * price / 1_000_000
        # Round up to the nearest µUSD (fail-safe for the customer).
        return max(1, math.ceil(usd * USD_MICROS_PER_USD))


__all__ = ["PriceKey", "PriceTable", "PricingLookup", "USD_MICROS_PER_USD"]
