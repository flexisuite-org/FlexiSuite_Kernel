import crypto from 'crypto';

// Deterministic JSON stringifier (sorts object keys, keeps array order).
export function stableStringify(value: any): string {
  const seen = new WeakSet();
  const helper = (input: any): any => {
    if (input === null || typeof input !== 'object') return input;
    if (seen.has(input)) throw new TypeError('circular reference in stableStringify');
    seen.add(input);
    if (Array.isArray(input)) return input.map(helper);
    const sorted = Object.keys(input).sort().reduce((acc, key) => {
      acc[key] = helper(input[key]);
      return acc;
    }, {} as any);
    seen.delete(input);
    return sorted;
  };
  return JSON.stringify(helper(value));
}

export function sha256Hex(data: Buffer | string) {
  return crypto.createHash('sha256').update(data).digest('hex');
}

export function hashJson(value: any) {
  return sha256Hex(stableStringify(value));
}

export function verifyIntegrity(expected: string, data: Buffer | string | object | null | undefined) {
  const payload =
    data === undefined
      ? 'undefined'
      : typeof data === 'string' || Buffer.isBuffer(data)
      ? data
      : stableStringify(data);
  return sha256Hex(payload) === expected;
}
