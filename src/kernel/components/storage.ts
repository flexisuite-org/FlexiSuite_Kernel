import fs from 'fs/promises';
import path from 'path';
import { sha256Hex } from '../../lib/integrity';
import { config } from '../../config';

export interface BundleStoreResult {
  uri: string; // storage location identifier
  integrity: string;
}

export interface BundleStorage {
  save(id: string, data: Buffer): Promise<BundleStoreResult>;
  load(id: string): Promise<Buffer>;
}

export class LocalBundleStorage implements BundleStorage {
  constructor(private baseDir = config.bundleStorage.localDir || 'storage/bundles') {}

  async save(id: string, data: Buffer): Promise<BundleStoreResult> {
    await fs.mkdir(this.baseDir, { recursive: true });
    const file = path.join(this.baseDir, `${id}.bin`);
    await fs.writeFile(file, data);
    return { uri: file, integrity: sha256Hex(data) };
  }

  async load(id: string): Promise<Buffer> {
    const file = path.join(this.baseDir, `${id}.bin`);
    return fs.readFile(file);
  }
}

class S3BundleStorage implements BundleStorage {
  private s3: any;
  private bucket: string;
  private PutObjectCommand: any;
  private GetObjectCommand: any;
  constructor() {
    this.bucket = config.bundleStorage.s3.bucket || '';
    if (!this.bucket) throw new Error('S3_BUCKET is required when STORAGE_DRIVER=s3');
  }

  private async ensureClient() {
    if (this.s3) return;
    try {
      // Dynamic import; suppressed type resolution for optional dependency
      // @ts-ignore optional runtime dependency
      const mod = await import('@aws-sdk/client-s3');
      const { S3, PutObjectCommand, GetObjectCommand } = mod as any;
      this.PutObjectCommand = PutObjectCommand;
      this.GetObjectCommand = GetObjectCommand;
      this.s3 = new S3({
        region: config.bundleStorage.s3.region,
        endpoint: config.bundleStorage.s3.endpoint,
        forcePathStyle: config.bundleStorage.s3.forcePathStyle,
        credentials:
          config.bundleStorage.s3.accessKeyId && config.bundleStorage.s3.secretAccessKey
            ? {
                accessKeyId: config.bundleStorage.s3.accessKeyId,
                secretAccessKey: config.bundleStorage.s3.secretAccessKey
              }
            : undefined
      });
    } catch (err) {
      throw new Error('AWS SDK for S3 is not installed. Add @aws-sdk/client-s3 to dependencies.');
    }
  }

  async save(id: string, data: Buffer): Promise<BundleStoreResult> {
    await this.ensureClient();
    const key = `${id}.bin`;
    await this.s3.send(
      new this.PutObjectCommand({
        Bucket: this.bucket,
        Key: key,
        Body: data
      })
    );
    return { uri: `s3://${this.bucket}/${key}`, integrity: sha256Hex(data) };
  }

  async load(id: string): Promise<Buffer> {
    await this.ensureClient();
    const key = `${id}.bin`;
    const res = await this.s3.send(
      new this.GetObjectCommand({
        Bucket: this.bucket,
        Key: key
      })
    );
    const body: any = res.Body;
    if (body?.transformToByteArray) {
      const arr = await body.transformToByteArray();
      return Buffer.from(arr);
    }
    const chunks: Buffer[] = [];
    for await (const chunk of body as any) {
      chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
    }
    return Buffer.concat(chunks);
  }
}

function createBundleStorage(): BundleStorage {
  if (config.bundleStorage.driver === 's3') {
    return new S3BundleStorage();
  }
  return new LocalBundleStorage();
}

export const bundleStorage: BundleStorage = createBundleStorage();
