export type GithubBuildStep =
  | 'queued'
  | 'cloning'
  | 'building'
  | 'bundling'
  | 'downloading'
  | 'uploading'
  | 'done'
  | 'failed';

export interface GithubBuildJobData {
  jobId: string;
  repo: string;
  branch: string;
  buildCommand: string;
  artifactPath: string;
  packageName: string;
  version: string;
  groupId: string;
  userId?: string | null;
  policyId?: string;
  approve?: boolean;
  install?: boolean;
  artifactUrl?: string;
  artifactToken?: string;
  manifest?: any;
}

export interface GithubBuildStatus {
  jobId: string;
  status: GithubBuildStep;
  message?: string;
  error?: string;
  progress?: number;
  repo?: string;
  branch?: string;
  artifactPath?: string;
  packageId?: string;
  groupId: string;
  userId?: string | null;
  updatedAt: string;
}
