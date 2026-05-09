# Phase 4 O3 — pricing seed loader.
# Reads /seed.yaml + writes pricing_table + pricing_versions in canonical DB,
# then drops /bundles/pricing.env so bundles-init picks up the real
# pricing_version + price_snapshot_hash.

FROM python:3.12-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    postgresql-client coreutils \
    && rm -rf /var/lib/apt/lists/* \
    && pip install --no-cache-dir PyYAML==6.0.2

COPY seed.sh /usr/local/bin/seed.sh
RUN chmod +x /usr/local/bin/seed.sh

ENV PRICING_YAML=/seed.yaml
ENTRYPOINT ["/usr/local/bin/seed.sh"]
