// The smallest test runner that reads well: each test logs a tick or throws.

export function test(name, body) {
  body();
  console.log(`✓ ${name}`);
}

export const assert = {
  equal(actual, expected) {
    if (actual !== expected) throw new Error(`${actual} is not ${expected}`);
  },
  deepEqual(actual, expected) {
    if (JSON.stringify(actual) !== JSON.stringify(expected)) throw new Error(`${actual} is not ${expected}`);
  },
};
