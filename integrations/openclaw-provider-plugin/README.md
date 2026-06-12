# SpendGuard OpenClaw Provider Plugin

This package is the D40b OpenClaw provider plugin adapter skeleton.

Slice `COV_D40B_01_plugin_package_init` pins the OpenClaw provider-plugin
surface to `openclaw@2026.6.2` at commit
`d4819948f37d45fe8f1428401316eaae456cdf16`.

The OpenClaw provider plugin runs in the OpenClaw process. It is an enforcement hook, not a sandbox boundary. Operators should install it only in trusted OpenClaw deployments. Use D40a base-URL routing when the plugin API changes or when plugin installation is not acceptable.

Runtime reserve, commit, streaming, demo, and docs publication behavior are intentionally not implemented in slice `COV_D40B_01_plugin_package_init`.
