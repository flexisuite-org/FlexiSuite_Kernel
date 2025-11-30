import websocket from '@fastify/websocket';

// Patch plugin metadata to allow Fastify v5; upstream still targets v4 but is API-compatible here.
const plugin: any = (websocket as any).default ?? websocket;
const metaKey = Symbol.for('plugin-meta');
if (plugin && plugin[metaKey] && plugin[metaKey].fastify) {
  plugin[metaKey].fastify = '>=5.0.0';
}

export default plugin;
