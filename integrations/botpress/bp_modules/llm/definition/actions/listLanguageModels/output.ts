/* eslint-disable */
/* tslint:disable */
// This file is generated. Do not edit it manually.

import { z } from "@botpress/sdk";
export const output = {
  schema: z
    .object({
      models: z.array(
        z.intersection(
          z.object({
            id: z
              .string()
              .describe("Unique identifier of the large language model"),
            name: z.string(),
            description: z.string(),
            tags: z.array(
              z.enum([
                "recommended",
                "deprecated",
                "general-purpose",
                "low-cost",
                "vision",
                "coding",
                "agents",
                "function-calling",
                "roleplay",
                "storytelling",
                "reasoning",
                "preview",
                "speech-to-text",
                "image-generation",
                "text-to-speech",
              ]),
            ),
            input: z
              .object({
                maxTokens: z.number().int(undefined),
                costPer1MTokens: z
                  .number()
                  .describe("Cost per 1 million tokens, in U.S. dollars"),
              })
              .catchall(z.never()),
            output: z
              .object({
                maxTokens: z.number().int(undefined),
                costPer1MTokens: z
                  .number()
                  .describe("Cost per 1 million tokens, in U.S. dollars"),
              })
              .catchall(z.never()),
          }),
          z.ref("modelRef"),
        ),
      ),
    })
    .catchall(z.never()),
};
