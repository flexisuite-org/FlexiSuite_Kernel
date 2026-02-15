import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Activity as ActivityIcon, Bell, Clock } from 'lucide-react';

export default function ActivityPage() {
    return (
        <div className="space-y-8">
            <header className="pb-6 border-b border-slate-100">
                <h1 className="text-2xl font-bold text-slate-900">Activity</h1>
                <p className="text-slate-500">
                    See what&apos;s happening across your workspace – jobs, installs, and important notices.
                </p>
            </header>

            <div className="grid grid-cols-1 gap-6 lg:grid-cols-2">
                <Card>
                    <CardHeader className="flex flex-row items-center justify-between space-y-0">
                        <div>
                            <CardTitle className="text-base flex items-center gap-2">
                                <ActivityIcon className="w-4 h-4 text-slate-400" />
                                Jobs & Builds
                            </CardTitle>
                            <CardDescription>
                                Long-running tasks like GitHub builds and AI jobs.
                            </CardDescription>
                        </div>
                    </CardHeader>
                    <CardContent className="py-8">
                        <div className="flex flex-col items-center justify-center text-slate-500 text-sm">
                            <Clock className="w-8 h-8 mb-3 opacity-30" />
                            <p>No recent jobs to show yet.</p>
                        </div>
                    </CardContent>
                </Card>

                <Card>
                    <CardHeader className="flex flex-row items-center justify-between space-y-0">
                        <div>
                            <CardTitle className="text-base flex items-center gap-2">
                                <Bell className="w-4 h-4 text-slate-400" />
                                Notifications
                            </CardTitle>
                            <CardDescription>
                                Invites, install completions, and important announcements.
                            </CardDescription>
                        </div>
                    </CardHeader>
                    <CardContent className="py-8">
                        <div className="flex flex-col items-center justify-center text-slate-500 text-sm">
                            <Bell className="w-8 h-8 mb-3 opacity-30" />
                            <p>No notifications yet. You&apos;re all caught up.</p>
                        </div>
                    </CardContent>
                </Card>
            </div>
        </div>
    );
}

