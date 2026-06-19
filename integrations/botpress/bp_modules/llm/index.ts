/* eslint-disable */
/* tslint:disable */
// This file is generated. Do not edit it manually.

import * as sdk from "@botpress/sdk"

import definition from "./definition"

export default {
  type: "interface",
  id: "ifver_01KNRZY99ANVZV16JA27AV2H5B",
  // NOTE: `bp add llm` emits `uri: undefined` here. This project compiles with
  // `exactOptionalPropertyTypes: true`, under which the SDK's own
  // `InterfacePackage` type (`uri?: string`) rejects an explicit-`undefined`
  // value. Omitting the key (an absent optional) is equivalent and conforms.
  // Re-running `bp add llm` would reintroduce `uri: undefined`; re-apply this
  // omission if so.
  name: "llm",
  version: "latest",
  definition,
} satisfies sdk.InterfacePackage