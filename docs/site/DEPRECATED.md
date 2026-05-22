# DEPRECATED — MkDocs source kept as a one-cycle rollback parachute

As of **2026-05-22** the public docs site (https://agenticspendguard.dev/)
is built from `docs/site-v2/` using Astro Starlight, not from this
directory. The old MkDocs source lives on for one release cycle so the
previous build is one revert away if the Astro cutover surfaces a
regression we did not catch.

If you are looking for the live docs source, edit pages under
`docs/site-v2/src/content/docs/`.

This folder will be deleted once the new site has been live for a full
release cycle with no rollback. Tracking issue: see the PR that
introduced `docs/site-v2/`.
