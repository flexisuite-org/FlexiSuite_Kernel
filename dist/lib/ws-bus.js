"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.dispatchLocalWs = dispatchLocalWs;
exports.useWsAdapter = useWsAdapter;
exports.publishWs = publishWs;
exports.registerWsClient = registerWsClient;
exports.subscribeWs = subscribeWs;
exports.unsubscribeWs = unsubscribeWs;
exports.removeWsClient = removeWsClient;
exports.shutdownWs = shutdownWs;
const logger_1 = require("./logger");
function isSocketOpen(socket) {
    const openState = typeof socket.OPEN === 'number' ? socket.OPEN : 1;
    return socket.readyState === openState;
}
class WsHub {
    constructor() {
        this.channels = new Map();
        this.clients = new Set();
    }
    addClient(socket, context) {
        const client = { socket, context, subscriptions: new Set() };
        this.clients.add(client);
        return client;
    }
    subscribe(client, channel) {
        if (!channel)
            return;
        client.subscriptions.add(channel);
        let set = this.channels.get(channel);
        if (!set) {
            set = new Set();
            this.channels.set(channel, set);
        }
        set.add(client);
    }
    unsubscribe(client, channel) {
        const set = this.channels.get(channel);
        if (set) {
            set.delete(client);
            if (!set.size)
                this.channels.delete(channel);
        }
        client.subscriptions.delete(channel);
    }
    removeClient(client) {
        if (!this.clients.delete(client))
            return;
        for (const channel of client.subscriptions) {
            const set = this.channels.get(channel);
            if (set) {
                set.delete(client);
                if (!set.size)
                    this.channels.delete(channel);
            }
        }
        if (client.heartbeat)
            clearInterval(client.heartbeat);
    }
    startHeartbeat(client, intervalMs = 30000) {
        if (client.heartbeat)
            clearInterval(client.heartbeat);
        client.heartbeat = setInterval(() => {
            if (!isSocketOpen(client.socket)) {
                this.removeClient(client);
                return;
            }
            try {
                // ws exposes ping; browsers do not. If missing, fall back to a small noop message.
                if (typeof client.socket.ping === 'function')
                    client.socket.ping();
                else
                    client.socket.send(JSON.stringify({ type: 'ping', ts: Date.now() }));
            }
            catch (err) {
                logger_1.logger.warn({ err }, 'ws heartbeat failed');
                this.removeClient(client);
            }
        }, intervalMs);
        if (typeof client.heartbeat.unref === 'function')
            client.heartbeat.unref();
    }
    broadcast(event) {
        const listeners = this.channels.get(event.channel);
        if (!listeners?.size)
            return;
        const base = event.payload && typeof event.payload === 'object'
            ? { channel: event.channel, ...event.payload }
            : { channel: event.channel, payload: event.payload };
        const message = JSON.stringify(base);
        for (const client of listeners) {
            if (client.context.groupId !== event.groupId)
                continue;
            if (!isSocketOpen(client.socket)) {
                this.removeClient(client);
                continue;
            }
            try {
                client.socket.send(message);
            }
            catch (err) {
                logger_1.logger.warn({ err }, 'ws send failed');
                this.removeClient(client);
            }
        }
    }
    shutdown() {
        for (const client of Array.from(this.clients)) {
            try {
                client.socket.close(1001, 'server_shutdown');
            }
            catch {
                /* ignore */
            }
            if (client.heartbeat)
                clearInterval(client.heartbeat);
        }
        this.channels.clear();
        this.clients.clear();
    }
}
const hub = new WsHub();
function dispatchLocalWs(event) {
    hub.broadcast(event);
}
const localAdapter = {
    publish: dispatchLocalWs
};
let adapter = localAdapter;
function useWsAdapter(next) {
    adapter = next;
}
function publishWs(channel, payload, options = {}) {
    const groupId = options.groupId ?? (payload && typeof payload === 'object' ? payload.groupId : undefined);
    if (!groupId)
        throw new Error('publishWs requires groupId');
    return adapter.publish({ channel, payload, groupId });
}
function registerWsClient(socket, context) {
    const client = hub.addClient(socket, context);
    hub.startHeartbeat(client);
    return client;
}
function subscribeWs(client, channel) {
    hub.subscribe(client, channel);
}
function unsubscribeWs(client, channel) {
    hub.unsubscribe(client, channel);
}
function removeWsClient(client) {
    hub.removeClient(client);
}
function shutdownWs() {
    hub.shutdown();
    if (adapter.shutdown)
        adapter.shutdown();
}
//# sourceMappingURL=ws-bus.js.map