"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.isDraftWriteNotAllowed = isDraftWriteNotAllowed;
exports.mapPrismaError = mapPrismaError;
const client_1 = require("@prisma/client");
function isDraftWriteNotAllowed(err) {
    return err instanceof Error && err.message === 'write_not_allowed_in_draft';
}
function mapPrismaError(err) {
    if (isDraftWriteNotAllowed(err)) {
        return { status: 403, body: { error: 'write_not_allowed_in_draft' } };
    }
    if (err instanceof client_1.Prisma.PrismaClientKnownRequestError) {
        return { status: 400, body: { error: err.code, meta: err.meta } };
    }
    return null;
}
//# sourceMappingURL=prisma-draft-guard.js.map