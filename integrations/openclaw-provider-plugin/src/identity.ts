import { OpenClawSpendGuardNotImplementedError } from "./errors.js";

export function prepareOpenClawIdentity(): never {
  throw new OpenClawSpendGuardNotImplementedError("OpenClaw identity derivation");
}
