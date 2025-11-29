import ivm from 'isolated-vm';
import { defaultPolicy, SandboxPolicy } from './policy';

export class Sandbox {
  private policy: SandboxPolicy;

  constructor(policy: SandboxPolicy = defaultPolicy) {
    this.policy = policy;
  }

  async run(script: string, sandboxGlobals: Record<string, unknown> = {}) {
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

export const sandbox = new Sandbox();
