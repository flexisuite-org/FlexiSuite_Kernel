import crypto from 'crypto';

export function signHmac(data: string, secret: string) {
  return crypto.createHmac('sha256', secret).update(data).digest('hex');
}

export function verifyHmac(data: string, signature: string, secret: string) {
  const expected = signHmac(data, secret);
  return crypto.timingSafeEqual(Buffer.from(expected), Buffer.from(signature));
}
