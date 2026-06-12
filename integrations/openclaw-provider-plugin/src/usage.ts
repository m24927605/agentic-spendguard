import { OpenClawSpendGuardNotImplementedError } from "./errors.js";

export function extractOpenClawUsage(): never {
  throw new OpenClawSpendGuardNotImplementedError("OpenClaw usage extraction");
}
