import { FastifyInstance } from 'fastify';
import jwt from 'jsonwebtoken';
import { config } from '../../config';
import {
  registerWsClient,
  removeWsClient,
  subscribeWs,
  unsubscribeWs
} from '../../lib/ws-bus';
import { setRequestContext } from '../../lib/request-context';
import { setRlsContext } from '../../lib/db';

interface JwtPayload {
  userId: string;
  groupId?: string | null;
  roles?: string[];
}

function extractToken(req: any) {
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
    if (!candidate) continue;
    if (candidate.toLowerCase().startsWith('bearer ')) return candidate.slice(7).trim();
    return candidate;
  }
  return null;
}

export default async function wsRoutes(fastify: FastifyInstance) {
  const handler = (connection: any, req: any) => {
    const socket = connection as any;
    if (!socket) {
      req.raw.destroy();
      return;
    }

    const fail = (code: number, reason: string) => {
      try {
        if (socket.readyState === 1) {
          // OPEN state
          socket.close(code, reason);
        } else if (socket.readyState === 0) {
          // CONNECTING state - wait a tick for it to open
          setImmediate(() => {
            if (socket.readyState === 1) {
              socket.close(code, reason);
            } else {
              socket.terminate?.();
            }
          });
        } else {
          socket.terminate?.();
        }
      } catch {
        try {
          socket.terminate?.();
        } catch {
          /* ignore */
        }
      }
    };

    const token = extractToken(req);
    if (!token) {
      fail(1008, 'missing_token');
      return;
    }

    let payload: JwtPayload;
    try {
      payload = jwt.verify(token, config.JWT_SECRET) as JwtPayload;
    } catch {
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
    setRequestContext({ groupId: ctx.groupId, userId: ctx.userId, mode: 'stable' });
    setRlsContext(ctx.groupId, ctx.userId, 'stable').catch(() => {});

    const client = registerWsClient(socket as any, ctx);
    const send = (data: any) => socket.send(JSON.stringify(data));

    send({ type: 'ready', groupId: ctx.groupId });

    const initialJobId = (req.query as any)?.jobId as string | undefined;
    if (initialJobId) {
      subscribeWs(client, `job:${initialJobId}`);
      send({ type: 'subscribed', channel: `job:${initialJobId}` });
    }

    socket.on('message', (msg: any) => {
      let parsed: any;
      try {
        parsed = JSON.parse(msg.toString());
      } catch {
        send({ type: 'error', error: 'invalid_json' });
        return;
      }

      if (parsed?.type === 'subscribe' && typeof parsed.channel === 'string') {
        subscribeWs(client, parsed.channel);
        send({ type: 'subscribed', channel: parsed.channel });
      } else if (parsed?.type === 'subscribe' && parsed.jobId) {
        const channel = `job:${parsed.jobId}`;
        subscribeWs(client, channel);
        send({ type: 'subscribed', channel });
      } else if (parsed?.type === 'unsubscribe' && typeof parsed.channel === 'string') {
        unsubscribeWs(client, parsed.channel);
        send({ type: 'unsubscribed', channel: parsed.channel });
      } else {
        send({ type: 'error', error: 'unknown_message' });
      }
    });

    socket.on('close', () => {
      removeWsClient(client);
    });
    socket.on('error', () => {
      removeWsClient(client);
    });
  };

  fastify.get('/', { websocket: true, config: { rateLimit: false } }, handler);
}
