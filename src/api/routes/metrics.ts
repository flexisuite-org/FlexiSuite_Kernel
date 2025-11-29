import { FastifyInstance } from 'fastify';
import client from 'prom-client';

const collectDefaultMetrics = client.collectDefaultMetrics;
collectDefaultMetrics();

export default async function metricsRoutes(fastify: FastifyInstance) {
  fastify.get('/', async (req, reply) => {
    reply.type('text/plain');
    return client.register.metrics();
  });
}
