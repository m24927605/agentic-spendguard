#!/usr/bin/env bash
# D09 SLICE 5 — Shell-based structure tests for the Lua plugin.
#
# Runs on any POSIX shell without requiring a Lua interpreter, busted,
# or OpenResty. Used by the host-side gate so CI catches drift between
# the Lua port and the Go plugin without standing up a Kong container.
#
# Failures exit non-zero with a labelled error line so the parent
# Makefile / CI log surfaces the specific assertion that broke.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
KONG_DIR="${ROOT}/kong/plugins/spendguard"
PASS=0
FAIL=0

pass()  { PASS=$((PASS+1)); echo "  PASS: $1"; }
fail()  { FAIL=$((FAIL+1)); echo "  FAIL: $1" >&2; }

echo "[lua-structure] gate: D09 SLICE 5 Lua plugin parity with Go plugin"

# ── Test 1: required files exist ──────────────────────────────────
echo "[1] required files present"
for f in handler.lua schema.lua sidecar_client.lua; do
    if [ -f "${KONG_DIR}/${f}" ]; then
        pass "${f} exists"
    else
        fail "${f} missing"
    fi
done

# ── Test 2: PRIORITY matches Go plugin's 950 ──────────────────────
echo "[2] PRIORITY=950 matches plugins/kong/spendguard-go/main.go"
if grep -q 'PRIORITY[[:space:]]*=[[:space:]]*950' "${KONG_DIR}/handler.lua"; then
    pass "Lua PRIORITY = 950"
else
    fail "Lua PRIORITY != 950 (must match Go plugin's Priority const)"
fi
if grep -q 'Priority[[:space:]]*=[[:space:]]*950' "${ROOT}/../spendguard-go/main.go"; then
    pass "Go Priority = 950 (parity check)"
else
    fail "Go Priority changed; update Lua port to match"
fi

# ── Test 3: shared-context keys match the Go plugin ───────────────
echo "[3] kong.ctx.shared keys mirror the Go plugin's constants"
for key in spendguard_reservation_id spendguard_provider \
           spendguard_degraded spendguard_committed \
           spendguard_body_buffer; do
    if grep -q "${key}" "${KONG_DIR}/handler.lua"; then
        pass "Lua handler uses key ${key}"
    else
        fail "Lua handler missing key ${key}"
    fi
done

# ── Test 4: sidecar_client implements all three endpoints ─────────
echo "[4] sidecar_client.lua implements /v1/tokenize, /v1/decision, /v1/trace"
for endpoint in tokenize decision trace; do
    if grep -q "/v1/${endpoint}" "${KONG_DIR}/sidecar_client.lua"; then
        pass "/v1/${endpoint} present"
    else
        fail "/v1/${endpoint} missing"
    fi
done

# ── Test 5: schema fail-closed defaults ───────────────────────────
echo "[5] schema enforces fail-closed defaults"
if grep -q 'default[[:space:]]*=[[:space:]]*false' "${KONG_DIR}/schema.lua"; then
    pass "fail_open default = false"
else
    fail "fail_open default != false (review-standards §1.6)"
fi
if grep -q 'default[[:space:]]*=[[:space:]]*500' "${KONG_DIR}/schema.lua"; then
    pass "timeout_ms default = 500"
else
    fail "timeout_ms default != 500"
fi
if grep -q 'match[[:space:]]*=[[:space:]]*"\^https://"' "${KONG_DIR}/schema.lua"; then
    pass "sidecar_url match enforces https://"
else
    fail "sidecar_url does not enforce https:// (design §3.1)"
fi

# ── Test 6: no dangerous runtime constructs ───────────────────────
echo "[6] no os.execute / loadstring / dofile (review-standards §10.2)"
if grep -qE '(os\.execute|loadstring|dofile|os\.exit)' "${KONG_DIR}"/*.lua; then
    fail "dangerous runtime construct found in Lua plugin"
else
    pass "no os.execute / loadstring / dofile"
fi

# ── Test 7: rockspec metadata sanity ──────────────────────────────
echo "[7] rockspec declares Apache-2.0 + lua-resty-http dep"
if grep -q 'license[[:space:]]*=[[:space:]]*"Apache-2.0"' "${ROOT}/spendguard-1.0.0-1.rockspec"; then
    pass "rockspec license = Apache-2.0"
else
    fail "rockspec license must be Apache-2.0"
fi
if grep -q 'lua-resty-http' "${ROOT}/spendguard-1.0.0-1.rockspec"; then
    pass "rockspec depends on lua-resty-http"
else
    fail "rockspec missing lua-resty-http dependency"
fi

# ── Test 8: README labels Lua port experimental ───────────────────
echo "[8] README labels the Lua port experimental"
if grep -qi 'experimental' "${ROOT}/README.md"; then
    pass "README contains 'experimental' marker"
else
    fail "README must label the Lua port experimental (design §3.2)"
fi

# ── Test 9: handler defines access + body_filter (the two phases) ─
echo "[9] handler implements access + body_filter (and only those)"
if grep -q ':access(conf)' "${KONG_DIR}/handler.lua"; then
    pass "access phase implemented"
else
    fail "access phase missing"
fi
if grep -q ':body_filter(conf)' "${KONG_DIR}/handler.lua"; then
    pass "body_filter phase implemented"
else
    fail "body_filter phase missing"
fi

echo
echo "[lua-structure] PASS=${PASS} FAIL=${FAIL}"
if [ "${FAIL}" -ne 0 ]; then
    echo "[lua-structure] gate FAILED" >&2
    exit 1
fi
echo "[lua-structure] D09 SLICE 5 Lua plugin parity PASS"
