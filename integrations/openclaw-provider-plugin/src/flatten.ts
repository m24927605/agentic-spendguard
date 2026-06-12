import { OpenClawSpendGuardNotImplementedError } from "./errors.js";

export function flattenOpenClawPrompt(): never {
  throw new OpenClawSpendGuardNotImplementedError("OpenClaw prompt flattening");
}
