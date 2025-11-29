import fs from 'fs/promises';
import path from 'path';
import { sha256Hex } from '../../lib/integrity';

export interface BundleStoreResult {
  uri: string; // storage location identifier
  integrity: string;
}

export interface BundleStorage {
  save(id: string, data: Buffer): Promise<BundleStoreResult>;
  load(id: string): Promise<Buffer>;
}

export class LocalBundleStorage implements BundleStorage {
  constructor(private baseDir = 'storage/bundles') {}

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

export const bundleStorage: BundleStorage = new LocalBundleStorage();
