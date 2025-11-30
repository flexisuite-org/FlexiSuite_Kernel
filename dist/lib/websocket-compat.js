"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
const websocket_1 = __importDefault(require("@fastify/websocket"));
// Patch plugin metadata to allow Fastify v5; upstream still targets v4 but is API-compatible here.
const plugin = websocket_1.default.default ?? websocket_1.default;
const metaKey = Symbol.for('plugin-meta');
if (plugin && plugin[metaKey] && plugin[metaKey].fastify) {
    plugin[metaKey].fastify = '>=5.0.0';
}
exports.default = plugin;
//# sourceMappingURL=websocket-compat.js.map