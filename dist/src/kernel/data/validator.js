"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.getValidator = getValidator;
const ajv_1 = __importDefault(require("ajv"));
const ajv = new ajv_1.default({ allErrors: true, useDefaults: true, removeAdditional: 'failing' });
const cache = new Map();
function getValidator(schemaId, schema) {
    if (cache.has(schemaId))
        return cache.get(schemaId);
    const validate = ajv.compile(schema);
    cache.set(schemaId, validate);
    return validate;
}
//# sourceMappingURL=validator.js.map