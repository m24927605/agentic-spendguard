export interface OpenClawUsage {
  inputTokens?: number;
  outputTokens?: number;
  providerEventId?: string;
}

export function extractOpenClawUsage(value: unknown): OpenClawUsage | undefined {
  const found = findUsageObject(value, 0);
  const providerEventId = findProviderEventId(value);
  if (found === undefined) {
    return providerEventId === undefined ? undefined : { providerEventId };
  }
  const inputTokens =
    readNumber(found, ["inputTokens", "input_tokens", "promptTokens", "prompt_tokens"]) ?? 0;
  const outputTokens =
    readNumber(found, ["outputTokens", "output_tokens", "completionTokens", "completion_tokens"]) ??
    0;
  const totalTokens = readNumber(found, ["totalTokens", "total_tokens"]);
  if (inputTokens <= 0 && outputTokens <= 0 && totalTokens !== undefined && totalTokens > 0) {
    return { inputTokens: totalTokens, outputTokens: 0, ...(providerEventId ? { providerEventId } : {}) };
  }
  if (inputTokens <= 0 && outputTokens <= 0) {
    return providerEventId === undefined ? undefined : { providerEventId };
  }
  return {
    inputTokens,
    outputTokens,
    ...(providerEventId ? { providerEventId } : {}),
  };
}

export function mergeOpenClawUsage(
  current: OpenClawUsage | undefined,
  next: OpenClawUsage | undefined,
): OpenClawUsage | undefined {
  if (next === undefined) return current;
  const inputTokens = next.inputTokens ?? current?.inputTokens;
  const outputTokens = next.outputTokens ?? current?.outputTokens;
  const providerEventId = next.providerEventId ?? current?.providerEventId;
  return {
    ...(inputTokens !== undefined ? { inputTokens } : {}),
    ...(outputTokens !== undefined ? { outputTokens } : {}),
    ...(providerEventId !== undefined ? { providerEventId } : {}),
  };
}

function findUsageObject(value: unknown, depth: number): Record<string, unknown> | undefined {
  if (depth > 4 || value === null || typeof value !== "object") return undefined;
  const record = value as Record<string, unknown>;
  if ("usage" in record && record.usage !== null && typeof record.usage === "object") {
    return record.usage as Record<string, unknown>;
  }
  if (hasAnyUsageKey(record)) return record;
  for (const key of ["response", "data", "message", "chunk", "delta"]) {
    const child = findUsageObject(record[key], depth + 1);
    if (child !== undefined) return child;
  }
  return undefined;
}

function hasAnyUsageKey(record: Record<string, unknown>): boolean {
  return [
    "inputTokens",
    "input_tokens",
    "promptTokens",
    "prompt_tokens",
    "outputTokens",
    "output_tokens",
    "completionTokens",
    "completion_tokens",
    "totalTokens",
    "total_tokens",
  ].some((key) => key in record);
}

function readNumber(record: Record<string, unknown>, keys: readonly string[]): number | undefined {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === "number" && Number.isFinite(value) && value >= 0) return value;
    if (typeof value === "bigint" && value >= 0n && value <= BigInt(Number.MAX_SAFE_INTEGER)) {
      return Number(value);
    }
  }
  return undefined;
}

function findProviderEventId(value: unknown): string | undefined {
  if (value === null || typeof value !== "object") return undefined;
  const record = value as Record<string, unknown>;
  for (const key of ["providerEventId", "responseId", "id"]) {
    const id = record[key];
    if (typeof id === "string" && id.length > 0) return id;
  }
  return undefined;
}
