'use client';

import { getCookie } from '@/lib/cookies';
import { JobSummary } from '@/types/api';

const API_BASE = process.env.NEXT_PUBLIC_KERNEL_API;

if (!API_BASE) {
  throw new Error('NEXT_PUBLIC_KERNEL_API is required for WebSocket support');
}

export type JobUpdateMessage = {
  channel: string;
  jobId?: string;
  title?: string;
  status?: JobSummary['status'];
  message?: string;
  progress?: number;
  updatedAt?: string;
};

export type JobSocketStatus = 'idle' | 'connecting' | 'open' | 'closed' | 'error';

export interface JobSocketController {
  subscribe: (channel: string) => void;
  close: () => void;
}

function isJobUpdateMessage(value: unknown): value is JobUpdateMessage {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof (value as JobUpdateMessage).channel === 'string'
  );
}

function buildWebsocketUrl() {
  const url = new URL(API_BASE);
  url.pathname = `${url.pathname.replace(/\/$/, '')}/ws`;
  if (url.protocol === 'https:') {
    url.protocol = 'wss:';
  } else if (url.protocol === 'http:') {
    url.protocol = 'ws:';
  }
  return url.toString();
}

export function createJobSocket(
  handlers: {
    onMessage: (message: JobUpdateMessage) => void;
    onStatus: (status: JobSocketStatus) => void;
  }
): JobSocketController | null {
  if (typeof window === 'undefined') {
    handlers.onStatus('idle');
    return null;
  }

  const token = getCookie('flexi_token');
  if (!token) {
    handlers.onStatus('error');
    return null;
  }

  const wsUrl = buildWebsocketUrl();
  handlers.onStatus('connecting');
  const socket = new WebSocket(wsUrl, `Bearer ${token}`);
  const queue: string[] = [];

  socket.addEventListener('open', () => {
    handlers.onStatus('open');
    while (queue.length) {
      const next = queue.shift();
      if (next) socket.send(next);
    }
  });

  socket.addEventListener('message', (event) => {
    try {
      const parsed = JSON.parse(event.data) as unknown;
      if (isJobUpdateMessage(parsed)) {
        handlers.onMessage(parsed);
      }
    } catch (err) {
      console.error('Failed to parse WS message', err);
    }
  });

  socket.addEventListener('close', () => {
    handlers.onStatus('closed');
  });

  socket.addEventListener('error', () => {
    handlers.onStatus('error');
  });

  const sendWhenReady = (payload: unknown) => {
    const data = JSON.stringify(payload);
    if (socket.readyState === WebSocket.OPEN) {
      socket.send(data);
    } else {
      queue.push(data);
    }
  };

  return {
    subscribe(channel: string) {
      if (!channel) return;
      sendWhenReady({ type: 'subscribe', channel });
    },
    close() {
      try {
        socket.close();
      } catch (err) {
        console.warn('Failed to close websocket', err);
      }
    },
  };
}
