import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { User, Bell, Shield } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { useAuth } from '@/lib/auth';

export default function SettingsPage() {
  const { user } = useAuth();
  const displayName = user ? user.email.split('@')[0] : '';

  return (
    <div className="space-y-8">
      <header className="pb-6 border-b border-slate-100">
        <h1 className="text-2xl font-bold text-slate-900">Settings</h1>
        <p className="text-slate-500">Manage your account preferences and group configurations</p>
      </header>

      <div className="grid grid-cols-1 lg:grid-cols-[240px_1fr] gap-8">
        <nav className="space-y-1">
          <button className="w-full flex items-center gap-3 px-3 py-2 text-sm font-medium bg-primary/10 text-primary rounded-lg">
            <User className="w-4 h-4" /> Profile
          </button>
          <button className="w-full flex items-center gap-3 px-3 py-2 text-sm font-medium text-slate-600 hover:bg-slate-50 rounded-lg">
            <Shield className="w-4 h-4" /> Security
          </button>
          <button className="w-full flex items-center gap-3 px-3 py-2 text-sm font-medium text-slate-600 hover:bg-slate-50 rounded-lg">
            <Bell className="w-4 h-4" /> Notifications
          </button>
        </nav>

        <div className="space-y-6">
          <Card>
            <CardHeader>
              <CardTitle>Profile Information</CardTitle>
              <CardDescription>Update your personal details</CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              <div className="grid gap-2">
                <label className="text-sm font-medium">Display Name</label>
                <Input value={displayName} placeholder="Your Name" readOnly />
              </div>
              <div className="grid gap-2">
                <label className="text-sm font-medium">Email Address</label>
                <Input value={user?.email ?? ''} disabled />
                <p className="text-xs text-slate-500">Contact admin to change email</p>
              </div>
              {user?.memberships.length ? (
                <div className="space-y-2 pt-2">
                  <p className="text-xs uppercase tracking-widest text-slate-400">Memberships</p>
                  {user.memberships.map((membership) => (
                    <div
                      key={membership.groupId}
                      className="rounded-lg border border-slate-100 bg-slate-50 px-3 py-2 text-sm text-slate-600"
                    >
                      {membership.name} • {membership.role}
                    </div>
                  ))}
                </div>
              ) : (
                <p className="text-xs text-slate-400">No memberships yet.</p>
              )}
              <div className="pt-2">
                <Button>Save Changes</Button>
              </div>
            </CardContent>
          </Card>
        </div>
      </div>
    </div>
  );
}
