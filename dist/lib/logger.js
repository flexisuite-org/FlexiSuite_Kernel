"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.logger = void 0;
const pino_1 = __importDefault(require("pino"));
const config_1 = require("../config");
exports.logger = (0, pino_1.default)({
    level: config_1.config.LOG_LEVEL,
    redact: ['password', 'passwordHash', 'token', 'authorization', 'refreshToken'],
    transport: process.env.NODE_ENV === 'development' ? { target: 'pino-pretty' } : undefined
});
//# sourceMappingURL=logger.js.map