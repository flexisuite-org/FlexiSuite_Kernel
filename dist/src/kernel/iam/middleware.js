"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.requireAuth = requireAuth;
exports.authorize = authorize;
const db_1 = require("../../lib/db");
function requireAuth() {
    return async (req, reply) => {
        if (!req.user) {
            reply.code(401).send({ error: 'unauthorized' });
        }
    };
}
function authorize(resource, action) {
    return async (req, reply) => {
        if (!req.user)
            return reply.code(401).send({ error: 'unauthorized' });
        const roles = req.user.roles ?? [];
        const rolePermissions = await db_1.prisma.rolePermission.findMany({
            where: { role: { name: { in: roles }, groupId: req.user.groupId } },
            include: { permission: true }
        });
        const allowed = rolePermissions.some((rp) => {
            const p = rp.permission;
            return p.resource === resource && (p.action === action || p.action === '*');
        });
        if (!allowed)
            return reply.code(403).send({ error: 'forbidden' });
    };
}
//# sourceMappingURL=middleware.js.map