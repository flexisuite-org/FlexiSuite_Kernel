"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.authHook = authHook;
const jsonwebtoken_1 = __importDefault(require("jsonwebtoken"));
const config_1 = require("../../config");
async function authHook(fastify) {
    fastify.addHook('onRequest', async (req, reply) => {
        const auth = req.headers.authorization;
        if (!auth || !auth.startsWith('Bearer '))
            return;
        const token = auth.slice('Bearer '.length);
        try {
            const payload = jsonwebtoken_1.default.verify(token, config_1.config.JWT_SECRET);
            req.user = {
                id: payload.userId,
                groupId: payload.groupId ?? null,
                roles: payload.roles ?? []
            };
        }
        catch (err) {
            // invalid token -> clear user and proceed; protected routes should still reject
            req.user = undefined;
            reply.header('WWW-Authenticate', 'Bearer error="invalid_token"');
        }
    });
}
//# sourceMappingURL=auth.js.map