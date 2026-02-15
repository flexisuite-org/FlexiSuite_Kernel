# Data Model (Prisma)

Core tables:
- User(id, email, passwordHash, mfaSecret?, timestamps)
- Group(id, parentId?, type, name, settings, deletedAt?)
- GroupMember(userId, groupId, roles relation)
- Role(id, name, groupId)
- Permission(id, resource, action, scope, groupId?)
- RolePermission(roleId, permissionId)
- EntityDefinition(id, appId, name, version, schema, strict)
- EntityRecord(id, definitionId, groupId, data JSONB, schemaVersion, timestamps, deletedAt?)
- EntityHistory(entityId, data, version, createdAt)
- RefreshToken(id, userId, tokenHash, familyId, deviceFingerprint?, ip?, expiresAt, revoked)
- AuditLog(id, actorUserId?, groupId?, resource, action, metadata, success, createdAt)

Tenancy:
- Logical filter on groupId; RLS SQL at prisma/rls.sql using current_setting('flexi.current_group').

Schema evolution:
- Lazy migration (transform on read/write) + optional backfill job; schemaVersion stored per record.
