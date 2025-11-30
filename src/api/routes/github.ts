import { FastifyInstance, FastifyReply, FastifyRequest } from 'fastify';
import crypto from 'crypto';
import { prisma } from '../../lib/db';
import { requestContext } from '../../lib/request-context';
import { config } from '../../config';

interface WebhookBody {
  ref?: string;
  repository?: { full_name?: string };
  head_commit?: { id?: string; message?: string };
}

interface BuildBody {
  repo: string;
  branch?: string;
  buildCommand?: string;
  artifactPath?: string;
  packageName?: string;
  version?: string;
}

export default async function githubRoutes(fastify: FastifyInstance) {
  // GitHub Webhook receiver (placeholder: signature optional)
  fastify.post('/webhook', async (req: FastifyRequest, reply: FastifyReply) => {
    const body = req.body as WebhookBody;
    const sig = req.headers['x-hub-signature-256'] as string | undefined;
    const secret = config.GITHUB_WEBHOOK_SECRET;
    if (secret && sig) {
      const payload = req.rawBody || JSON.stringify(body);
      const hmac = 'sha256=' + crypto.createHmac('sha256', secret).update(payload).digest('hex');
      if (!crypto.timingSafeEqual(Buffer.from(hmac), Buffer.from(sig))) {
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
    if (!ctx?.groupId) return reply.code(401).send({ error: 'unauthorized' });
    const body = req.body as BuildBody;
    const jobId = crypto.randomUUID();
    await prisma.auditLog.create({
      data: {
        groupId: ctx.groupId,
        actorUserId: ctx.userId ?? undefined,
        resource: 'github.build',
        action: 'enqueue',
        metadata: { jobId, ...body },
        success: true
      }
    });
    reply.code(202).send({ jobId, status: 'queued' });
  });

  fastify.get('/status', async (req: FastifyRequest, reply: FastifyReply) => {
    const jobId = (req.query as any).jobId as string | undefined;
    reply.send({ jobId, status: 'pending' });
  });
}
