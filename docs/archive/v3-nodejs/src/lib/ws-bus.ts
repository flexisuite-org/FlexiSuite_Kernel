import { logger } from './logger';

export interface WsConnectionContext {
  userId: string;
  groupId: string;
  roles: string[];
}

export interface WsEvent<T = any> {
  channel: string;
  payload: T;
  groupId: string;
}

export interface WsAdapter {
  publish: (event: WsEvent) => Promise<void> | void;
  shutdown?: () => Promise<void> | void;
}

type SocketLike = {
  readyState: number;
  OPEN?: number;
  send: (data: string) => void;
  ping?: () => void;
  close: (code?: number, reason?: string) => void;
};

function isSocketOpen(socket: SocketLike) {
  const openState = typeof socket.OPEN === 'number' ? socket.OPEN : 1;
  return socket.readyState === openState;
}

interface WsClient {
  socket: SocketLike;
  context: WsConnectionContext;
  subscriptions: Set<string>;
  heartbeat?: NodeJS.Timeout;
}

class WsHub {
  private channels = new Map<string, Set<WsClient>>();
  private clients = new Set<WsClient>();

  addClient(socket: SocketLike, context: WsConnectionContext) {
    const client: WsClient = { socket, context, subscriptions: new Set() };
    this.clients.add(client);
    return client;
  }

  subscribe(client: WsClient, channel: string) {
    if (!channel) return;
    client.subscriptions.add(channel);
    let set = this.channels.get(channel);
    if (!set) {
      set = new Set();
      this.channels.set(channel, set);
    }
    set.add(client);
  }

  unsubscribe(client: WsClient, channel: string) {
    const set = this.channels.get(channel);
    if (set) {
      set.delete(client);
      if (!set.size) this.channels.delete(channel);
    }
    client.subscriptions.delete(channel);
  }

  removeClient(client: WsClient) {
    if (!this.clients.delete(client)) return;
    for (const channel of client.subscriptions) {
      const set = this.channels.get(channel);
      if (set) {
        set.delete(client);
        if (!set.size) this.channels.delete(channel);
      }
    }
    if (client.heartbeat) clearInterval(client.heartbeat);
  }

  startHeartbeat(client: WsClient, intervalMs = 30000) {
    if (client.heartbeat) clearInterval(client.heartbeat);
    client.heartbeat = setInterval(() => {
      if (!isSocketOpen(client.socket)) {
        this.removeClient(client);
        return;
      }
      try {
        // ws exposes ping; browsers do not. If missing, fall back to a small noop message.
        if (typeof client.socket.ping === 'function') client.socket.ping();
        else client.socket.send(JSON.stringify({ type: 'ping', ts: Date.now() }));
      } catch (err) {
        logger.warn({ err }, 'ws heartbeat failed');
        this.removeClient(client);
      }
    }, intervalMs);
    if (typeof client.heartbeat.unref === 'function') client.heartbeat.unref();
  }

  broadcast(event: WsEvent) {
    const listeners = this.channels.get(event.channel);
    if (!listeners?.size) return;
    const base =
      event.payload && typeof event.payload === 'object'
        ? { channel: event.channel, ...(event.payload as Record<string, unknown>) }
        : { channel: event.channel, payload: event.payload };
    const message = JSON.stringify(base);
    for (const client of listeners) {
      if (client.context.groupId !== event.groupId) continue;
      if (!isSocketOpen(client.socket)) {
        this.removeClient(client);
        continue;
      }
      try {
        client.socket.send(message);
      } catch (err) {
        logger.warn({ err }, 'ws send failed');
        this.removeClient(client);
      }
    }
  }

  shutdown() {
    for (const client of Array.from(this.clients)) {
      try {
        client.socket.close(1001, 'server_shutdown');
      } catch {
        /* ignore */
      }
      if (client.heartbeat) clearInterval(client.heartbeat);
    }
    this.channels.clear();
    this.clients.clear();
  }
}

const hub = new WsHub();

export function dispatchLocalWs(event: WsEvent) {
  hub.broadcast(event);
}

const localAdapter: WsAdapter = {
  publish: dispatchLocalWs
};

let adapter: WsAdapter = localAdapter;

export function useWsAdapter(next: WsAdapter) {
  adapter = next;
}

export function publishWs<T = any>(channel: string, payload: T, options: { groupId?: string } = {}) {
  const groupId =
    options.groupId ?? (payload && typeof payload === 'object' ? (payload as any).groupId : undefined);
  if (!groupId) throw new Error('publishWs requires groupId');
  return adapter.publish({ channel, payload, groupId });
}

export function registerWsClient(socket: SocketLike, context: WsConnectionContext) {
  const client = hub.addClient(socket, context);
  hub.startHeartbeat(client);
  return client;
}

export function subscribeWs(client: WsClient, channel: string) {
  hub.subscribe(client, channel);
}

export function unsubscribeWs(client: WsClient, channel: string) {
  hub.unsubscribe(client, channel);
}

export function removeWsClient(client: WsClient) {
  hub.removeClient(client);
}

export function shutdownWs() {
  hub.shutdown();
  if (adapter.shutdown) adapter.shutdown();
}

export type { WsClient };
