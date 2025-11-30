import { prisma } from '../../src/lib/db';
import { setRequestContext } from '../../src/lib/request-context';
import { hashJson } from '../../src/lib/integrity';

export async function truncateAll() {
  await prisma.$executeRawUnsafe(`
    TRUNCATE "PlaygroundLog",
            "ComponentInstall",
            "ComponentDependency",
            "ComponentPackage",
            "ComponentPolicy",
            "EntityHistory",
            "EntityRecord",
            "EntityDefinition",
            "AppInstall",
            "App",
            "RolePermission",
            "Permission",
            "Role",
            "GroupMember",
            "RefreshToken",
            "AuditLog",
            "Group",
            "User"
    RESTART IDENTITY CASCADE;
  `);
}

export async function createTenantSeed(suffix: string) {
  const group = await prisma.group.create({ data: { name: `G-${suffix}`, type: 'ORG' } });
  const user = await prisma.user.create({ data: { email: `user+${suffix}@example.com`, passwordHash: 'x' } });
  return { groupId: group.id, userId: user.id };
}

export async function createPolicy(name = 'default-policy') {
  const existing = await prisma.componentPolicy.findFirst({ where: { name } });
  if (existing) return existing.id;
  return (await prisma.componentPolicy.create({ data: { name } })).id;
}

export async function createPackage(opts: {
  name: string;
  version: string;
  groupId: string;
  userId: string;
  status?: 'DRAFT' | 'APPROVED';
  capabilities?: string[];
}) {
  const manifest = {
    name: opts.name,
    version: opts.version,
    engine: '1.0.0',
    capabilities: opts.capabilities ?? ['echo']
  };
  const integrity = hashJson(manifest);
  const policyId = await createPolicy();
  setRequestContext({ groupId: opts.groupId, userId: opts.userId });
  return prisma.componentPackage.create({
    data: {
      name: manifest.name,
      version: manifest.version,
      status: opts.status ?? 'APPROVED',
      integrityHash: integrity,
      manifest,
      policyId,
      ownerGroupId: opts.groupId,
      createdById: opts.userId
    }
  });
}
