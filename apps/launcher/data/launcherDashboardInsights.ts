import { AppInstallSummary, DraftSummary, JobSummary, LauncherInsights } from '@/types/api';

const baseId = (name: string) => name.toLowerCase().replace(/\s+/g, '-');

const makeApp = (name: string): AppInstallSummary => ({
  installId: `install-${baseId(name)}`,
  packageId: `pkg.${baseId(name)}`,
  name,
  iconUrl: undefined,
});

const makeDraft = (id: string, title: string, status: string, updatedAt: string, owner?: string): DraftSummary => ({
  id,
  title,
  status,
  updatedAt,
  owner,
});

const makeJob = (
  jobId: string,
  title: string,
  status: JobSummary['status'],
  message: string,
  progress: number | undefined,
  updatedAt: string
): JobSummary => ({
  jobId,
  title,
  status,
  message,
  progress,
  updatedAt,
});

export const launcherDashboardInsights: LauncherInsights = {
  recentApps: [
    makeApp('Workspace AI Assistant'),
    makeApp('Design System Builder'),
    makeApp('Customer Portal'),
  ],
  pinnedApps: [
    makeApp('Audit Log Viewer'),
    makeApp('Team Calendar'),
  ],
  recommendations: [
    'Try the new AI + Run workflow to scaffold a policy update.',
    'Install the Collaboration board to surface async threads.',
  ],
  jobs: [
    makeJob(
      'job-demo-build',
      'Workspace build',
      'running',
      'Syncing packages with the kernel',
      52,
      '2025-11-30T09:12:00Z'
    ),
    makeJob(
      'job-demo-checks',
      'Policy checks',
      'queued',
      'Waiting for build job to finish',
      undefined,
      '2025-11-30T08:45:00Z'
    ),
  ],
  drafts: [
    makeDraft('draft-1', 'Payments modal prototype', 'In progress', '2025-11-28T09:14:00Z', 'Yohaku Hashioka'),
    makeDraft('draft-2', 'AI onboarding flow', 'Review', '2025-11-27T16:02:00Z'),
  ],
};
