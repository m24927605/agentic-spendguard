// AgUiEventValidationError — the single error class thrown by the
// @spendguard/ag-ui validators, builders, and canonical serializer.
//
// `field` carries the payload-key-style (snake_case) name of the offending
// field (design.md §8.1; review-standards §5.6) — NOT the camelCase input
// property name. Serializer-level violations that have no payload key use
// the sentinel names `"(value)"` / `"(key)"`.
export class AgUiEventValidationError extends Error {
  readonly field: string;

  constructor(field: string, message?: string) {
    super(message ?? `invalid value for field "${field}"`);
    this.name = "AgUiEventValidationError";
    this.field = field;
  }
}
