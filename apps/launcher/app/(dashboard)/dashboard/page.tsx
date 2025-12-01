'use client';

import { useEffect, useState } from 'react';
import { useAuth } from '@/lib/auth';
import { getCookie } from '@/lib/cookies';
import { LauncherGroup, GroupInvite } from '@/types/api';
import { GroupSwitcher } from '@/components/launcher/GroupSwitcher';
import { AppTileGrid } from '@/components/launcher/AppTileGrid';
import { PendingInvitesSection } from '@/components/launcher/PendingInvitesSection';
import { EmptyState } from '@/components/launcher/EmptyState';
import { Loader2 } from 'lucide-react';

export default function LauncherPage() {
    const { user } = useAuth();
    const [groups, setGroups] = useState<LauncherGroup[]>([]);
    const [invites, setInvites] = useState<GroupInvite[]>([]);
    const [selectedGroupId, setSelectedGroupId] = useState<string | null>(null);
    const [isLoading, setIsLoading] = useState(true);

    useEffect(() => {
        const fetchData = async () => {
            const token = getCookie('flexi_token');
            if (!token) return;

            try {
                // Fetch groups and installs
                const groupsRes = await fetch(`${process.env.NEXT_PUBLIC_KERNEL_API}/launcher/groups`, {
                    headers: { Authorization: `Bearer ${token}` }
                });

                // Fetch pending invites
                const invitesRes = await fetch(`${process.env.NEXT_PUBLIC_KERNEL_API}/invites/pending`, {
                    headers: { Authorization: `Bearer ${token}` }
                });

                if (groupsRes.ok) {
                    const groupsData = await groupsRes.json();
                    setGroups(groupsData);
                    if (groupsData.length > 0 && !selectedGroupId) {
                        setSelectedGroupId(groupsData[0].groupId);
                    }
                }

                if (invitesRes.ok) {
                    const invitesData = await invitesRes.json();
                    setInvites(invitesData);
                }

            } catch (e) {
                console.error('Failed to fetch launcher data', e);
            } finally {
                setIsLoading(false);
            }
        };

        fetchData();
    }, [selectedGroupId]);

    const handleAppClick = (packageId: string) => {
        // In a real app, this would navigate to the app's route or open it
        console.log('Opening app:', packageId);
        alert(`Opening app: ${packageId}`);
    };

    if (isLoading) {
        return (
            <div className="flex items-center justify-center h-64">
                <Loader2 className="w-8 h-8 animate-spin text-slate-300" />
            </div>
        );
    }

    const selectedGroup = groups.find(g => g.groupId === selectedGroupId);

    // If user has no groups at all
    if (!isLoading && groups.length === 0 && invites.length === 0) {
        return <EmptyState />;
    }

    return (
        <div className="space-y-8">
            <header className="flex items-center justify-between pb-6 border-b border-slate-100">
                <div>
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

                {groups.length > 0 && (
                    <div className="flex gap-2">
                        <select
                            value={selectedGroupId || ''}
                            onChange={(e) => setSelectedGroupId(e.target.value)}
                            className="px-4 py-2 border border-slate-200 rounded-lg text-sm font-medium bg-white hover:bg-slate-50 transition-colors md:hidden"
                        >
                            {groups.map((group) => (
                                <option key={group.groupId} value={group.groupId}>
                                    {group.name}
                                </option>
                            ))}
                        </select>
                    </div>
                )}
            </header>

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
                        <p className="text-slate-500">Select a group from the dropdown above</p>
                    </div>
                ) : null}
            </section>
        </div>
    );
}
