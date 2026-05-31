# GA_01 Release Packaging Evidence

Date: 2026-05-31
Branch: `ga/GA_01_release_packaging`
Tested implementation head: `d1d38b45a7b34dbe1deef62b85e905a61ab11613`
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
- Helm demo render: PASS
- Helm production validation render: PASS
- Bundle checksum manifest checksum: `6143ee00b59826ea532c4ac3fd369638d5f4143b7d216f2315c38887efb8b4a2`

The bundle checker validated required files, chart package presence and chart metadata, commit SHA shape, v1alpha1 manifest required fields, manifest consistency, fixed release notes template pointer resolution in the committed tree, full checksum coverage, deploy-only migration inventory checksum and committed-tree parity, unpacked chart content parity with a chart rebuilt from `git archive <commit>`, clean-checkout verification, and common secret-pattern absence across both bundle files and unpacked chart content.
