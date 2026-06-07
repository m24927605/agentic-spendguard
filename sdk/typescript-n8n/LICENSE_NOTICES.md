# License Notices — `n8n-nodes-spendguard`

This package is distributed under the Apache License 2.0 (see the repo
root `LICENSE` file).

## Third-party dependencies

| Package                  | License                              | Notes                                              |
| ------------------------ | ------------------------------------ | -------------------------------------------------- |
| `@spendguard/sdk`        | Apache-2.0                           | Runtime dep (pinned `workspace:*` in source tree). |
| `@spendguard/langchain`  | Apache-2.0                           | Runtime dep.                                       |
| `n8n-workflow`           | n8n Sustainable Use License (peer)   | Peer-dep; NOT redistributed.                       |
| `@langchain/core`        | MIT                                  | Peer-dep.                                          |

n8n itself ships under the [n8n Sustainable Use License](https://docs.n8n.io/sustainable-use-license/);
this community node is a peer of the n8n runtime and is NOT
redistributed with it.
