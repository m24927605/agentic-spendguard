export class OpenClawSpendGuardError extends Error {
  constructor(message: string) {
    super(message);
    this.name = new.target.name;
  }
}

export class OpenClawSpendGuardNotImplementedError extends OpenClawSpendGuardError {
  constructor(feature: string) {
    super(`${feature} is not implemented in the current D40b slice`);
  }
}

export class OpenClawSpendGuardConfigError extends OpenClawSpendGuardError {}
