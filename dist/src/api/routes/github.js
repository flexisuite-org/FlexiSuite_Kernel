"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.default = githubRoutes;
const crypto_1 = __importDefault(require("crypto"));
const zod_1 = require("zod");
const db_1 = require("../../lib/db");
const request_context_1 = require("../../lib/request-context");
const config_1 = require("../../config");
const queue_1 = require("../../integrations/github/queue");
const status_store_1 = require("../../integrations/github/status-store");
const semver_1 = __importDefault(require("semver"));
const buildSchema = zod_1.z.object({
    repo: zod_1.z.string().min(1),
    branch: zod_1.z.string().default('main'),
    buildCommand: zod_1.z.string().default('npm ci && npm run build'),
    artifactPath: zod_1.z.string().default('dist'),
    packageName: zod_1.z.string().min(1),
    version: zod_1.z.string().min(1).refine((v) => !!semver_1.default.valid(v), { message: 'invalid_semver' }),
    policyId: zod_1.z.string().optional(),
    approve: zod_1.z.boolean().optional(),
    install: zod_1.z.boolean().optional()
});
async function githubRoutes(fastify) {
    // GitHub Webhook receiver (signature verification if secret configured)
    fastify.post('/webhook', async (req, reply) => {
        const body = req.body;
        const sig = req.headers['x-hub-signature-256'];
        const secret = config_1.config.GITHUB_WEBHOOK_SECRET;
        if (secret) {
            if (!sig)
                return reply.code(401).send({ error: 'missing_signature' });
            const raw = req.rawBody || JSON.stringify(body);
            const expected = 'sha256=' + crypto_1.default.createHmac('sha256', secret).update(raw).digest('hex');
            try {
                if (!crypto_1.default.timingSafeEqual(Buffer.from(expected), Buffer.from(sig))) {
                    return reply.code(401).send({ error: 'invalid_signature' });
                }
            }
            catch {
                return reply.code(401).send({ error: 'invalid_signature' });
            }
        }
        await db_1.prisma.auditLog.create({
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
    fastify.post('/build', async (req, reply) => {
        const ctx = request_context_1.requestContext.getStore();
        const user = req.user;
        const groupId = user?.groupId ?? ctx?.groupId;
        const userId = user?.id ?? ctx?.userId;
        if (!groupId || !userId)
            return reply.code(401).send({ error: 'unauthorized' });
        const parsed = buildSchema.safeParse(req.body);
        if (!parsed.success) {
            return reply.code(400).send({ error: 'invalid_input', details: parsed.error.flatten() });
        }
        const input = parsed.data;
        const jobId = crypto_1.default.randomUUID();
        const job = {
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
            install: input.install
        };
        await (0, status_store_1.writeStatus)({
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
        await db_1.prisma.auditLog.create({
            data: {
                groupId,
                actorUserId: userId,
                resource: 'github.build',
                action: 'enqueue',
                metadata: { jobId, repo: job.repo, packageName: job.packageName, version: job.version },
                success: true
            }
        });
        (0, queue_1.ensureGithubBuildWorker)();
        await (0, queue_1.getGithubBuildQueue)().add('github-build', job, { jobId });
        reply.code(202).send({ jobId, status: 'queued' });
    });
    fastify.get('/status', async (req, reply) => {
        const jobId = req.query.jobId;
        if (!jobId)
            return reply.code(400).send({ error: 'jobId_required' });
        const ctx = request_context_1.requestContext.getStore();
        const user = req.user;
        const groupId = user?.groupId ?? ctx?.groupId;
        if (!groupId)
            return reply.code(401).send({ error: 'unauthorized' });
        const status = await (0, status_store_1.readStatus)(jobId);
        if (!status || status.groupId !== groupId) {
            return reply.code(404).send({ error: 'not_found' });
        }
        reply.send(status);
    });
}
//# sourceMappingURL=github.js.map