# Third-party licence notices — @spendguard/flowise-nodes

This package is Apache-2.0 licensed.

## Peer dependencies

- `@spendguard/sdk` — Apache-2.0, copyright Michael Chen / SpendGuard.
- `@spendguard/langchain` — Apache-2.0, copyright Michael Chen /
  SpendGuard.
- `flowise-components` — Apache-2.0, copyright Flowise AI Inc.
  See https://github.com/FlowiseAI/Flowise/blob/main/LICENSE.

## Runtime dependencies

The published bundle has no transitive runtime dependencies beyond the
peer set above. The `tsup` build externalises `@spendguard/sdk`,
`@spendguard/langchain`, `flowise-components`, and `@langchain/core` so
the consumer's Flowise install wins.

## Dev / build dependencies

- `tsup`, `typescript`, `vitest`, `@biomejs/biome` — MIT.
- `@types/node` — MIT.
