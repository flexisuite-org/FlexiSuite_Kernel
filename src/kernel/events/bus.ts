import { EventEmitter } from 'events';
import { Queue, Worker, JobsOptions } from 'bullmq';
import { v4 as uuid } from 'uuid';
import { redis } from '../../lib/redis';
import { logger } from '../../lib/logger';
import { EventMessage, EventName, EventPayloadMap } from './definitions';

const DEFAULT_QUEUE = 'flexi-events';

export class EventBus {
  private emitter = new EventEmitter();
  private queue: Queue | null = null;
  private worker: Worker | null = null;

  constructor(enableQueue = true) {
    if (enableQueue) {
      this.queue = new Queue(DEFAULT_QUEUE, { connection: redis });
      this.worker = new Worker(
        DEFAULT_QUEUE,
        async (job) => {
          const msg = job.data as EventMessage;
          this.emitter.emit(msg.name, msg);
        },
        { connection: redis }
      );

      this.worker.on('failed', (job, err) => {
        logger.error({ jobId: job?.id, err }, 'event worker failed');
      });
    }
  }

  async publish<K extends EventName>(name: K, payload: EventPayloadMap[K], opts: JobsOptions = {}) {
    const message: EventMessage<K> = {
      id: uuid(),
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

  subscribe<K extends EventName>(name: K, handler: (event: EventMessage<K>) => Promise<void> | void) {
    this.emitter.on(name, handler as any);
  }
}

export const eventBus = new EventBus(true);
