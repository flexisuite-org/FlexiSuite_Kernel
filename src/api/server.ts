import Fastify from 'fastify';
import helmet from '@fastify/helmet';
import rateLimit from '@fastify/rate-limit';
import cors from '@fastify/cors';
import { config } from '../config';
import { contextPlugin } from '../kernel/iam/context.plugin';
import healthRoutes from './routes/health';
import authRoutes from './routes/auth';
import metricsRoutes from './routes/metrics';
import adminRoutes from './routes/admin';
import { authHook } from './hooks/auth';
import registryRoutes from './routes/registry';
import installRoutes from './routes/install';
import componentsRoutes from './routes/components';
import draftsRoutes from './routes/drafts';
import sandboxRoutes from './routes/sandbox';
import launcherRoutes from './routes/launcher';
import { mapPrismaError } from '../lib/prisma-draft-guard';
import { closeRedis } from '../lib/redis';
import websocket from '../lib/websocket-compat';
import githubRoutes from './routes/github';
import wsRoutes from './routes/ws';
import aiRoutes from './routes/ai';
import { shutdownWs } from '../lib/ws-bus';
import { ensureGithubBuildWorker, shutdownGithubBuildQueue } from '../integrations/github/queue';
import invitesRoutes from './routes/invites';

export function buildServer() {
  // Fastify v5 requires logger to be passed as a configuration object or boolean
  const app = Fastify({
    logger: {
      level: config.LOG_LEVEL || 'info'
    }
  });

  // Capture raw JSON bodies (needed for HMAC verification) while still parsing into objects.
  app.addContentTypeParser('application/json', { parseAs: 'buffer' }, (req, body, done) => {
    try {
      (req as any).rawBody = body;
      const json = body.length ? JSON.parse(body.toString()) : {};
      done(null, json);
    } catch (err) {
      done(err as Error);
    }
  });

  app.register(helmet);
  app.register(cors, { origin: true, credentials: true });
  app.register(rateLimit, {
    max: config.rateLimit.max,
    timeWindow: config.rateLimit.windowMs
  });

  // hooks should be global (not encapsulated), so register directly
  authHook(app);
  contextPlugin(app);
  app.register(healthRoutes, { prefix: '/health' });
  app.register(authRoutes, { prefix: '/auth' });
  app.register(adminRoutes, { prefix: '/admin' });
  app.register(registryRoutes, { prefix: '/registry' });
  app.register(installRoutes, { prefix: '/' });
  app.register(componentsRoutes, { prefix: '/' });
  app.register(launcherRoutes, { prefix: '/launcher' });
  app.register(draftsRoutes, { prefix: '/' });
  app.register(sandboxRoutes, { prefix: '/sandbox' });
  app.register(aiRoutes, { prefix: '/ai' });
  app.register(websocket);
  app.register(wsRoutes, { prefix: '/ws' });
  app.register(githubRoutes, { prefix: '/integrations/github' });
  app.register(invitesRoutes, { prefix: '/invites' });
  app.register(metricsRoutes, { prefix: '/metrics' });

  app.addHook('onReady', async () => {
    if (config.NODE_ENV !== 'test') {
      ensureGithubBuildWorker();
    }
  });

  app.setErrorHandler((error: any, _req, reply) => {
    const mapped = mapPrismaError(error);
    if (mapped) return reply.code(mapped.status).send(mapped.body);
    reply.code(500).send({ error: 'internal_error', message: error?.message ?? 'unknown' });
  });

  app.addHook('onClose', async () => {
    await shutdownGithubBuildQueue().catch(() => {});
    await closeRedis().catch(() => {});
    shutdownWs();
  });

  return app;
}
