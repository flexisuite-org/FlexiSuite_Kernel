import { FastifyInstance } from 'fastify';
import client from 'prom-client';

export default async function metricsRoutes(fastify: FastifyInstance) {
  // Collect default metrics (CPU, Memory, etc.)
  client.collectDefaultMetrics();

  // Custom metrics
  const httpRequestDurationMicroseconds = new client.Histogram({
    name: 'http_request_duration_seconds',
    help: 'Duration of HTTP requests in seconds',
    labelNames: ['method', 'route', 'status_code'],
    buckets: [0.1, 0.3, 0.5, 0.7, 1, 3, 5, 7, 10]
  });

  // Hook to measure request duration
  fastify.addHook('onResponse', async (request, reply) => {
    if (request.routeOptions.config.url) {
      httpRequestDurationMicroseconds.observe(
        {
          method: request.method,
          route: request.routeOptions.config.url,
          status_code: reply.statusCode
        },
        reply.elapsedTime / 1000
      );
    }
  });

  fastify.get('/', async (_req, reply) => {
    reply.header('Content-Type', client.register.contentType);
    return client.register.metrics();
  });
}