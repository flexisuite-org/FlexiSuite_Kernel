# Security Patterns & Best Practices

This document details security implementation patterns for FlexiSuite Kernel.

## Table of Contents
- [Authentication](#authentication)
- [Authorization](#authorization)
- [Multi-Tenancy Isolation](#multi-tenancy-isolation)
- [Session Management](#session-management)
- [Secrets Management](#secrets-management)
- [Audit Logging](#audit-logging)

---

## Authentication

### Password Hashing

**Always use Argon2id** for password hashing.

```typescript
import * as argon2 from 'argon2';

// Hashing
const hash = await argon2.hash(password, {
  type: argon2.argon2id,
  memoryCost: 65536,  // 64 MiB
  timeCost: 3,        // iterations
  parallelism: 4,     // threads
});

// Verification
const valid = await argon2.verify(hash, password);
```

**Password Requirements:**
- Minimum 12 characters
- Must include: uppercase, lowercase, number, special character
- Check against common password lists (e.g., haveibeenpwned API)

```typescript
import { z } from 'zod';

const passwordSchema = z.string()
  .min(12, 'Password must be at least 12 characters')
  .regex(/[A-Z]/, 'Must contain uppercase letter')
  .regex(/[a-z]/, 'Must contain lowercase letter')
  .regex(/[0-9]/, 'Must contain number')
  .regex(/[^A-Za-z0-9]/, 'Must contain special character');
```

### Multi-Factor Authentication (MFA)

**TOTP (Time-based One-Time Password)** for admin actions.

```typescript
import * as speakeasy from 'speakeasy';

// Generate secret
const secret = speakeasy.generateSecret({
  name: `FlexiSuite (${user.email})`,
  issuer: 'FlexiSuite',
});

// Store secret.base32 in User.mfa_secret

// Verify token
const verified = speakeasy.totp.verify({
  secret: user.mfaSecret,
  encoding: 'base32',
  token: userProvidedToken,
  window: 1, // Allow 30s clock drift
});
```

---

## Session Management

### JWT Token Structure

**Access Token** (Short-lived: 15 minutes)

```typescript
import jwt from 'jsonwebtoken';
import { v4 as uuidv4 } from 'uuid';

interface AccessTokenPayload {
  userId: string;
  groupId: string;
  roles: string[];
  iat: number;
  exp: number;
  jti: string; // JWT ID for revocation
}

const accessToken = jwt.sign(
  {
    userId: user.id,
    groupId: currentGroup.id,
    roles: user.roles.map(r => r.name),
  },
  process.env.JWT_SECRET!,
  {
    expiresIn: '15m',
    algorithm: 'HS256',
    jwtid: uuidv4(),
  }
);
```

**Refresh Token** (Long-lived: 7 days)

```typescript
interface RefreshTokenData {
  userId: string;
  familyId: string;      // For rotation tracking
  deviceFingerprint: string;
  ip: string;
  expiresAt: Date;
}

async function createRefreshToken(data: RefreshTokenData): Promise<string> {
  const token = uuidv4();
  const tokenHash = await argon2.hash(token);

  await prisma.refreshToken.create({
    data: {
      userId: data.userId,
      tokenHash,
      familyId: data.familyId,
      deviceFingerprint: data.deviceFingerprint,
      ip: data.ip,
      expiresAt: new Date(Date.now() + 7 * 24 * 60 * 60 * 1000),
      revoked: false,
    },
  });

  return token;
}
```

### Token Rotation & Reuse Detection

**Critical**: Detect and revoke tokens when reuse is attempted.

```typescript
async function refreshAccessToken(
  refreshToken: string,
  deviceFingerprint: string,
  ip: string
): Promise<{ accessToken: string; refreshToken: string }> {
  // Find token in database
  const tokenRecord = await prisma.refreshToken.findFirst({
    where: {
      tokenHash: await argon2.hash(refreshToken),
      revoked: false,
      expiresAt: { gt: new Date() },
    },
  });

  if (!tokenRecord) {
    throw new UnauthorizedError('Invalid refresh token');
  }

  // Check for reuse (token already rotated)
  const reuseDetected = await prisma.refreshToken.findFirst({
    where: {
      familyId: tokenRecord.familyId,
      createdAt: { gt: tokenRecord.createdAt },
    },
  });

  if (reuseDetected) {
    // REUSE DETECTED - Revoke entire family
    await prisma.refreshToken.updateMany({
      where: { familyId: tokenRecord.familyId },
      data: { revoked: true },
    });

    await auditLog.log({
      action: 'token.reuse.detected',
      userId: tokenRecord.userId,
      metadata: { familyId: tokenRecord.familyId, ip },
      severity: 'critical',
    });

    throw new UnauthorizedError('Token reuse detected');
  }

  // Revoke current token
  await prisma.refreshToken.update({
    where: { id: tokenRecord.id },
    data: { revoked: true },
  });

  // Issue new tokens
  const newAccessToken = jwt.sign(
    { userId: tokenRecord.userId, groupId: '...' },
    process.env.JWT_SECRET!,
    { expiresIn: '15m' }
  );

  const newRefreshToken = await createRefreshToken({
    userId: tokenRecord.userId,
    familyId: tokenRecord.familyId, // Same family
    deviceFingerprint,
    ip,
    expiresAt: new Date(Date.now() + 7 * 24 * 60 * 60 * 1000),
  });

  return { accessToken: newAccessToken, refreshToken: newRefreshToken };
}
```

### CSRF Protection

For cookie-based refresh tokens, use **SameSite** and **double-submit pattern**.

```typescript
// Set refresh token as HttpOnly cookie
reply.setCookie('refresh_token', refreshToken, {
  httpOnly: true,
  secure: true,           // HTTPS only
  sameSite: 'strict',     // CSRF protection
  maxAge: 7 * 24 * 60 * 60, // 7 days
  path: '/api/auth/refresh',
});

// Also set CSRF token
const csrfToken = uuidv4();
await redis.setex(`csrf:${userId}`, 900, csrfToken); // 15 min

reply.send({ csrfToken });
```

---

## Authorization

### Role-Based Access Control (RBAC)

```typescript
interface Permission {
  id: string;
  resource: string;  // e.g., 'entity', 'app', 'user'
  action: string;    // e.g., 'read', 'write', 'admin'
  scope: 'group' | 'tenant';
  groupId: string;   // Scoped to group
}

// Check permission
async function requirePermission(
  userId: string,
  groupId: string,
  permission: string // Format: "resource:action"
): Promise<void> {
  const [resource, action] = permission.split(':');

  const hasPermission = await prisma.permission.findFirst({
    where: {
      resource,
      action: { in: [action, '*'] }, // Wildcard support
      groupId,
      roles: {
        some: {
          groupMembers: {
            some: { userId, groupId },
          },
        },
      },
    },
  });

  if (!hasPermission) {
    await auditLog.log({
      action: 'permission.denied',
      userId,
      groupId,
      metadata: { resource, action },
    });

    throw new ForbiddenError(`Missing permission: ${permission}`);
  }
}
```

### Permission Decorators

```typescript
// Fastify hook for permission checking
export function requiresPermission(permission: string) {
  return async (request: FastifyRequest, reply: FastifyReply) => {
    const { userId, groupId } = request.user;
    await requirePermission(userId, groupId, permission);
  };
}

// Usage in routes
fastify.get('/api/entities', {
  preHandler: requiresPermission('entity:read'),
  handler: async (request, reply) => {
    // Handler code
  },
});
```

---

## Multi-Tenancy Isolation

### Prisma Middleware (Auto-inject groupId)

```typescript
import { Prisma } from '@prisma/client';

export function createTenancyMiddleware(prisma: PrismaClient) {
  prisma.$use(async (params, next) => {
    const groupId = (params as any).groupId; // Set by request context

    if (!groupId) {
      throw new Error('Missing groupId in Prisma query');
    }

    // Models that require tenant scoping
    const multiTenantModels = [
      'EntityRecord',
      'EntityDefinition',
      'AppInstall',
      'GroupMember',
    ];

    if (multiTenantModels.includes(params.model || '')) {
      if (params.action === 'findUnique' || params.action === 'findFirst') {
        params.args.where = { ...params.args.where, groupId };
      } else if (params.action === 'findMany') {
        params.args.where = { ...params.args.where, groupId };
      } else if (params.action === 'create') {
        params.args.data = { ...params.args.data, groupId };
      } else if (params.action === 'update' || params.action === 'delete') {
        params.args.where = { ...params.args.where, groupId };
      }
    }

    return next(params);
  });
}
```

### PostgreSQL Row-Level Security (RLS)

```sql
-- Enable RLS on multi-tenant tables
ALTER TABLE "EntityRecord" ENABLE ROW LEVEL SECURITY;

-- Create policy
CREATE POLICY tenant_isolation ON "EntityRecord"
  USING (group_id = current_setting('flexi.current_group'));

-- Set in Fastify request context
SET LOCAL flexi.current_group = '<groupId>';
```

### Request Context Hook

```typescript
// Fastify plugin to set tenant context
fastify.addHook('onRequest', async (request, reply) => {
  const { groupId } = request.user; // From JWT

  // Set for Prisma middleware
  (request as any).groupId = groupId;

  // Set for PostgreSQL RLS
  await prisma.$executeRawUnsafe(
    `SET LOCAL flexi.current_group = $1`,
    groupId
  );
});
```

---

## Secrets Management

### Environment Variables

```typescript
import { z } from 'zod';
import dotenv from 'dotenv';

dotenv.config();

const envSchema = z.object({
  DATABASE_URL: z.string().url(),
  REDIS_URL: z.string().url(),
  JWT_SECRET: z.string().min(32),
  JWT_REFRESH_SECRET: z.string().min(32),
  NODE_ENV: z.enum(['development', 'production', 'test']),
});

export const env = envSchema.parse(process.env);
```

### Never Log Secrets

```typescript
// ❌ Bad
logger.info({ password, token }, 'User logged in');

// ✅ Good - Redact sensitive fields
const safeLog = (obj: any) => {
  const redacted = { ...obj };
  const sensitiveFields = ['password', 'token', 'secret', 'apiKey'];

  sensitiveFields.forEach(field => {
    if (redacted[field]) {
      redacted[field] = '[REDACTED]';
    }
  });

  return redacted;
};

logger.info(safeLog(data), 'User logged in');
```

### Error Sanitization

```typescript
// Never expose internal errors to clients
export function sanitizeError(error: Error): { message: string; code: string } {
  if (error instanceof UnauthorizedError) {
    return { message: error.message, code: 'UNAUTHORIZED' };
  }

  if (error instanceof ForbiddenError) {
    return { message: error.message, code: 'FORBIDDEN' };
  }

  // Generic error for unexpected failures
  logger.error({ error }, 'Unexpected error');
  return { message: 'Internal server error', code: 'INTERNAL_ERROR' };
}
```

---

## Audit Logging

### Audit Log Schema

```typescript
interface AuditLogEntry {
  id: string;
  actorUserId: string;
  groupId: string;
  resource: string;      // e.g., 'user', 'entity', 'permission'
  action: string;        // e.g., 'create', 'update', 'delete'
  targetId?: string;     // ID of affected resource
  metadata: Record<string, any>;
  success: boolean;
  ip?: string;
  userAgent?: string;
  correlationId?: string; // For tracing requests
  createdAt: Date;
}
```

### Implementation

```typescript
export class AuditLogger {
  async log(entry: Omit<AuditLogEntry, 'id' | 'createdAt'>): Promise<void> {
    await prisma.auditLog.create({
      data: {
        ...entry,
        id: uuidv4(),
        createdAt: new Date(),
      },
    });

    // Also log to structured logger
    logger.info({
      audit: true,
      ...entry,
    }, `Audit: ${entry.action} ${entry.resource}`);
  }
}
```

### Critical Actions to Audit

- ✅ Authentication (login, logout, MFA)
- ✅ Authorization changes (role assignments, permission grants)
- ✅ User management (create, update, delete)
- ✅ Data access (read/write sensitive entities)
- ✅ Security events (token reuse, failed auth attempts)
- ✅ Configuration changes (app installs, settings updates)

```typescript
// Example usage
await auditLog.log({
  actorUserId: request.user.userId,
  groupId: request.user.groupId,
  resource: 'entity',
  action: 'delete',
  targetId: entityId,
  metadata: { entityType: 'Customer' },
  success: true,
  ip: request.ip,
  userAgent: request.headers['user-agent'],
  correlationId: request.id,
});
```

---

## Rate Limiting

### Per-User Rate Limiting

```typescript
import rateLimit from '@fastify/rate-limit';

fastify.register(rateLimit, {
  max: 100,           // 100 requests
  timeWindow: 60000,  // per minute
  keyGenerator: (request) => request.user?.userId || request.ip,
  errorResponseBuilder: () => ({
    statusCode: 429,
    error: 'Too Many Requests',
    message: 'Rate limit exceeded',
  }),
});
```

### Per-Endpoint Rate Limiting

```typescript
// Stricter limits for auth endpoints
fastify.post('/api/auth/login', {
  config: {
    rateLimit: {
      max: 5,
      timeWindow: 60000, // 5 per minute
    },
  },
  handler: loginHandler,
});
```

---

## Security Headers

```typescript
import helmet from '@fastify/helmet';

fastify.register(helmet, {
  contentSecurityPolicy: {
    directives: {
      defaultSrc: ["'self'"],
      styleSrc: ["'self'", "'unsafe-inline'"],
      scriptSrc: ["'self'"],
      imgSrc: ["'self'", 'data:', 'https:'],
    },
  },
  hsts: {
    maxAge: 31536000,
    includeSubDomains: true,
    preload: true,
  },
});
```

---

## Security Checklist

- [ ] All passwords hashed with Argon2id
- [ ] JWT access tokens expire in ≤15 minutes
- [ ] Refresh token rotation implemented
- [ ] Token reuse detection active
- [ ] MFA enabled for admin actions
- [ ] All queries include `groupId` filter
- [ ] PostgreSQL RLS enabled
- [ ] Sensitive data never logged
- [ ] Audit logging for security events
- [ ] Rate limiting on all endpoints
- [ ] HTTPS/TLS enforced (HSTS)
- [ ] Security headers configured (Helmet)
- [ ] CSRF protection for cookie-based auth
- [ ] Input validation with Zod/Ajv
- [ ] Error messages sanitized
