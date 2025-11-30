"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.saveDraftResult = saveDraftResult;
const db_1 = require("./db");
// Playground storage (non-prod). Writes go to PlaygroundLog with RLS enforced.
async function saveDraftResult(groupId, userId, payload) {
    return db_1.prisma.playgroundLog.create({
        data: {
            groupId,
            userId: userId ?? undefined,
            payload
        }
    });
}
//# sourceMappingURL=playground-db.js.map