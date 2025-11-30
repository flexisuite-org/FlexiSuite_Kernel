"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.readStatus = readStatus;
exports.writeStatus = writeStatus;
exports.updateStatus = updateStatus;
exports.channelForJob = channelForJob;
const redis_1 = require("../../lib/redis");
const logger_1 = require("../../lib/logger");
const ws_bus_1 = require("../../lib/ws-bus");
const STATUS_TTL_SECONDS = 60 * 60 * 24; // keep status for 1 day
const statusKey = (jobId) => `github:job:${jobId}`;
const statusChannel = (jobId) => `job:${jobId}`;
async function readStatus(jobId) {
    const raw = await (0, redis_1.getRedis)().get(statusKey(jobId));
    if (!raw)
        return null;
    try {
        return JSON.parse(raw);
    }
    catch (err) {
        logger_1.logger.warn({ err, jobId }, 'failed to parse github job status');
        return null;
    }
}
async function writeStatus(status) {
    const redis = (0, redis_1.getRedis)();
    const payload = JSON.stringify(status);
    await redis.set(statusKey(status.jobId), payload, 'EX', STATUS_TTL_SECONDS);
    await redis.publish(statusChannel(status.jobId), payload);
    try {
        (0, ws_bus_1.publishWs)(statusChannel(status.jobId), status, { groupId: status.groupId });
    }
    catch (err) {
        logger_1.logger.warn({ err }, 'failed to publish ws status');
    }
    return status;
}
async function updateStatus(jobId, patch) {
    const current = (await readStatus(jobId)) ?? {};
    const next = {
        ...current,
        ...patch,
        jobId,
        groupId: patch.groupId ?? current.groupId ?? 'unknown',
        updatedAt: new Date().toISOString()
    };
    return writeStatus(next);
}
function channelForJob(jobId) {
    return statusChannel(jobId);
}
//# sourceMappingURL=status-store.js.map