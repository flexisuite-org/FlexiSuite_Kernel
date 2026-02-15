import semver from 'semver';
import { ComponentManifest, ComponentLock, DependencyKind, LockDependency, ManifestDependency } from './types';

export interface FetchManifestResult {
  manifest: ComponentManifest;
  integrity: string; // sha256 hex for payload
  resolved: string; // download URL or storage id
}

export type ManifestFetcher = (name: string, range: string) => Promise<FetchManifestResult>;

export interface ResolveOptions {
  onCycleDetect?: (chain: string[]) => void;
  allowPeerWarning?: boolean;
}

/**
 * Resolve dependencies into a deterministic lock structure.
 * - Runtime deps are fetched and locked.
 * - Peer deps must already be present in the host; we record expected ranges.
 * - Optional deps are best-effort; failures do not break resolution.
 */
export async function resolveToLock(
  root: FetchManifestResult,
  fetcher: ManifestFetcher,
  opts: ResolveOptions = {}
): Promise<ComponentLock> {
  const visited = new Set<string>();

  const walk = async (
    name: string,
    manifest: ComponentManifest,
    integrity: string,
    resolved: string,
    path: string[]
  ): Promise<LockDependency | ComponentLock> => {
    const key = `${name}@${manifest.version}`;
    if (visited.has(key)) return { name, version: manifest.version, integrity } as LockDependency;
    visited.add(key);

    const nextPath = [...path, key];
    const deps = await Promise.all(
      (manifest.dependencies ?? []).map(async (dep) => {
        const fetched = await fetcher(dep.name, dep.version);
        if (!semver.satisfies(fetched.manifest.version, dep.version)) {
          throw new Error(`Dependency version mismatch: ${dep.name} resolved ${fetched.manifest.version} !~ ${dep.version}`);
        }
        return (await walk(dep.name, fetched.manifest, fetched.integrity, fetched.resolved, nextPath)) as LockDependency;
      })
    );

    // peerDependencies are not fetched; we just record expected versions.
    const peerState: Record<string, string> | undefined = (manifest.peerDependencies ?? []).reduce((acc, peer) => {
      acc[peer.name] = peer.version;
      return acc;
    }, {} as Record<string, string>);

    return {
      name,
      version: manifest.version,
      integrity,
      resolved,
      dependencies: deps.length ? deps : undefined,
      peerState
    } as LockDependency;
  };

  const rootLock = (await walk(root.manifest.name, root.manifest, root.integrity, root.resolved, [])) as ComponentLock;
  return rootLock;
}
