"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.getGithubBuildQueue = getGithubBuildQueue;
exports.ensureGithubBuildWorker = ensureGithubBuildWorker;
exports.shutdownGithubBuildQueue = shutdownGithubBuildQueue;
const bullmq_1 = require("bullmq");
const ioredis_1 = __importDefault(require("ioredis"));
const config_1 = require("../../config");
const logger_1 = require("../../lib/logger");
const runner_1 = require("./runner");
const status_store_1 = require("./status-store");
let connection = null;
let queue = null;
let worker = null;
function getConnection() {
    if (!connection) {
        connection = new ioredis_1.default(config_1.config.REDIS_URL, { maxRetriesPerRequest: null });
        connection.on('error', (err) => logger_1.logger.error({ err }, 'redis connection error (github queue)'));
    }
    return connection;
}
function getGithubBuildQueue() {
    if (!queue) {
        queue = new bullmq_1.Queue('github-build', {
            connection: getConnection(),
            defaultJobOptions: { removeOnComplete: true, attempts: 1 }
        });
    }
    return queue;
}
function ensureGithubBuildWorker() {
    if (worker)
        return worker;
    worker = new bullmq_1.Worker('github-build', runner_1.processGithubBuildJob, {
        connection: getConnection(),
        concurrency: 1
    });
    worker.on('failed', async (job, err) => {
        if (!job)
            return;
        await (0, status_store_1.updateStatus)(job.data.jobId, {
            status: 'failed',
            error: err?.message || 'job_failed',
            message: 'failed'
        });
    });
    return worker;
}
async function shutdownGithubBuildQueue() {
    if (worker) {
        await worker.close().catch(() => { });
        worker = null;
    }
    if (queue) {
        await queue.close().catch(() => { });
        queue = null;
    }
    if (connection) {
        await connection.quit().catch(() => { });
        connection = null;
    }
}
//# sourceMappingURL=queue.js.map