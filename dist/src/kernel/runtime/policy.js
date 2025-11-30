"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.defaultPolicy = void 0;
exports.defaultPolicy = {
    memoryMb: parseInt(process.env.SANDBOX_MEMORY_MB || '128', 10),
    timeoutMs: parseInt(process.env.SANDBOX_TIMEOUT_MS || '500', 10),
    allowNetwork: false,
    allowedModules: []
};
//# sourceMappingURL=policy.js.map