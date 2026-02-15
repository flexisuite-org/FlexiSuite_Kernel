let ivm: any;
try {
  ivm = require('isolated-vm');
} catch (err) {
  // optional dependency may be absent in CI; sandbox.run will throw if ivm unavailable
  ivm = null;
}
import { defaultPolicy, SandboxPolicy } from './policy';
import client from 'prom-client';

const sandboxExecutions = new client.Counter({
  name: 'sandbox_executions_total',
  help: 'Total number of sandbox script executions'
});

const sandboxErrors = new client.Counter({
  name: 'sandbox_errors_total',
  help: 'Total number of sandbox script execution errors'
});

export class Sandbox {
  private policy: SandboxPolicy;

  constructor(policy: SandboxPolicy = defaultPolicy) {
    this.policy = policy;
  }

  async run(script: string, sandboxGlobals: Record<string, unknown> = {}) {
    if (!ivm) {
      throw new Error('isolated-vm not available');
    }
    sandboxExecutions.inc();
    const isolate = new ivm.Isolate({ memoryLimit: this.policy.memoryMb });
    try {
      const context = await isolate.createContext();
      const jail = context.global;
      await jail.set('kernel', sandboxGlobals, { copy: true });

      // Remove/poison dangerous globals
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
    } catch (err) {
      sandboxErrors.inc();
      throw err;
    } finally {
      isolate.dispose();
    }
  }
}

export const sandbox = new Sandbox();
