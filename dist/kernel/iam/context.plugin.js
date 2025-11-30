"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.contextPlugin = contextPlugin;
const db_1 = require("../../lib/db");
const request_context_1 = require("../../lib/request-context");
async function contextPlugin(fastify) {
    fastify.addHook('onRequest', async (req) => {
        const groupId = req.user?.groupId ?? null;
        const userId = req.user?.id ?? null;
        const mode = req.headers['x-flexi-mode'] === 'draft' ? 'draft' : 'stable';
        (0, request_context_1.setRequestContext)({ groupId, userId, mode });
        await (0, db_1.setRlsContext)(groupId, userId, mode);
    });
}
//# sourceMappingURL=context.plugin.js.map