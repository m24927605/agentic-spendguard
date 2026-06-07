// Minimal mutable BaseChatModel-shaped stub.
//
// The wrapper only touches `callbacks`; we don't need to wire RunManager
// events for the unit suite. The E2E suite (tests/e2e) drives a real
// `@langchain/openai` ChatOpenAI through the wrapper end-to-end.

import type { MutableChatModel } from "../../src/nodes/SpendGuardChatModelWrapper.js";

export interface RecordingChatModel extends MutableChatModel {
  // biome-ignore lint/suspicious/noExplicitAny: callback list is heterogeneous
  callbacks: any[];
  /** Identity marker so tests can assert reference equality. */
  readonly _id: string;
}

export function createChatModel(id = "stub-chat-model"): RecordingChatModel {
  return {
    callbacks: [],
    _id: id,
  };
}
