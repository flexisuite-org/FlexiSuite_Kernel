import argon2 from 'argon2';
import jwt, { SignOptions, Secret } from 'jsonwebtoken';
import crypto from 'crypto';
import { prisma } from '../../lib/db';
import { config } from '../../config';
import { v4 as uuid } from 'uuid';

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
    return this.issueTokens(user.id, null, []);
  }

  async login(email: string, password: string): Promise<Tokens> {
    const user = await prisma.user.findUnique({ where: { email } });
    if (!user) throw new Error('Invalid credentials');
    const ok = await argon2.verify(user.passwordHash, password);
    if (!ok) throw new Error('Invalid credentials');
    return this.issueTokens(user.id, null, []);
  }

  async refresh(userId: string, refreshToken: string, familyId?: string): Promise<Tokens> {
    const hashed = hashToken(refreshToken);
    const token = await prisma.refreshToken.findFirst({ where: { tokenHash: hashed, userId, revoked: false } });
    if (!token) throw new Error('Invalid refresh token');

    // Reuse detection
    if (familyId && token.familyId !== familyId) {
      await prisma.refreshToken.updateMany({ where: { familyId: token.familyId }, data: { revoked: true } });
      throw new Error('Refresh token reuse detected');
    }

    await prisma.refreshToken.update({ where: { id: token.id }, data: { revoked: true } });
    return this.issueTokens(userId, null, [], token.familyId);
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
