export interface SandboxPolicy {
  memoryMb: number;
  timeoutMs: number;
  allowNetwork: boolean;
  allowedModules: string[];
}

export const defaultPolicy: SandboxPolicy = {
  memoryMb: parseInt(process.env.SANDBOX_MEMORY_MB || '128', 10),
  timeoutMs: parseInt(process.env.SANDBOX_TIMEOUT_MS || '500', 10),
  allowNetwork: false,
  allowedModules: []
};
