"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.resolveToLock = resolveToLock;
const semver_1 = __importDefault(require("semver"));
/**
 * Resolve dependencies into a deterministic lock structure.
 * - Runtime deps are fetched and locked.
 * - Peer deps must already be present in the host; we record expected ranges.
 * - Optional deps are best-effort; failures do not break resolution.
 */
async function resolveToLock(root, fetcher, opts = {}) {
    const visited = new Set();
    const walk = async (name, manifest, integrity, resolved, path) => {
        const key = `${name}@${manifest.version}`;
        if (visited.has(key))
            return { name, version: manifest.version, integrity };
        visited.add(key);
        const nextPath = [...path, key];
        const deps = await Promise.all((manifest.dependencies ?? []).map(async (dep) => {
            const fetched = await fetcher(dep.name, dep.version);
            if (!semver_1.default.satisfies(fetched.manifest.version, dep.version)) {
                throw new Error(`Dependency version mismatch: ${dep.name} resolved ${fetched.manifest.version} !~ ${dep.version}`);
            }
            return (await walk(dep.name, fetched.manifest, fetched.integrity, fetched.resolved, nextPath));
        }));
        // peerDependencies are not fetched; we just record expected versions.
        const peerState = (manifest.peerDependencies ?? []).reduce((acc, peer) => {
            acc[peer.name] = peer.version;
            return acc;
        }, {});
        return {
            name,
            version: manifest.version,
            integrity,
            resolved,
            dependencies: deps.length ? deps : undefined,
            peerState
        };
    };
    const rootLock = (await walk(root.manifest.name, root.manifest, root.integrity, root.resolved, []));
    return rootLock;
}
//# sourceMappingURL=resolver.js.map