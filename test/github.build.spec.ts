import request from 'supertest';
import jwt from 'jsonwebtoken';
import path from 'path';
import os from 'os';
import fs from 'fs/promises';
import { promisify } from 'util';
import { exec } from 'child_process';
import { buildServer } from '../src/api/server';
import { prisma } from '../src/lib/db';
import { truncateAll, createTenantSeed, createPolicy } from './helpers/seed';
import { config } from '../src/config';

const execAsync = promisify(exec);

function token(userId: string, groupId: string) {
  return jwt.sign({ userId, groupId, roles: [] }, config.JWT_SECRET);
}

describe('github build workflow', () => {
  const app = buildServer();
  let groupId: string;
  let userId: string;
  let policyId: string;

  beforeEach(async () => {
    jest.setTimeout(30000);
    await app.ready();
    await truncateAll();
    const seed = await createTenantSeed(`gh-${Date.now()}`);
    groupId = seed.groupId;
    userId = seed.userId;
    policyId = await createPolicy(`gh-pol-${Date.now()}`);
  });

  afterAll(async () => {
    await prisma.$disconnect();
    await app.close();
    const { closeRedis } = await import('../src/lib/redis');
    await closeRedis();
  });

  it('queues build job and produces bundle upload', async () => {
    const repoDir = await fs.mkdtemp(path.join(os.tmpdir(), 'gh-build-repo-'));
    const pkgName = `@demo/github-${Date.now()}`;

    await execAsync('git init -b main', { cwd: repoDir });
    await execAsync('git config user.email "tester@example.com"', { cwd: repoDir });
    await execAsync('git config user.name "Tester"', { cwd: repoDir });
    await fs.writeFile(path.join(repoDir, 'README.md'), '# demo');
    await execAsync('git add README.md', { cwd: repoDir });
    await execAsync('git commit -m "init"', { cwd: repoDir });

    const buildCommand = 'mkdir -p dist && echo "ok" > dist/out.txt';

    const enqueue = await request(app.server)
      .post('/integrations/github/build')
      .set('authorization', 'Bearer ' + token(userId, groupId))
      .send({
        repo: repoDir,
        branch: 'main',
        buildCommand,
        artifactPath: 'dist',
        packageName: pkgName,
        version: '1.0.0',
        policyId,
        approve: true
      });

    expect(enqueue.status).toBe(202);
    const jobId = enqueue.body.jobId;
    expect(jobId).toBeDefined();

    let status: any;
    for (let i = 0; i < 25; i++) {
      const res = await request(app.server)
        .get('/integrations/github/status')
        .set('authorization', 'Bearer ' + token(userId, groupId))
        .query({ jobId });
      if (res.status === 200) {
        status = res.body;
        if (status.status === 'done' || status.status === 'failed') break;
      }
      await new Promise((r) => setTimeout(r, 300));
    }

    expect(status?.status).toBe('done');
    expect(status?.packageId).toBeTruthy();

    const pkg = await prisma.componentPackage.findFirst({
      where: { id: status.packageId, ownerGroupId: groupId }
    });
    expect(pkg?.bundleIntegrity).toBeTruthy();

    const bundlePath = path.join(config.bundleStorage.localDir, `${status.packageId}.bin`);
    const stat = await fs.stat(bundlePath);
    expect(stat.isFile()).toBe(true);

    await fs.rm(repoDir, { recursive: true, force: true });
  });
});
