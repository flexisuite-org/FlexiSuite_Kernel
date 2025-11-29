// Component manifest / lock file types (aligns with docs/component-manifest.md)

export type DependencyKind = 'runtime' | 'peer' | 'optional';

export interface ManifestDependency {
  name: string;
  version: string; // semver range
  integrity?: string; // sha256 hex
}

export interface ComponentManifest {
  name: string; // scoped name e.g. @group/app-header
  version: string; // semver
  engine: string; // kernel API semver
  entry?: string | null; // server entry identifier
  bundle?: string | null; // client bundle URI/ID
  dependencies?: ManifestDependency[];
  peerDependencies?: ManifestDependency[];
  optionalDependencies?: ManifestDependency[];
  integrity: string; // overall payload integrity (sha256)
  policyId: string;
  executionMode?: 'API' | 'SANDBOX';
  capabilities?: string[];
  uiMount?: string;
  signature?: string; // HMAC signature of manifest JSON (optional)
}

export interface LockDependency {
  name: string;
  version: string; // resolved version
  integrity: string; // required
  dependencies?: LockDependency[];
}

export interface ComponentLock {
  name: string;
  version: string;
  integrity: string;
  resolved: string; // download URL or storage id
  dependencies?: LockDependency[];
  peerState?: Record<string, string>; // resolved peers for verification
}
