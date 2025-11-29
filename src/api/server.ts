import Fastify from 'fastify';
import helmet from '@fastify/helmet';
import rateLimit from '@fastify/rate-limit';
import cors from '@fastify/cors';
import { config } from '../config';
import { contextPlugin } from '../kernel/iam/context.plugin';
import healthRoutes from './routes/health';
import authRoutes from './routes/auth';
import metricsRoutes from './routes/metrics';
import { authHook } from './hooks/auth';
import registryRoutes from './routes/registry';
import installRoutes from './routes/install';
import componentsRoutes from './routes/components';
import draftsRoutes from './routes/drafts';

export function buildServer() {
  // Fastify v5 requires logger to be passed as a configuration object or boolean
  const app = Fastify({
    logger: {
      level: config.NODE_ENV === 'development' ? 'info' : 'error'
    }
  });

  app.register(helmet);
  app.register(cors, { origin: true, credentials: true });
  app.register(rateLimit, {
    max: config.rateLimit.max,
    timeWindow: config.rateLimit.windowMs
  });

  app.register(async (instance) => authHook(instance));
  app.register(async (instance) => contextPlugin(instance));
  app.register(healthRoutes, { prefix: '/health' });
  app.register(authRoutes, { prefix: '/auth' });
  app.register(registryRoutes, { prefix: '/registry' });
  app.register(installRoutes, { prefix: '/' });
  app.register(componentsRoutes, { prefix: '/' });
  app.register(draftsRoutes, { prefix: '/' });
  app.register(metricsRoutes, { prefix: '/metrics' });

  return app;
}
