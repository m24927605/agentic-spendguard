# GA_01 Release Packaging Evidence

Date: 2026-05-31
Branch: `ga/GA_01_release_packaging`
Tested implementation head: `745f428ee773cc85b5be91fcba2b62e7a2d9e05f`
Evidence commit: this file is updated after each gate rerun; current branch head may be the evidence-only commit that records this result.

## Commands

```bash
scripts/release/build-release-bundle.sh --output /tmp/spendguard-ga01-release
scripts/release/check-release-bundle.sh /tmp/spendguard-ga01-release
helm template spendguard charts/spendguard --set chart.profile=demo >/tmp/spendguard-ga01-helm-demo.yaml
helm template spendguard charts/spendguard -f scripts/helm-validate-test-values.yaml >/tmp/spendguard-ga01-helm-prod.yaml
```

## Result

- Release bundle build: PASS
- Release bundle check: PASS
- Moved release bundle check: PASS
- Helm demo render: PASS
- Helm production validation render: PASS
- R5 symlink tamper regression: PASS, failed closed
- R5 relative migration checksum regression: PASS
- R5 committed-tree release-notes pointer regression from clean `main` worktree: PASS
- Bundle checksum manifest checksum: `9710757ddb36a267835669124c42ba0cc8d79ce4c2b195522f5c21d2bbad4155`

The bundle checker validated required non-symlink files, chart package presence and chart metadata, commit SHA shape, v1alpha1 manifest required fields, manifest consistency, fixed release notes template pointer resolution in the committed tree, full checksum coverage, portable deploy-only migration inventory checksum and committed-tree parity, unpacked chart content parity with a chart rebuilt from `git archive <commit>`, clean-checkout verification, and common secret-pattern absence across both bundle files and unpacked chart content.
