"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.buildServer = buildServer;
const fastify_1 = __importDefault(require("fastify"));
const helmet_1 = __importDefault(require("@fastify/helmet"));
const rate_limit_1 = __importDefault(require("@fastify/rate-limit"));
const cors_1 = __importDefault(require("@fastify/cors"));
const config_1 = require("../config");
const context_plugin_1 = require("../kernel/iam/context.plugin");
const health_1 = __importDefault(require("./routes/health"));
const auth_1 = __importDefault(require("./routes/auth"));
const metrics_1 = __importDefault(require("./routes/metrics"));
const auth_2 = require("./hooks/auth");
const registry_1 = __importDefault(require("./routes/registry"));
const install_1 = __importDefault(require("./routes/install"));
const components_1 = __importDefault(require("./routes/components"));
const drafts_1 = __importDefault(require("./routes/drafts"));
const prisma_draft_guard_1 = require("../lib/prisma-draft-guard");
const redis_1 = require("../lib/redis");
const websocket_compat_1 = __importDefault(require("../lib/websocket-compat"));
const github_1 = __importDefault(require("./routes/github"));
const ws_1 = __importDefault(require("./routes/ws"));
const ai_1 = __importDefault(require("./routes/ai"));
const ws_bus_1 = require("../lib/ws-bus");
const queue_1 = require("../integrations/github/queue");
function buildServer() {
    // Fastify v5 requires logger to be passed as a configuration object or boolean
    const app = (0, fastify_1.default)({
        logger: {
            level: config_1.config.LOG_LEVEL || 'info'
        }
    });
    // Capture raw JSON bodies (needed for HMAC verification) while still parsing into objects.
    app.addContentTypeParser('application/json', { parseAs: 'buffer' }, (req, body, done) => {
        try {
            req.rawBody = body;
            const json = body.length ? JSON.parse(body.toString()) : {};
            done(null, json);
        }
        catch (err) {
            done(err);
        }
    });
    app.register(helmet_1.default);
    app.register(cors_1.default, { origin: true, credentials: true });
    app.register(rate_limit_1.default, {
        max: config_1.config.rateLimit.max,
        timeWindow: config_1.config.rateLimit.windowMs
    });
    // hooks should be global (not encapsulated), so register directly
    (0, auth_2.authHook)(app);
    (0, context_plugin_1.contextPlugin)(app);
    app.register(health_1.default, { prefix: '/health' });
    app.register(auth_1.default, { prefix: '/auth' });
    app.register(registry_1.default, { prefix: '/registry' });
    app.register(install_1.default, { prefix: '/' });
    app.register(components_1.default, { prefix: '/' });
    app.register(drafts_1.default, { prefix: '/' });
    app.register(ai_1.default, { prefix: '/ai' });
    app.register(websocket_compat_1.default);
    app.register(ws_1.default, { prefix: '/ws' });
    app.register(github_1.default, { prefix: '/integrations/github' });
    app.register(metrics_1.default, { prefix: '/metrics' });
    app.addHook('onReady', async () => {
        if (config_1.config.NODE_ENV !== 'test') {
            (0, queue_1.ensureGithubBuildWorker)();
        }
    });
    app.setErrorHandler((error, _req, reply) => {
        const mapped = (0, prisma_draft_guard_1.mapPrismaError)(error);
        if (mapped)
            return reply.code(mapped.status).send(mapped.body);
        reply.code(500).send({ error: 'internal_error', message: error?.message ?? 'unknown' });
    });
    app.addHook('onClose', async () => {
        await (0, queue_1.shutdownGithubBuildQueue)().catch(() => { });
        await (0, redis_1.closeRedis)().catch(() => { });
        (0, ws_bus_1.shutdownWs)();
    });
    return app;
}
//# sourceMappingURL=server.js.map