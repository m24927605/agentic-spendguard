// Minimal `BaseChatModel`-like fixture used by D37 unit tests.
//
// We do NOT subclass `BaseChatModel` from `@langchain/core/language_models/
// chat_models` — instantiating the real ABC drags in a giant prototype
// chain for what is purely a `callbacks` carrier. Tests assert on
// reference equality and on the `callbacks` shape after `supplyData`
// pushes the SpendGuard handler; the prototype identity check in
// `N-16 returns the original prototype chain` operates against a marker
// symbol the fixture sets, not a real `instanceof` test.

import type { BaseCallbackHandler } from "@langchain/core/callbacks/base";

export const FIXTURE_MARKER = Symbol.for("d37.test.upstreamMarker");

export interface MockUpstreamModel {
  callbacks?: BaseCallbackHandler[] | BaseCallbackHandler;
  [FIXTURE_MARKER]: true;
  modelName: string;
}

export function makeMockUpstreamModel(partial: Partial<MockUpstreamModel> = {}): MockUpstreamModel {
  return {
    modelName: "fixture-chat-model",
    callbacks: undefined,
    ...partial,
    [FIXTURE_MARKER]: true as const,
  };
}
