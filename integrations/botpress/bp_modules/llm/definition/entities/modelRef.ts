/* eslint-disable */
/* tslint:disable */
// This file is generated. Do not edit it manually.

import { z } from "@botpress/sdk";
export const modelRef = {
  schema: z
    .object({
      id: z
        .string()
        .title("LLM Model ID")
        .describe("Unique identifier of the large language model"),
    })
    .catchall(z.never()),
};
