import crypto from 'crypto';

export function sha256Hex(data: Buffer | string) {
  return crypto.createHash('sha256').update(data).digest('hex');
}

export function verifyIntegrity(expected: string, data: Buffer | string) {
  return sha256Hex(data) === expected;
}
