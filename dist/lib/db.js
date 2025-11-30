"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.prisma = void 0;
exports.setRlsContext = setRlsContext;
const client_1 = require("@prisma/client");
const logger_1 = require("./logger");
const request_context_1 = require("./request-context");
exports.prisma = new client_1.PrismaClient({
    log: ['error', 'warn']
});
// Set Postgres session variables for RLS per request.
async function setRlsContext(groupId, userId, mode = 'stable') {
    const group = groupId ?? null;
    const user = userId ?? null;
    try {
        await exports.prisma.$executeRawUnsafe("SELECT set_config('flexi.current_group', $1, true), set_config('flexi.current_user', $2, true), set_config('default_transaction_read_only', $3, true)", group, user, mode === 'draft' ? 'on' : 'off');
    }
    catch (err) {
        logger_1.logger.warn({ err, group, user }, 'failed to set RLS context');
    }
}
// Prisma middleware to enforce group scoping for multi-tenant models.
const GROUP_SCOPED_FIELDS = {
    GroupMember: 'groupId',
    Role: 'groupId',
    Permission: 'groupId',
    AppInstall: 'groupId',
    EntityRecord: 'groupId',
    ComponentInstall: 'groupId',
    PlaygroundLog: 'groupId'
};
const OWNER_SCOPED_FIELDS = {
    ComponentPackage: 'ownerGroupId'
};
exports.prisma.$use(async (params, next) => {
    const ctx = (0, request_context_1.getRequestContext)();
    const groupId = ctx?.groupId || null;
    const mode = ctx?.mode || 'stable';
    const field = GROUP_SCOPED_FIELDS[params.model ?? ''] ?? OWNER_SCOPED_FIELDS[params.model ?? ''];
    if (field) {
        if (!groupId)
            throw new Error('missing groupId in request context');
        params.args ?? (params.args = {});
        // Write operations should stamp the group
        if (['create', 'createMany', 'upsert'].includes(params.action)) {
            if (mode === 'draft' && params.model !== 'PlaygroundLog') {
                throw new Error('write_not_allowed_in_draft');
            }
            const assign = (data) => {
                if (data && typeof data === 'object')
                    data[field] = groupId;
            };
            if (Array.isArray(params.args.data))
                params.args.data.forEach(assign);
            else
                assign(params.args.data);
        }
        // Read/update/delete operations should be scoped
        const scopedActions = [
            'findUnique',
            'findFirst',
            'findMany',
            'update',
            'updateMany',
            'delete',
            'deleteMany',
            'upsert'
        ];
        if (scopedActions.includes(params.action)) {
            if (mode === 'draft' && ['update', 'updateMany', 'delete', 'deleteMany', 'upsert'].includes(params.action) && params.model !== 'PlaygroundLog') {
                throw new Error('write_not_allowed_in_draft');
            }
            params.args.where = { ...(params.args.where || {}), [field]: groupId };
        }
    }
    return next(params);
});
//# sourceMappingURL=db.js.map