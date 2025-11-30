"use strict";
var __createBinding = (this && this.__createBinding) || (Object.create ? (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    var desc = Object.getOwnPropertyDescriptor(m, k);
    if (!desc || ("get" in desc ? !m.__esModule : desc.writable || desc.configurable)) {
      desc = { enumerable: true, get: function() { return m[k]; } };
    }
    Object.defineProperty(o, k2, desc);
}) : (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    o[k2] = m[k];
}));
var __setModuleDefault = (this && this.__setModuleDefault) || (Object.create ? (function(o, v) {
    Object.defineProperty(o, "default", { enumerable: true, value: v });
}) : function(o, v) {
    o["default"] = v;
});
var __importStar = (this && this.__importStar) || (function () {
    var ownKeys = function(o) {
        ownKeys = Object.getOwnPropertyNames || function (o) {
            var ar = [];
            for (var k in o) if (Object.prototype.hasOwnProperty.call(o, k)) ar[ar.length] = k;
            return ar;
        };
        return ownKeys(o);
    };
    return function (mod) {
        if (mod && mod.__esModule) return mod;
        var result = {};
        if (mod != null) for (var k = ownKeys(mod), i = 0; i < k.length; i++) if (k[i] !== "default") __createBinding(result, mod, k[i]);
        __setModuleDefault(result, mod);
        return result;
    };
})();
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.bundleStorage = exports.LocalBundleStorage = void 0;
const promises_1 = __importDefault(require("fs/promises"));
const path_1 = __importDefault(require("path"));
const integrity_1 = require("../../lib/integrity");
const config_1 = require("../../config");
class LocalBundleStorage {
    constructor(baseDir = config_1.config.bundleStorage.localDir || 'storage/bundles') {
        this.baseDir = baseDir;
    }
    async save(id, data) {
        await promises_1.default.mkdir(this.baseDir, { recursive: true });
        const file = path_1.default.join(this.baseDir, `${id}.bin`);
        await promises_1.default.writeFile(file, data);
        return { uri: file, integrity: (0, integrity_1.sha256Hex)(data) };
    }
    async load(id) {
        const file = path_1.default.join(this.baseDir, `${id}.bin`);
        return promises_1.default.readFile(file);
    }
}
exports.LocalBundleStorage = LocalBundleStorage;
class S3BundleStorage {
    constructor() {
        this.bucket = config_1.config.bundleStorage.s3.bucket || '';
        if (!this.bucket)
            throw new Error('S3_BUCKET is required when STORAGE_DRIVER=s3');
    }
    async ensureClient() {
        if (this.s3)
            return;
        try {
            // Dynamic import; suppressed type resolution for optional dependency
            // @ts-ignore optional runtime dependency
            const mod = await Promise.resolve().then(() => __importStar(require('@aws-sdk/client-s3')));
            const { S3, PutObjectCommand, GetObjectCommand } = mod;
            this.PutObjectCommand = PutObjectCommand;
            this.GetObjectCommand = GetObjectCommand;
            this.s3 = new S3({
                region: config_1.config.bundleStorage.s3.region,
                endpoint: config_1.config.bundleStorage.s3.endpoint,
                forcePathStyle: config_1.config.bundleStorage.s3.forcePathStyle,
                credentials: config_1.config.bundleStorage.s3.accessKeyId && config_1.config.bundleStorage.s3.secretAccessKey
                    ? {
                        accessKeyId: config_1.config.bundleStorage.s3.accessKeyId,
                        secretAccessKey: config_1.config.bundleStorage.s3.secretAccessKey
                    }
                    : undefined
            });
        }
        catch (err) {
            throw new Error('AWS SDK for S3 is not installed. Add @aws-sdk/client-s3 to dependencies.');
        }
    }
    async save(id, data) {
        await this.ensureClient();
        const key = `${id}.bin`;
        await this.s3.send(new this.PutObjectCommand({
            Bucket: this.bucket,
            Key: key,
            Body: data
        }));
        return { uri: `s3://${this.bucket}/${key}`, integrity: (0, integrity_1.sha256Hex)(data) };
    }
    async load(id) {
        await this.ensureClient();
        const key = `${id}.bin`;
        const res = await this.s3.send(new this.GetObjectCommand({
            Bucket: this.bucket,
            Key: key
        }));
        const body = res.Body;
        if (body?.transformToByteArray) {
            const arr = await body.transformToByteArray();
            return Buffer.from(arr);
        }
        const chunks = [];
        for await (const chunk of body) {
            chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
        }
        return Buffer.concat(chunks);
    }
}
function createBundleStorage() {
    if (config_1.config.bundleStorage.driver === 's3') {
        return new S3BundleStorage();
    }
    return new LocalBundleStorage();
}
exports.bundleStorage = createBundleStorage();
//# sourceMappingURL=storage.js.map