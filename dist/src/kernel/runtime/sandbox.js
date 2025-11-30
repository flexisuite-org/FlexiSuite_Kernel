"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.sandbox = exports.Sandbox = void 0;
let ivm;
try {
    ivm = require('isolated-vm');
}
catch (err) {
    // optional dependency may be absent in CI; sandbox.run will throw if ivm unavailable
    ivm = null;
}
const policy_1 = require("./policy");
class Sandbox {
    constructor(policy = policy_1.defaultPolicy) {
        this.policy = policy;
    }
    async run(script, sandboxGlobals = {}) {
        if (!ivm) {
            throw new Error('isolated-vm not available');
        }
        const isolate = new ivm.Isolate({ memoryLimit: this.policy.memoryMb });
        const context = await isolate.createContext();
        const jail = context.global;
        await jail.set('kernel', sandboxGlobals, { copy: true });
        // Remove/poison dangerous globals (best-effort; isolated-vm starts with minimal globals already)
        const blocked = ['fetch', 'require', 'process', 'global', 'globalThis', 'Function', 'eval'];
        for (const key of blocked) {
            await jail.set(key, undefined, { copy: true });
        }
        const wrapped = `
      'use strict';
      (async () => { ${script} })();
    `;
        const compiled = await isolate.compileScript(wrapped);
        const result = await compiled.run(context, { timeout: this.policy.timeoutMs });
        return result;
    }
}
exports.Sandbox = Sandbox;
exports.sandbox = new Sandbox();
//# sourceMappingURL=sandbox.js.map