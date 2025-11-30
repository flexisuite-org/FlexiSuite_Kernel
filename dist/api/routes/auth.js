"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.default = authRoutes;
const auth_service_1 = require("../../kernel/iam/auth.service");
const zod_1 = require("zod");
async function authRoutes(fastify) {
    const signupSchema = zod_1.z.object({ email: zod_1.z.string().email(), password: zod_1.z.string().min(8) });
    const loginSchema = signupSchema;
    const refreshSchema = zod_1.z.object({ userId: zod_1.z.string(), refreshToken: zod_1.z.string(), familyId: zod_1.z.string().optional() });
    fastify.post('/signup', { config: { rateLimit: { max: 10, timeWindow: '1 minute' } } }, async (req, reply) => {
        const parsed = signupSchema.safeParse(req.body);
        if (!parsed.success)
            return reply.code(400).send({ error: 'invalid_input', details: parsed.error.flatten() });
        const { email, password } = parsed.data;
        const tokens = await auth_service_1.authService.signup(email, password);
        reply.send(tokens);
    });
    fastify.post('/login', { config: { rateLimit: { max: 20, timeWindow: '1 minute' } } }, async (req, reply) => {
        const parsed = loginSchema.safeParse(req.body);
        if (!parsed.success)
            return reply.code(400).send({ error: 'invalid_input', details: parsed.error.flatten() });
        const { email, password } = parsed.data;
        const tokens = await auth_service_1.authService.login(email, password);
        reply.send(tokens);
    });
    fastify.post('/refresh', { config: { rateLimit: { max: 30, timeWindow: '1 minute' } } }, async (req, reply) => {
        const parsed = refreshSchema.safeParse(req.body);
        if (!parsed.success)
            return reply.code(400).send({ error: 'invalid_input', details: parsed.error.flatten() });
        const { userId, refreshToken, familyId } = parsed.data;
        const tokens = await auth_service_1.authService.refresh(userId, refreshToken, familyId);
        reply.send(tokens);
    });
}
//# sourceMappingURL=auth.js.map