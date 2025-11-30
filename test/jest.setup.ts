process.env.NODE_ENV = process.env.NODE_ENV || 'test';

import { requestContext } from '../src/lib/request-context';
import { prisma } from '../src/lib/db';
import { truncateAll } from './helpers/seed';

process.env.SIGNING_SECRET = process.env.SIGNING_SECRET || 'testsecret';
process.env.GITHUB_WEBHOOK_SECRET = process.env.GITHUB_WEBHOOK_SECRET || 'testhooksecret';

jest.setTimeout(20000);

beforeEach(async () => {
  requestContext.disable?.();
  await truncateAll();
});

afterAll(async () => {
  try {
    const { closeRedis } = await import('../src/lib/redis');
    await closeRedis();
  } catch {
    /* ignore */
  }
  await prisma.$disconnect().catch(() => {});
});
