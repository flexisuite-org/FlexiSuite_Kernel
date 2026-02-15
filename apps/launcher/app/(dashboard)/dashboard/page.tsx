'use client';

import { useEffect, useMemo } from 'react';
import { useRouter, useSearchParams } from 'next/navigation';
import { Loader2, Plus, Users, Rocket, Activity as ActivityIcon } from 'lucide-react';
import { PendingInvitesSection } from '@/components/launcher/PendingInvitesSection';
import { AppTileGrid } from '@/components/launcher/AppTileGrid';
import { EmptyState } from '@/components/launcher/EmptyState';
import { Button } from '@/components/ui/button';
import { launcherDashboardInsights } from '@/data/launcherDashboardInsights';
import { useLauncherGroups } from '@/hooks/useLauncherGroups';
import { usePendingInvites } from '@/hooks/usePendingInvites';
import { useJobStream } from '@/hooks/useJobStream';

const STATUS_STYLES: Record<string, string> = {
  running: 'text-primary bg-primary/10',
  queued: 'text-slate-600 bg-slate-100',
  completed: 'text-emerald-600 bg-emerald-100',
  failed: 'text-red-600 bg-red-100',
};

const formatDate = (value?: string) => {
  if (!value) return '';
  const dt = new Date(value);
  if (Number.isNaN(dt.getTime())) return value;
  return dt.toLocaleString('en-US', { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' });
};

const InsightAppList = ({
  title,
  description,
  apps,
}: {
  title: string;
  description: string;
  apps?: { packageId: string; name: string }[];
}) => {
  if (!apps || apps.length === 0) return null;

  const handleClick = (packageId: string) => {
    if (typeof window === 'undefined') return;
    window.open(`/apps/${packageId}`, '_blank');
  };

  return (
    <section className="border border-slate-200 rounded-xl bg-white shadow-sm p-5">
      <div className="flex items-center justify-between mb-4">
        <div>
          <h2 className="text-lg font-semibold text-slate-900">{title}</h2>
          <p className="text-xs text-slate-500">{description}</p>
        </div>
      </div>
      <div className="grid grid-cols-2 sm:grid-cols-3 gap-3">
        {apps.map((app) => (
          <button
            key={app.packageId}
            type="button"
            onClick={() => handleClick(app.packageId)}
            className="flex flex-col items-start gap-2 p-3 rounded-lg border border-slate-100 hover:border-primary/40 transition-colors bg-slate-50"
          >
            <div className="w-10 h-10 flex items-center justify-center rounded-2xl bg-white text-xs font-semibold uppercase text-slate-500">
              {app.name.charAt(0)}
            </div>
            <span className="text-sm font-medium text-slate-900 text-left">{app.name}</span>
          </button>
        ))}
      </div>
    </section>
  );
};

export default function LauncherPage() {
  const router = useRouter();
  const searchParams = useSearchParams();
  const { user, switchGroup } = useAuth();
  const { groups, isLoading: groupsLoading, error: groupsError } = useLauncherGroups();
  const { invites, isLoading: invitesLoading } = usePendingInvites();
  const groupFromQuery = searchParams.get('group');
  const activeGroupId = useMemo(() => {
    if (groups.length === 0) return null;
    const hasQueryMatch = groupFromQuery && groups.some((group) => group.groupId === groupFromQuery);
    return hasQueryMatch ? groupFromQuery : groups[0].groupId;
  }, [groupFromQuery, groups]);

  useEffect(() => {
    if (!activeGroupId) return;

    if (groupFromQuery !== activeGroupId) {
      const params = new URLSearchParams(searchParams.toString());
      params.set('group', activeGroupId);
      router.replace(`/dashboard?${params.toString()}`, { scroll: false });
    }

    // Sync backend context if needed
    if (user && activeGroupId !== user.currentGroupId) {
      console.log(`[Dashboard] Switching backend context to ${activeGroupId}`);
      switchGroup(activeGroupId).catch(err => {
        console.error('Failed to switch group context', err);
      });
    }
  }, [activeGroupId, groupFromQuery, router, searchParams, user, switchGroup]);

  const handleGroupSelect = (groupId: string) => {
    const params = new URLSearchParams(searchParams.toString());
    params.set('group', groupId);
    router.push(`/dashboard?${params.toString()}`);
  };

  const selectedGroup = groups.find((group) => group.groupId === activeGroupId) ?? null;
  const insightSource = selectedGroup?.insights ?? launcherDashboardInsights;
  const jobIds = insightSource.jobs?.map((job) => job.jobId) ?? [];
  const { jobs: liveJobs, status: jobStatus } = useJobStream(jobIds);
  const jobFeed = liveJobs.length > 0 ? liveJobs : insightSource.jobs ?? [];
  const displayJobStatus = jobIds.length > 0 ? jobStatus : 'idle';

  if (groupsLoading) {
    return (
      <div className="flex items-center justify-center h-64">
        <Loader2 className="w-8 h-8 animate-spin text-slate-300" />
      </div>
    );
  }

  if (!groupsLoading && groups.length === 0 && !invitesLoading && invites.length === 0) {
    return <EmptyState />;
  }

  return (
    <div className="space-y-8">
      <header className="flex flex-col gap-4 pb-6 border-b border-slate-100 md:flex-row md:items-center md:justify-between">
        <div className="space-y-1">
          <h1 className="text-2xl font-bold text-slate-900">
            {selectedGroup ? selectedGroup.name : 'Dashboard'}
          </h1>
          <p className="text-slate-500">
            {selectedGroup
              ? `${selectedGroup.installs.length} apps installed`
              : groups.length > 0
              ? 'Select a group to view apps'
              : 'Welcome to FlexiSuite'}
          </p>
          {groupsError && (
            <p className="text-xs text-red-600">{groupsError}</p>
          )}
        </div>

        <div className="flex flex-col gap-2 md:flex-row md:items-center md:gap-3">
          {groups.length > 0 && (
            <select
              value={activeGroupId ?? ''}
              onChange={(event) => handleGroupSelect(event.target.value)}
              className="px-4 py-2 border border-slate-200 rounded-lg text-sm font-medium bg-white hover:bg-slate-50 transition-colors md:hidden"
            >
              {groups.map((group) => (
                <option key={group.groupId} value={group.groupId}>
                  {group.name}
                </option>
              ))}
            </select>
          )}

          <div className="flex flex-wrap gap-2">
            <Button variant="secondary" size="sm">
              <Plus className="w-4 h-4 mr-1.5" />
              New Group
            </Button>
            <Button variant="outline" size="sm" onClick={() => router.push('/store')}>
              <Rocket className="w-4 h-4 mr-1.5" />
              Install App
            </Button>
            <Button variant="outline" size="sm" onClick={() => router.push('/settings')}>
              <Users className="w-4 h-4 mr-1.5" />
              Invite Member
            </Button>
          </div>
        </div>
      </header>

      <div className="grid grid-cols-1 gap-8 xl:grid-cols-[minmax(0,2fr)_minmax(260px,1fr)]">
        <div className="space-y-8">
          <PendingInvitesSection invites={invites} />

          <InsightAppList
            title="Recently used apps"
            description="Quick access to where you left off"
            apps={insightSource.recentApps}
          />

          <InsightAppList
            title="Pinned for this workspace"
            description="Apps you rely on regularly"
            apps={insightSource.pinnedApps}
          />

          <section>
            <div className="flex items-center justify-between mb-4">
              <h2 className="text-lg font-semibold text-slate-800">Installed Apps</h2>
              <Button size="sm" variant="ghost" onClick={() => router.push('/store')}>
                Browse store
              </Button>
            </div>

            {selectedGroup ? (
              <AppTileGrid apps={selectedGroup.installs} onAppClick={(id) => window.open(`/apps/${id}`, '_blank')} />
            ) : groups.length > 0 ? (
              <div className="text-center py-12 bg-slate-50 rounded-xl border-2 border-dashed border-slate-200">
                <p className="text-slate-500">Select a group from the selector above</p>
              </div>
            ) : null}
          </section>

          <section className="border border-slate-200 rounded-xl bg-white shadow-sm p-6 space-y-3">
            <div className="flex items-center justify-between">
              <div>
                <h2 className="text-base font-semibold text-slate-900">Today in your workspace</h2>
                <p className="text-xs text-slate-500 mt-1">
                  Activity will appear here as jobs finish and drafts are updated for this group.
                </p>
              </div>
              <ActivityIcon className="w-5 h-5 text-slate-400" />
            </div>
            <p className="text-sm text-slate-500">
              Integrations with job servers and AI systems will surface build results and runtime events without changing the layout.
            </p>
          </section>
        </div>

        <aside className="space-y-6">
          <section className="border border-slate-200 rounded-xl bg-white shadow-sm p-5 space-y-3">
            <h3 className="text-sm font-semibold text-slate-900">Recommended for this group</h3>
            <p className="text-xs text-slate-500">
              Suggestions powered by your installed apps and drafts.
            </p>
            <ul className="space-y-2">
              {insightSource.recommendations?.length ? (
                insightSource.recommendations.map((item) => (
                  <li key={item} className="text-sm text-slate-600">
                    • {item}
                  </li>
                ))
              ) : (
                <li className="text-sm text-slate-400">Get recommendations once activity is tracked.</li>
              )}
            </ul>
            <Button variant="secondary" size="sm" className="w-full" onClick={() => router.push('/store')}>
              Browse App Store
            </Button>
          </section>

          <section className="border border-slate-200 rounded-xl bg-white shadow-sm p-5 space-y-4">
            <div className="flex items-center justify-between">
              <div>
                <h3 className="text-sm font-semibold text-slate-900">Active jobs</h3>
                <p className="text-xs text-slate-500">
                  {displayJobStatus === 'open'
                    ? 'Live stream connected'
                    : displayJobStatus === 'connecting'
                    ? 'Connecting to job stream'
                    : 'Waiting for job updates'}
                </p>
              </div>
              <span className="text-xs text-slate-400">{jobFeed.length} tracked</span>
            </div>
            <div className="space-y-3">
              {jobFeed.length > 0 ? (
                jobFeed.map((job) => {
                  const statusClass = STATUS_STYLES[job.status] ?? STATUS_STYLES.queued;
                  const progress = job.progress ?? 0;
                  return (
                    <div key={job.jobId} className="space-y-2 rounded-xl border border-slate-100 bg-slate-50 p-3">
                      <div className="flex items-center justify-between">
                        <div>
                          <p className="font-semibold text-slate-900">{job.title}</p>
                          <p className="text-xs text-slate-500">{job.message || 'Awaiting updates from the kernel'}</p>
                        </div>
                        <span className={`px-2 py-0.5 text-[11px] font-semibold uppercase rounded-full ${statusClass}`}>
                          {job.status}
                        </span>
                      </div>
                      <div className="h-1.5 w-full rounded-full bg-slate-200">
                        <div
                          className="h-full rounded-full bg-primary"
                          style={{ width: `${Math.min(Math.max(progress, 0), 100)}%` }}
                        />
                      </div>
                      <p className="text-[11px] text-slate-500">{formatDate(job.updatedAt)}</p>
                    </div>
                  );
                })
              ) : (
                <div className="flex flex-col items-center justify-center py-6 text-center text-slate-500 text-sm">
                  <p>No active jobs at the moment.</p>
                  <p className="text-xs">Live updates will appear here once a job is subscribed.</p>
                </div>
              )}
            </div>
          </section>

          <section className="border border-slate-200 rounded-xl bg-white shadow-sm p-5 space-y-4">
            <h3 className="text-sm font-semibold text-slate-900">Your drafts</h3>
            <p className="text-xs text-slate-500">Recent sandbox sessions and draft components.</p>
            <div className="space-y-3">
              {insightSource.drafts?.length ? (
                insightSource.drafts.map((draft) => (
                  <div key={draft.id} className="rounded-xl border border-slate-100 bg-slate-50 p-3">
                    <p className="text-sm font-semibold text-slate-900">{draft.title}</p>
                    <p className="text-xs text-slate-500">
                      {draft.status} • {formatDate(draft.updatedAt)}
                    </p>
                    {draft.owner && (
                      <p className="text-[11px] text-slate-400">Owned by {draft.owner}</p>
                    )}
                  </div>
                ))
              ) : (
                <p className="text-xs text-slate-400">No draft sessions detected yet.</p>
              )}
            </div>
            <Button variant="outline" size="sm" className="w-full" onClick={() => router.push('/sandbox')}>
              Open Sandbox
            </Button>
          </section>
        </aside>
      </div>
    </div>
  );
}
