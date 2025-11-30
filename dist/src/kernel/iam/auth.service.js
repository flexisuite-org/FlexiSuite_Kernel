"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.authService = exports.AuthService = void 0;
const argon2_1 = __importDefault(require("argon2"));
const jsonwebtoken_1 = __importDefault(require("jsonwebtoken"));
const crypto_1 = __importDefault(require("crypto"));
const db_1 = require("../../lib/db");
const config_1 = require("../../config");
const uuid_1 = require("uuid");
function signAccessToken(userId, groupId, roles = []) {
    const payload = { userId, groupId, roles };
    const options = { expiresIn: config_1.config.JWT_EXPIRES_IN };
    return jsonwebtoken_1.default.sign(payload, config_1.config.JWT_SECRET, options);
}
function hashToken(raw) {
    return crypto_1.default.createHash('sha256').update(raw).digest('hex');
}
class AuthService {
    async signup(email, password) {
        const passwordHash = await argon2_1.default.hash(password);
        const user = await db_1.prisma.user.create({ data: { email, passwordHash } });
        return this.issueTokens(user.id, null, []);
    }
    async login(email, password) {
        const user = await db_1.prisma.user.findUnique({ where: { email } });
        if (!user)
            throw new Error('Invalid credentials');
        const ok = await argon2_1.default.verify(user.passwordHash, password);
        if (!ok)
            throw new Error('Invalid credentials');
        return this.issueTokens(user.id, null, []);
    }
    async refresh(userId, refreshToken, familyId) {
        const hashed = hashToken(refreshToken);
        const token = await db_1.prisma.refreshToken.findFirst({ where: { tokenHash: hashed, userId, revoked: false } });
        if (!token)
            throw new Error('Invalid refresh token');
        // Reuse detection
        if (familyId && token.familyId !== familyId) {
            await db_1.prisma.refreshToken.updateMany({ where: { familyId: token.familyId }, data: { revoked: true } });
            throw new Error('Refresh token reuse detected');
        }
        await db_1.prisma.refreshToken.update({ where: { id: token.id }, data: { revoked: true } });
        return this.issueTokens(userId, null, [], token.familyId);
    }
    async issueTokens(userId, groupId, roles, familyId) {
        const accessToken = signAccessToken(userId, groupId, roles);
        const rawRefresh = (0, uuid_1.v4)();
        const refreshId = (0, uuid_1.v4)();
        const family = familyId ?? (0, uuid_1.v4)();
        await db_1.prisma.refreshToken.create({
            data: {
                id: refreshId,
                userId,
                tokenHash: hashToken(rawRefresh),
                familyId: family,
                expiresAt: new Date(Date.now() + this.parseExpiry(config_1.config.REFRESH_TOKEN_EXPIRES_IN)),
                revoked: false
            }
        });
        return { accessToken, refreshToken: rawRefresh, refreshTokenId: refreshId };
    }
    parseExpiry(expr) {
        // very small parser: supports m,h,d
        const match = expr.match(/(\d+)([smhd])/);
        if (!match)
            return 0;
        const value = parseInt(match[1], 10);
        const unit = match[2];
        const multipliers = { s: 1000, m: 60000, h: 3600000, d: 86400000 };
        return value * (multipliers[unit] ?? 0);
    }
}
exports.AuthService = AuthService;
exports.authService = new AuthService();
//# sourceMappingURL=auth.service.js.map