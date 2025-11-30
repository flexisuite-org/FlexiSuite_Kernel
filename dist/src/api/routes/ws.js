"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.default = wsRoutes;
const jsonwebtoken_1 = __importDefault(require("jsonwebtoken"));
const config_1 = require("../../config");
const ws_bus_1 = require("../../lib/ws-bus");
const request_context_1 = require("../../lib/request-context");
const db_1 = require("../../lib/db");
function extractToken(req) {
    const auth = req.headers['authorization'];
    if (auth && typeof auth === 'string' && auth.toLowerCase().startsWith('bearer ')) {
        return auth.slice(7);
    }
    const protocolHeader = req.headers['sec-websocket-protocol'];
    const protocols = Array.isArray(protocolHeader)
        ? protocolHeader
        : typeof protocolHeader === 'string'
            ? protocolHeader.split(',')
            : [];
    for (const raw of protocols) {
        const candidate = raw.trim();
        if (!candidate)
            continue;
        if (candidate.toLowerCase().startsWith('bearer '))
            return candidate.slice(7).trim();
        return candidate;
    }
    return null;
}
async function wsRoutes(fastify) {
    const handler = (connection, req) => {
        const socket = connection;
        if (!socket) {
            req.raw.destroy();
            return;
        }
        const fail = (code, reason) => {
            try {
                if (socket.readyState === 1) {
                    // OPEN state
                    socket.close(code, reason);
                }
                else if (socket.readyState === 0) {
                    // CONNECTING state - wait a tick for it to open
                    setImmediate(() => {
                        if (socket.readyState === 1) {
                            socket.close(code, reason);
                        }
                        else {
                            socket.terminate?.();
                        }
                    });
                }
                else {
                    socket.terminate?.();
                }
            }
            catch {
                try {
                    socket.terminate?.();
                }
                catch {
                    /* ignore */
                }
            }
        };
        const token = extractToken(req);
        if (!token) {
            fail(1008, 'missing_token');
            return;
        }
        let payload;
        try {
            payload = jsonwebtoken_1.default.verify(token, config_1.config.JWT_SECRET);
        }
        catch {
            fail(1008, 'invalid_token');
            return;
        }
        if (!payload.groupId || !payload.userId) {
            fail(1008, 'missing_claims');
            return;
        }
        const groupId = payload.groupId;
        const userId = payload.userId;
        const ctx = { userId, groupId, roles: payload.roles ?? [] };
        (0, request_context_1.setRequestContext)({ groupId: ctx.groupId, userId: ctx.userId, mode: 'stable' });
        (0, db_1.setRlsContext)(ctx.groupId, ctx.userId, 'stable').catch(() => { });
        const client = (0, ws_bus_1.registerWsClient)(socket, ctx);
        const send = (data) => socket.send(JSON.stringify(data));
        send({ type: 'ready', groupId: ctx.groupId });
        const initialJobId = req.query?.jobId;
        if (initialJobId) {
            (0, ws_bus_1.subscribeWs)(client, `job:${initialJobId}`);
            send({ type: 'subscribed', channel: `job:${initialJobId}` });
        }
        socket.on('message', (msg) => {
            let parsed;
            try {
                parsed = JSON.parse(msg.toString());
            }
            catch {
                send({ type: 'error', error: 'invalid_json' });
                return;
            }
            if (parsed?.type === 'subscribe' && typeof parsed.channel === 'string') {
                (0, ws_bus_1.subscribeWs)(client, parsed.channel);
                send({ type: 'subscribed', channel: parsed.channel });
            }
            else if (parsed?.type === 'subscribe' && parsed.jobId) {
                const channel = `job:${parsed.jobId}`;
                (0, ws_bus_1.subscribeWs)(client, channel);
                send({ type: 'subscribed', channel });
            }
            else if (parsed?.type === 'unsubscribe' && typeof parsed.channel === 'string') {
                (0, ws_bus_1.unsubscribeWs)(client, parsed.channel);
                send({ type: 'unsubscribed', channel: parsed.channel });
            }
            else {
                send({ type: 'error', error: 'unknown_message' });
            }
        });
        socket.on('close', () => {
            (0, ws_bus_1.removeWsClient)(client);
        });
        socket.on('error', () => {
            (0, ws_bus_1.removeWsClient)(client);
        });
    };
    fastify.get('/', { websocket: true, config: { rateLimit: false } }, handler);
}
//# sourceMappingURL=ws.js.map