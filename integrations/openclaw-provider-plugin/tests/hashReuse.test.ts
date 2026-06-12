import { readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

import { flattenOpenClawPrompt } from "../src/flatten.js";

const packageRoot = dirname(dirname(fileURLToPath(import.meta.url)));
const forbiddenSourceTokens = [
  "node:crypto",
  "@noble/hashes",
  "createHash",
  "blake2",
  "failOpen",
  "degradeOnUnavailable",
  "SPENDGUARD_DISABLE",
] as const;

describe("OpenClaw prompt flattening", () => {
  it("flattens structured prompt/message content deterministically", () => {
    expect(
      flattenOpenClawPrompt({
        messages: [
          { role: "system", content: "stay within budget" },
          { role: "user", content: [{ type: "text", text: "hello" }] },
        ],
      }),
    ).toBe("system: stay within budget\nuser: hello");
  });
});

describe("OpenClaw source hygiene", () => {
  it("contains no local hash implementation or fail-open bypass tokens", () => {
    const violations: string[] = [];
    for (const file of walk(join(packageRoot, "src"))) {
      const text = readFileSync(file, "utf8");
      for (const token of forbiddenSourceTokens) {
        if (text.includes(token)) {
          violations.push(`${file}:${token}`);
        }
      }
    }

    expect(violations).toEqual([]);
  });
});

function walk(root: string): string[] {
  const files: string[] = [];
  for (const name of readdirSync(root)) {
    const path = join(root, name);
    const stat = statSync(path);
    if (stat.isDirectory()) {
      files.push(...walk(path));
    } else if (path.endsWith(".ts")) {
      files.push(path);
    }
  }
  return files;
}
