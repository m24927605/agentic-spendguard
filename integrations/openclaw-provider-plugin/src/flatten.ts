const PROMPT_FIELDS = ["messages", "prompt", "input", "text", "content"] as const;

export function flattenOpenClawPrompt(request: unknown): string {
  const flattened = flattenValue(request, 0).trim();
  return flattened.length > 0 ? flattened : stableStringify(request);
}

function flattenValue(value: unknown, depth: number): string {
  if (depth > 8 || value === null || value === undefined) return "";
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean" || typeof value === "bigint") {
    return String(value);
  }
  if (Array.isArray(value)) {
    return value
      .map((item) => flattenValue(item, depth + 1))
      .filter((item) => item.length > 0)
      .join("\n");
  }
  if (typeof value !== "object") return "";

  const record = value as Record<string, unknown>;
  const role = typeof record.role === "string" && record.role.length > 0 ? `${record.role}: ` : "";
  for (const field of PROMPT_FIELDS) {
    if (field in record) {
      const child = flattenValue(record[field], depth + 1);
      if (child.length > 0) return `${role}${child}`;
    }
  }
  return stableStringify(record);
}

function stableStringify(value: unknown): string {
  if (value === null || value === undefined) return "";
  if (typeof value !== "object") return String(value);
  if (Array.isArray(value)) return `[${value.map((item) => stableStringify(item)).join(",")}]`;
  const record = value as Record<string, unknown>;
  const parts = Object.keys(record)
    .sort()
    .map((key) => `${JSON.stringify(key)}:${stableStringify(record[key])}`);
  return `{${parts.join(",")}}`;
}
