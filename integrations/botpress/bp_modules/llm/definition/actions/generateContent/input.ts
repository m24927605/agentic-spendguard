/* eslint-disable */
/* tslint:disable */
// This file is generated. Do not edit it manually.

import { z } from "@botpress/sdk";
export const input = {
  schema: z
    .object({
      model: z.optional(
        z.ref("modelRef").describe("Model to use for content generation"),
      ),
      reasoningEffort: z.optional(
        z
          .enum(["low", "medium", "high", "dynamic", "none"])
          .title("Reasoning Effort Level")
          .describe(
            'Reasoning effort level to use for models that support reasoning. Specifying \"none\" will indicate the LLM to not use reasoning (for models that support optional reasoning). A \"dynamic\" effort will indicate the provider to automatically determine the reasoning effort (if supported by the provider). If not provided the model will not use reasoning for models with optional reasoning or use the default reasoning effort specified by the provider for reasoning-only models.\nNote: A higher reasoning effort will incur in higher output token charges from the LLM provider.',
          ),
      ),
      systemPrompt: z.optional(
        z
          .string()
          .title("System Prompt")
          .describe("Optional system prompt to guide the model"),
      ),
      messages: z
        .array(
          z
            .object({
              role: z.enum(["user", "assistant"]),
              type: z.default(
                z.enum(["text", "tool_calls", "tool_result", "multipart"]),
                "text",
              ),
              toolCalls: z.optional(
                z
                  .array(
                    z
                      .object({
                        id: z.string(),
                        type: z.literal("function"),
                        function: z
                          .object({
                            name: z.string(),
                            arguments: z.union([
                              z
                                .record(z.string(), z.any())
                                .describe(
                                  "Some LLMs may generate invalid JSON for a tool call, so this will be `null` when it happens.",
                                ),
                              z.null(),
                            ]),
                          })
                          .catchall(z.never()),
                      })
                      .catchall(z.never()),
                  )
                  .describe('Required if `type` is "tool_calls"'),
              ),
              toolResultCallId: z.optional(
                z.string().describe('Required if `type` is "tool_result"'),
              ),
              content: z.union([
                z
                  .union([
                    z.string(),
                    z.array(
                      z
                        .object({
                          type: z.enum(["text", "image"]),
                          mimeType: z.optional(
                            z
                              .string()
                              .describe(
                                "Indicates the MIME type of the content. If not provided it will be detected from the content-type header of the provided URL.",
                              ),
                          ),
                          text: z.optional(
                            z
                              .string()
                              .describe('Required if part type is \"text\" '),
                          ),
                          url: z.optional(
                            z
                              .string()
                              .describe('Required if part type is \"image\"'),
                          ),
                        })
                        .catchall(z.never()),
                    ),
                  ])
                  .describe(
                    'Required unless `type` is "tool_call". If `type` is "multipart", this field must be an array of content objects. If `type` is "tool_result" then this field should be the result of the tool call (a plain string or a JSON-encoded array or object). If `type` is "tool_call" then the `toolCalls` field should be used instead.',
                  ),
                z.null(),
              ]),
            })
            .catchall(z.never()),
        )
        .title("Messages to Process")
        .describe("Array of messages for the model to process"),
      responseFormat: z.optional(
        z
          .enum(["text", "json_object"])
          .title("Response Format")
          .describe(
            'Response format expected from the model. If \"json_object\" is chosen, you must instruct the model to generate JSON either via the system prompt or a user message.',
          ),
      ),
      maxTokens: z.optional(
        z
          .number()
          .title("Maximum number of output tokens")
          .describe(
            "Maximum number of tokens allowed in the generated response",
          ),
      ),
      temperature: z.default(
        z
          .number()
          .min(0, undefined)
          .max(2, undefined)
          .title("Temperature")
          .displayAs({
            id: "slider",
            params: { stepSize: 0.01, horizontal: true },
          })
          .describe(
            "Sampling temperature for the model. Higher values result in more random outputs.",
          ),
        1,
      ),
      topP: z.default(
        z
          .number()
          .min(0, undefined)
          .max(1, undefined)
          .title("Top-P")
          .displayAs({
            id: "slider",
            params: { stepSize: 0.01, horizontal: true },
          })
          .describe(
            "Top-p sampling parameter. Limits sampling to the smallest set of tokens with a cumulative probability above the threshold.",
          ),
        1,
      ),
      stopSequences: z.optional(
        z
          .array(z.string())
          .max(4, undefined)
          .title("Stop Sequences")
          .describe(
            "Sequences where the model should stop generating further tokens.",
          ),
      ),
      tools: z.optional(
        z
          .array(
            z
              .object({
                type: z.literal("function"),
                function: z
                  .object({
                    name: z.string().describe("Function name"),
                    description: z.optional(z.string()),
                    argumentsSchema: z.optional(
                      z
                        .object({})
                        .catchall(z.any())
                        .describe("JSON schema of the function arguments"),
                    ),
                  })
                  .catchall(z.never()),
              })
              .catchall(z.never()),
          )
          .title("Tools")
          .describe("List of tools available for the model to use"),
      ),
      toolChoice: z.optional(
        z
          .object({
            type: z.optional(z.enum(["auto", "specific", "any", "none", ""])),
            functionName: z.optional(
              z.string().describe('Required if `type` is "specific"'),
            ),
          })
          .catchall(z.never())
          .title("Tool Choice")
          .describe("The chosen tool to use for content generation"),
      ),
      userId: z.optional(
        z
          .string()
          .title("User ID")
          .describe("Unique identifier of the user that sent the prompt"),
      ),
      debug: z.optional(
        z
          .boolean()
          .title("Debug Mode")
          .hidden(true)
          .describe(
            "Set to `true` to output debug information to the bot logs",
          ),
      ),
      meta: z.optional(
        z
          .object({
            promptSource: z.optional(
              z
                .string()
                .describe(
                  "Source of the prompt, e.g. agent/:id/:version cards/ai-generate, cards/ai-task, nodes/autonomous, etc.",
                ),
            ),
            promptCategory: z.optional(z.string()),
            integrationName: z.optional(
              z
                .string()
                .describe(
                  "Name of the integration that originally received the message that initiated this action",
                ),
            ),
          })
          .catchall(z.never())
          .title("Prompt Metadata")
          .hidden(true)
          .describe("Contextual metadata about the prompt"),
      ),
    })
    .catchall(z.never()),
};
