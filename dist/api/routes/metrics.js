"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.default = metricsRoutes;
const prom_client_1 = __importDefault(require("prom-client"));
const collectDefaultMetrics = prom_client_1.default.collectDefaultMetrics;
collectDefaultMetrics();
async function metricsRoutes(fastify) {
    fastify.get('/', async (req, reply) => {
        reply.type('text/plain');
        return prom_client_1.default.register.metrics();
    });
}
//# sourceMappingURL=metrics.js.map