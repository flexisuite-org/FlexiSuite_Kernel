import { FastifyInstance } from 'fastify';

export default async function wsRoutes(fastify: FastifyInstance) {
  fastify.get('/', { websocket: true }, (connection, req) => {
    const user = (req as any).user;
    if (!user?.groupId) {
      connection.socket.close(1008, 'unauthorized');
      return;
    }
    connection.socket.send(JSON.stringify({ status: 'connected', groupId: user.groupId }));

    connection.socket.on('message', (msg) => {
      connection.socket.send(JSON.stringify({ echo: msg.toString() }));
    });
  });
}
