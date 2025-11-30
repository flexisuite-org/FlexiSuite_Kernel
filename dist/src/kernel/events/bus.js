"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.eventBus = exports.EventBus = void 0;
const events_1 = require("events");
const bullmq_1 = require("bullmq");
const uuid_1 = require("uuid");
const redis_1 = require("../../lib/redis");
const logger_1 = require("../../lib/logger");
const DEFAULT_QUEUE = 'flexi-events';
class EventBus {
    constructor(enableQueue = true) {
        this.emitter = new events_1.EventEmitter();
        this.queue = null;
        this.worker = null;
        if (enableQueue) {
            this.queue = new bullmq_1.Queue(DEFAULT_QUEUE, { connection: redis_1.redis });
            this.worker = new bullmq_1.Worker(DEFAULT_QUEUE, async (job) => {
                const msg = job.data;
                this.emitter.emit(msg.name, msg);
            }, { connection: redis_1.redis });
            this.worker.on('failed', (job, err) => {
                logger_1.logger.error({ jobId: job?.id, err }, 'event worker failed');
            });
        }
    }
    async publish(name, payload, opts = {}) {
        const message = {
            id: (0, uuid_1.v4)(),
            name,
            payload,
            occurredAt: new Date()
        };
        // Fire locally for in-process consumers
        this.emitter.emit(name, message);
        if (this.queue) {
            await this.queue.add(name, message, {
                attempts: opts.attempts ?? 5,
                backoff: { type: 'exponential', delay: 500 },
                removeOnComplete: true,
                removeOnFail: false,
                ...opts
            });
        }
    }
    subscribe(name, handler) {
        this.emitter.on(name, handler);
    }
}
exports.EventBus = EventBus;
exports.eventBus = new EventBus(true);
//# sourceMappingURL=bus.js.map