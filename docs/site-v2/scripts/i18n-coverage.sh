#!/usr/bin/env bash
# i18n coverage report for the docs site.
#
# Surface layout: English (root locale) content lives at
#   src/content/docs/docs/**            and locale content at
#   src/content/docs/<locale>/docs/**   (zh-tw = 繁體, zh-cn = 簡體).
# A page is "covered" for a locale when the same key (path under docs/)
# exists in that locale tree. Run from docs/site-v2/ or anywhere.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/src/content/docs"
LOCALES=(zh-tw zh-cn)

keys() { # keys <dir-relative-to-ROOT>
  ( cd "$ROOT/$1" 2>/dev/null && find docs -type f \( -name '*.md' -o -name '*.mdx' \) \
      | sed -E 's#^docs/##' | sort ) || true
}

EN="$(keys .)"
TW="$(cd "$ROOT/zh-tw" 2>/dev/null && find docs -type f \( -name '*.md' -o -name '*.mdx' \) | sed -E 's#^docs/##' | sort || true)"
CN="$(cd "$ROOT/zh-cn" 2>/dev/null && find docs -type f \( -name '*.md' -o -name '*.mdx' \) | sed -E 's#^docs/##' | sort || true)"

n() { printf '%s\n' "$1" | sed '/^$/d' | wc -l | tr -d ' '; }

echo "# docs i18n coverage"
echo
echo "English pages: $(n "$EN")   |   繁體 zh-tw: $(n "$TW")   |   簡體 zh-cn: $(n "$CN")"
echo

echo "## Covered (translated)"
both="$(comm -12 <(printf '%s\n' "$TW") <(printf '%s\n' "$CN") | sed '/^$/d')"
printf '%s\n' "$both" | sed 's/^/  ✓ /'
echo

echo "## Asymmetric / orphan checks"
tw_only="$(comm -23 <(printf '%s\n' "$TW") <(printf '%s\n' "$CN") | sed '/^$/d')"
cn_only="$(comm -13 <(printf '%s\n' "$TW") <(printf '%s\n' "$CN") | sed '/^$/d')"
orphan="$(comm -13 <(printf '%s\n' "$EN") <(sort -u <(printf '%s\n' "$TW") <(printf '%s\n' "$CN")) | sed '/^$/d')"
[ -z "$tw_only" ] && echo "  繁 only:    none" || { echo "  繁 only (missing 簡):"; printf '%s\n' "$tw_only" | sed 's/^/    /'; }
[ -z "$cn_only" ] && echo "  簡 only:    none" || { echo "  簡 only (missing 繁):"; printf '%s\n' "$cn_only" | sed 's/^/    /'; }
[ -z "$orphan" ]  && echo "  orphan:     none (every translation maps to an English source)" \
                  || { echo "  orphan (no English source):"; printf '%s\n' "$orphan" | sed 's/^/    /'; }
echo

echo "## Untranslated English pages (by section, lines desc)"
untrans="$(comm -23 <(printf '%s\n' "$EN") <(sort -u <(printf '%s\n' "$TW") <(printf '%s\n' "$CN")) | sed '/^$/d')"
while IFS= read -r k; do
  [ -z "$k" ] && continue
  lines=$(wc -l < "$ROOT/docs/$k" | tr -d ' ')
  sec=$(dirname "$k"); [ "$sec" = "." ] && sec="(root)"
  printf '%s\t%s\t%s\n' "$sec" "$lines" "$k"
done <<< "$untrans" | sort -t$'\t' -k1,1 -k2,2nr \
  | awk -F'\t' '{printf "  %-22s %5s  %s\n", $1, $2, $3}'
echo
echo "  untranslated total: $(n "$untrans")"
