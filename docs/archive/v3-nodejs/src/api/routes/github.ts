import { FastifyInstance, FastifyReply, FastifyRequest } from 'fastify';
import crypto from 'crypto';
import { z } from 'zod';
import { prisma } from '../../lib/db';
import { requestContext } from '../../lib/request-context';
import { config } from '../../config';
import { ensureGithubBuildWorker, getGithubBuildQueue } from '../../integrations/github/queue';
import { writeStatus, readStatus } from '../../integrations/github/status-store';
import { GithubBuildJobData } from '../../integrations/github/types';
import semver from 'semver';

interface WebhookBody {
  ref?: string;
  repository?: { full_name?: string };
  head_commit?: { id?: string; message?: string };
}

const buildSchema = z.object({
  repo: z.string().min(1),
  branch: z.string().default('main'),
  buildCommand: z.string().default('npm ci && npm run build'),
  artifactPath: z.string().default('dist'),
  packageName: z.string().min(1),
  version: z.string().min(1).refine((v) => !!semver.valid(v), { message: 'invalid_semver' }),
  policyId: z.string().optional(),
  approve: z.boolean().optional(),
  install: z.boolean().optional(),
  artifactUrl: z.string().url().optional(),
  artifactToken: z.string().optional(),
  manifest: z.any().optional()
});

export default async function githubRoutes(fastify: FastifyInstance) {
  // GitHub Webhook receiver (signature verification if secret configured)
  fastify.post('/webhook', async (req: FastifyRequest, reply: FastifyReply) => {
    const body = req.body as WebhookBody;
    const sig = req.headers['x-hub-signature-256'] as string | undefined;
    const secret = config.GITHUB_WEBHOOK_SECRET;
    if (secret) {
      if (!sig) return reply.code(401).send({ error: 'missing_signature' });
      const raw = (req as any).rawBody || JSON.stringify(body);
      const expected = 'sha256=' + crypto.createHmac('sha256', secret).update(raw).digest('hex');
      try {
        if (!crypto.timingSafeEqual(Buffer.from(expected), Buffer.from(sig))) {
          return reply.code(401).send({ error: 'invalid_signature' });
        }
      } catch {
        return reply.code(401).send({ error: 'invalid_signature' });
      }
    }
    await prisma.auditLog.create({
      data: {
        resource: 'github.webhook',
        action: 'received',
        metadata: { ref: body.ref, repo: body.repository?.full_name, commit: body.head_commit?.id },
        success: true
      }
    });
    reply.code(202).send({ status: 'accepted' });
  });

  // Trigger build from repo
  fastify.post('/build', async (req: FastifyRequest, reply: FastifyReply) => {
    const ctx = requestContext.getStore();
    const user = (req as any).user;
    const groupId = user?.groupId ?? ctx?.groupId;
    const userId = user?.id ?? ctx?.userId;
    if (!groupId || !userId) return reply.code(401).send({ error: 'unauthorized' });

    const parsed = buildSchema.safeParse(req.body);
    if (!parsed.success) {
      return reply.code(400).send({ error: 'invalid_input', details: parsed.error.flatten() });
    }
    const input = parsed.data;

    const jobId = crypto.randomUUID();
    const job: GithubBuildJobData = {
      jobId,
      repo: input.repo,
      branch: input.branch,
      buildCommand: input.buildCommand,
      artifactPath: input.artifactPath,
      packageName: input.packageName,
      version: input.version,
      groupId,
      userId,
      policyId: input.policyId,
      approve: input.approve,
      install: input.install,
      artifactUrl: input.artifactUrl,
      artifactToken: input.artifactToken,
      manifest: input.manifest
    };

    await writeStatus({
      jobId,
      status: 'queued',
      message: 'queued',
      repo: job.repo,
      branch: job.branch,
      artifactPath: job.artifactPath,
      groupId,
      userId,
      updatedAt: new Date().toISOString()
    });

    await prisma.auditLog.create({
      data: {
        groupId,
        actorUserId: userId,
        resource: 'github.build',
        action: 'enqueue',
        metadata: { jobId, repo: job.repo, packageName: job.packageName, version: job.version },
        success: true
      }
    });

    ensureGithubBuildWorker();
    await getGithubBuildQueue().add('github-build', job, { jobId });

    reply.code(202).send({ jobId, status: 'queued' });
  });

  fastify.get('/status', async (req: FastifyRequest, reply: FastifyReply) => {
    const jobId = (req.query as any).jobId as string | undefined;
    if (!jobId) return reply.code(400).send({ error: 'jobId_required' });
    const ctx = requestContext.getStore();
    const user = (req as any).user;
    const groupId = user?.groupId ?? ctx?.groupId;
    if (!groupId) return reply.code(401).send({ error: 'unauthorized' });

    const status = await readStatus(jobId);
    if (!status || status.groupId !== groupId) {
      return reply.code(404).send({ error: 'not_found' });
    }
    reply.send(status);
  });
}
