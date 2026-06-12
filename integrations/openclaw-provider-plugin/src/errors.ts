export class OpenClawSpendGuardError extends Error {
  constructor(message: string) {
    super(message);
    this.name = new.target.name;
  }
}

export class OpenClawSpendGuardNotImplementedError extends OpenClawSpendGuardError {
  constructor(feature: string) {
    super(`${feature} is not implemented in COV_D40B_01_plugin_package_init`);
  }
}

export class OpenClawSpendGuardConfigError extends OpenClawSpendGuardError {}
