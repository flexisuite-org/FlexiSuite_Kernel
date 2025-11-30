import { processArtifactFlow } from '../src/integrations/github/queue/artifact-job';
import { GithubBuildJobData } from '../src/integrations/github/types';

describe('GitHub artifact pipeline', () => {
  it('fetches the artifact before registering it', async () => {
    const job: GithubBuildJobData = {
      jobId: 'job-artifact-1',
      repo: 'demo/repo',
      branch: 'main',
      buildCommand: 'echo hi',
      artifactPath: 'dist',
      packageName: '@demo/test',
      version: '1.0.0',
      groupId: 'group-1'
    };

    const order: string[] = [];
    const fakeBuffer = Buffer.from('artifact');

    const downloader = jest.fn(async () => {
      order.push('download');
      return fakeBuffer;
    });

    const registrar = jest.fn(async () => {
      order.push('register');
      return { packageId: 'pkg', bundleIntegrity: 'abc' };
    });

    const result = await processArtifactFlow(job, downloader, registrar);

    expect(result.packageId).toBe('pkg');
    expect(result.bundleIntegrity).toBe('abc');
    expect(order).toEqual(['download', 'register']);
    expect(downloader).toHaveBeenCalledWith(job);
    expect(registrar).toHaveBeenCalledWith(job, fakeBuffer);
  });
});
