'use client';

import { useEffect, useState } from 'react';
import { useRouter, useSearchParams } from 'next/navigation';
import { getCookie } from '@/lib/cookies';
import { LauncherGroup, GroupInvite } from '@/types/api';
import { AppTileGrid } from '@/components/launcher/AppTileGrid';
import { PendingInvitesSection } from '@/components/launcher/PendingInvitesSection';
import { EmptyState } from '@/components/launcher/EmptyState';
import { Loader2, Plus, Users, Rocket, Activity as ActivityIcon } from 'lucide-react';
import { Button } from '@/components/ui/button';

export default function LauncherPage() {
    const router = useRouter();
    const searchParams = useSearchParams();
    const [groups, setGroups] = useState<LauncherGroup[]>([]);
    const [invites, setInvites] = useState<GroupInvite[]>([]);
    const [selectedGroupId, setSelectedGroupId] = useState<string | null>(null);
    const [isLoading, setIsLoading] = useState(true);

    const groupFromQuery = searchParams.get('group');

    useEffect(() => {
        const fetchData = async () => {
            const token = getCookie('flexi_token');
            if (!token) {
                setIsLoading(false);
                return;
            }

            try {
                const [groupsRes, invitesRes] = await Promise.all([
                    fetch(`${process.env.NEXT_PUBLIC_KERNEL_API}/launcher/groups`, {
                        headers: { Authorization: `Bearer ${token}` },
                    }),
                    fetch(`${process.env.NEXT_PUBLIC_KERNEL_API}/invites/pending`, {
                        headers: { Authorization: `Bearer ${token}` },
                    }),
                ]);

                if (groupsRes.ok) {
                    const groupsData: LauncherGroup[] = await groupsRes.json();
                    setGroups(groupsData);

                    if (groupsData.length > 0) {
                        let nextGroupId: string | null = null;

                        if (groupFromQuery && groupsData.some((g) => g.groupId === groupFromQuery)) {
                            nextGroupId = groupFromQuery;
                        } else {
                            nextGroupId = groupsData[0].groupId;
                            const params = new URLSearchParams(searchParams.toString());
                            params.set('group', nextGroupId);
                            router.replace(`/dashboard?${params.toString()}`, { scroll: false });
                        }

                        setSelectedGroupId(nextGroupId);
                    }
                }

                if (invitesRes.ok) {
                    const invitesData: GroupInvite[] = await invitesRes.json();
                    setInvites(invitesData);
                }
            } catch (e) {
                console.error('Failed to fetch launcher data', e);
            } finally {
                setIsLoading(false);
            }
        };

        fetchData();
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [groupFromQuery]);

    const handleGroupSelectChange = (groupId: string) => {
        setSelectedGroupId(groupId);
        const params = new URLSearchParams(searchParams.toString());
        params.set('group', groupId);
        router.push(`/dashboard?${params.toString()}`);
    };

    const handleAppClick = (packageId: string) => {
        // AppTileGrid で新規タブオープンするので、ここではログのみ
        console.log('Opening app:', packageId);
    };

    if (isLoading) {
        return (
            <div className="flex items-center justify-center h-64">
                <Loader2 className="w-8 h-8 animate-spin text-slate-300" />
            </div>
        );
    }

    const selectedGroup = groups.find((g) => g.groupId === selectedGroupId);

    // If user has no groups at all
    if (!isLoading && groups.length === 0 && invites.length === 0) {
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
                </div>

                <div className="flex flex-col gap-2 md:flex-row md:items-center md:gap-3">
                    {groups.length > 0 && (
                        <select
                            value={selectedGroupId || ''}
                            onChange={(e) => handleGroupSelectChange(e.target.value)}
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

                    <section>
                        <div className="flex items-center justify-between mb-4">
                            <h2 className="text-lg font-semibold text-slate-800">Installed Apps</h2>
                        </div>

                        {selectedGroup ? (
                            <AppTileGrid
                                apps={selectedGroup.installs}
                                onAppClick={handleAppClick}
                            />
                        ) : groups.length > 0 ? (
                            <div className="text-center py-12 bg-slate-50 rounded-xl border-2 border-dashed border-slate-200">
                                <p className="text-slate-500">
                                    Select a group from the selector above
                                </p>
                            </div>
                        ) : null}
                    </section>

                    {/* Today digest placeholder */}
                    <section className="border border-slate-200 rounded-xl bg-white shadow-sm p-6 space-y-3">
                        <div className="flex items-center justify-between">
                            <div>
                                <h2 className="text-base font-semibold text-slate-900">
                                    Today in your workspace
                                </h2>
                                <p className="text-xs text-slate-500 mt-1">
                                    New components, failed builds, fresh drafts – all in one place.
                                </p>
                            </div>
                            <ActivityIcon className="w-5 h-5 text-slate-400" />
                        </div>
                        <p className="text-sm text-slate-500">
                            Activity feed will appear here as we connect build jobs and draft events.
                        </p>
                    </section>
                </div>

                {/* Right column sections */}
                <aside className="space-y-6">
                    <section className="border border-slate-200 rounded-xl bg-white shadow-sm p-5">
                        <h3 className="text-sm font-semibold text-slate-900 mb-1">
                            Recommended for this group
                        </h3>
                        <p className="text-xs text-slate-500 mb-3">
                            Curated suggestions based on your installed apps will appear here.
                        </p>
                        <Button
                            variant="secondary"
                            size="sm"
                            className="w-full"
                            onClick={() => router.push('/store')}
                        >
                            Browse App Store
                        </Button>
                    </section>

                    <section className="border border-slate-200 rounded-xl bg-white shadow-sm p-5">
                        <h3 className="text-sm font-semibold text-slate-900 mb-1">
                            Active jobs
                        </h3>
                        <p className="text-xs text-slate-500 mb-3">
                            Long-running tasks like builds and AI jobs will show up here.
                        </p>
                        <p className="text-xs text-slate-400">
                            No active jobs at the moment.
                        </p>
                    </section>

                    <section className="border border-slate-200 rounded-xl bg-white shadow-sm p-5">
                        <h3 className="text-sm font-semibold text-slate-900 mb-1">
                            Your drafts
                        </h3>
                        <p className="text-xs text-slate-500 mb-3">
                            Recent sandbox sessions and draft components will be listed here.
                        </p>
                        <Button
                            variant="outline"
                            size="sm"
                            className="w-full"
                            onClick={() => router.push('/sandbox')}
                        >
                            Open Sandbox
                        </Button>
                    </section>
                </aside>
            </div>
        </div>
    );
}
