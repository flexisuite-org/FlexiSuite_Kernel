import Fastify from 'fastify';
import helmet from '@fastify/helmet';
import rateLimit from '@fastify/rate-limit';
import cors from '@fastify/cors';
import { config } from '../config';
import { contextPlugin } from '../kernel/iam/context.plugin';
import healthRoutes from './routes/health';
import authRoutes from './routes/auth';
import metricsRoutes from './routes/metrics';

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

  app.register(async (instance) => contextPlugin(instance));
  app.register(healthRoutes, { prefix: '/health' });
  app.register(authRoutes, { prefix: '/auth' });
  app.register(metricsRoutes, { prefix: '/metrics' });

  return app;
}
