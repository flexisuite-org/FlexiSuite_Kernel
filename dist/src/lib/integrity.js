"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.stableStringify = stableStringify;
exports.sha256Hex = sha256Hex;
exports.hashJson = hashJson;
exports.verifyIntegrity = verifyIntegrity;
const crypto_1 = __importDefault(require("crypto"));
// Deterministic JSON stringifier (sorts object keys, keeps array order).
function stableStringify(value) {
    const seen = new WeakSet();
    const helper = (input) => {
        if (input === null || typeof input !== 'object')
            return input;
        if (seen.has(input))
            throw new TypeError('circular reference in stableStringify');
        seen.add(input);
        if (Array.isArray(input))
            return input.map(helper);
        const sorted = Object.keys(input).sort().reduce((acc, key) => {
            acc[key] = helper(input[key]);
            return acc;
        }, {});
        seen.delete(input);
        return sorted;
    };
    return JSON.stringify(helper(value));
}
function sha256Hex(data) {
    return crypto_1.default.createHash('sha256').update(data).digest('hex');
}
function hashJson(value) {
    return sha256Hex(stableStringify(value));
}
function verifyIntegrity(expected, data) {
    const payload = data === undefined
        ? 'undefined'
        : typeof data === 'string' || Buffer.isBuffer(data)
            ? data
            : stableStringify(data);
    return sha256Hex(payload) === expected;
}
//# sourceMappingURL=integrity.js.map