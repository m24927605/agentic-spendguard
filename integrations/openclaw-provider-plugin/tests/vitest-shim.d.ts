declare module "vitest" {
  export function describe(name: string, fn: () => void): void;
  export function it(name: string, fn: () => void | Promise<void>): void;

  export function expect<T>(actual: T): {
    toBe(expected: unknown): void;
    toContain(expected: string): void;
    toEqual(expected: unknown): void;
    toThrow(expected?: unknown): void;
    rejects: {
      toThrow(expected?: unknown): Promise<void>;
    };
  };
}
