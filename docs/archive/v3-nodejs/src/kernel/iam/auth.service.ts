import argon2 from 'argon2';
import jwt, { SignOptions, Secret } from 'jsonwebtoken';
import crypto from 'crypto';
import { prisma } from '../../lib/db';
import { config } from '../../config';
import { v4 as uuid } from 'uuid';
import { recordAuditLog } from '../../lib/audit';

interface Tokens {
  accessToken: string;
  refreshToken: string;
  refreshTokenId: string;
}

function signAccessToken(userId: string, groupId: string | null, roles: string[] = []): string {
  const payload = { userId, groupId, roles };
  const options: SignOptions = { expiresIn: config.JWT_EXPIRES_IN as any };
  return (jwt.sign as (p: any, s: Secret, o?: SignOptions) => string)(payload, config.JWT_SECRET as Secret, options);
}

function hashToken(raw: string): string {
  return crypto.createHash('sha256').update(raw).digest('hex');
}

export class AuthService {
  async signup(email: string, password: string): Promise<Tokens> {
    const passwordHash = await argon2.hash(password);
    const user = await prisma.user.create({ data: { email, passwordHash } });
    
    await recordAuditLog({ resource: 'User', action: 'signup', metadata: { userId: user.id } });
    
    return this.issueTokens(user.id, null, []);
  }

  async login(email: string, password: string): Promise<Tokens> {
    const user = await prisma.user.findUnique({ where: { email } });
    if (!user) {
      await recordAuditLog({ resource: 'User', action: 'login', success: false, metadata: { email } });
      throw new Error('Invalid credentials');
    }
    const ok = await argon2.verify(user.passwordHash, password);
    if (!ok) {
      await recordAuditLog({ resource: 'User', action: 'login', success: false, metadata: { userId: user.id } });
      throw new Error('Invalid credentials');
    }

    await recordAuditLog({ resource: 'User', action: 'login', metadata: { userId: user.id } });

    return this.issueTokens(user.id, null, []);
  }

  async refresh(userId: string, refreshToken: string, familyId?: string): Promise<Tokens> {
    const hashed = hashToken(refreshToken);
    const token = await prisma.refreshToken.findFirst({ where: { tokenHash: hashed, userId } });
    
    if (!token) {
      await recordAuditLog({ resource: 'RefreshToken', action: 'refresh', success: false, metadata: { userId } });
      throw new Error('Invalid refresh token');
    }

    // Reuse detection: If token is already revoked, revoke the whole family.
    if (token.revoked) {
      await prisma.refreshToken.updateMany({
        where: { familyId: token.familyId },
        data: { revoked: true }
      });
      await recordAuditLog({ 
        resource: 'RefreshToken', 
        action: 'reuse_detected', 
        success: false, 
        metadata: { userId, familyId: token.familyId } 
      });
      throw new Error('Refresh token reuse detected');
    }

    // Secondary reuse detection: If familyId mismatch (if provided)
    if (familyId && token.familyId !== familyId) {
      await prisma.refreshToken.updateMany({
        where: { familyId: token.familyId },
        data: { revoked: true }
      });
      await recordAuditLog({ 
        resource: 'RefreshToken', 
        action: 'family_mismatch', 
        success: false, 
        metadata: { userId, familyId: token.familyId, expectedFamilyId: familyId } 
      });
      throw new Error('Refresh token family mismatch');
    }

    if (token.expiresAt.getTime() <= Date.now()) {
      throw new Error('Refresh token expired');
    }

    // Revoke old token and issue new pair
    await prisma.refreshToken.update({ where: { id: token.id }, data: { revoked: true } });
    
    return this.issueTokens(userId, null, [], token.familyId);
  }

  async switchGroup(userId: string, refreshToken: string, groupId: string): Promise<Tokens> {
    const hashed = hashToken(refreshToken);
    const token = await prisma.refreshToken.findFirst({ where: { tokenHash: hashed, userId, revoked: false } });
    
    if (!token) {
      throw new Error('Invalid or revoked refresh token');
    }

    if (token.expiresAt.getTime() <= Date.now()) {
      throw new Error('Refresh token expired');
    }

    // Verify membership
    const membership = await prisma.groupMember.findUnique({
      where: { userId_groupId: { userId, groupId } },
      include: { roles: true }
    });

    if (!membership) {
      await recordAuditLog({ 
        resource: 'Group', 
        action: 'switch_forbidden', 
        success: false, 
        metadata: { userId, groupId } 
      });
      throw new Error('User is not a member of the target group');
    }

    const roles = membership.roles.map(r => r.name);

    await recordAuditLog({ resource: 'Group', action: 'switch', metadata: { userId, groupId } });

    // Revoke old token and issue new pair for the target group
    await prisma.refreshToken.update({ where: { id: token.id }, data: { revoked: true } });
    return this.issueTokens(userId, groupId, roles, token.familyId);
  }

  private async issueTokens(userId: string, groupId: string | null, roles: string[], familyId?: string): Promise<Tokens> {
    const accessToken = signAccessToken(userId, groupId, roles);
    const rawRefresh = uuid();
    const refreshId = uuid();
    const family = familyId ?? uuid();

    await prisma.refreshToken.create({
      data: {
        id: refreshId,
        userId,
        tokenHash: hashToken(rawRefresh),
        familyId: family,
        expiresAt: new Date(Date.now() + this.parseExpiry(config.REFRESH_TOKEN_EXPIRES_IN)),
        revoked: false
      }
    });

    return { accessToken, refreshToken: rawRefresh, refreshTokenId: refreshId };
  }

  private parseExpiry(expr: string): number {
    // very small parser: supports m,h,d
    const match = expr.match(/(\d+)([smhd])/);
    if (!match) return 0;
    const value = parseInt(match[1], 10);
    const unit = match[2];
    const multipliers: Record<string, number> = { s: 1000, m: 60000, h: 3600000, d: 86400000 };
    return value * (multipliers[unit] ?? 0);
  }
}

export const authService = new AuthService();
