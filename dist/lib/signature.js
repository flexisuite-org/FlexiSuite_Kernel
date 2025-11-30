"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.signHmac = signHmac;
exports.verifyHmac = verifyHmac;
const crypto_1 = __importDefault(require("crypto"));
function signHmac(data, secret) {
    return crypto_1.default.createHmac('sha256', secret).update(data).digest('hex');
}
function verifyHmac(data, signature, secret) {
    const expected = signHmac(data, secret);
    return crypto_1.default.timingSafeEqual(Buffer.from(expected), Buffer.from(signature));
}
//# sourceMappingURL=signature.js.map