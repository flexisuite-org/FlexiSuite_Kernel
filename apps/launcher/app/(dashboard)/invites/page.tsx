import { useState } from 'react';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Loader2, Mail } from 'lucide-react';
import { usePendingInvites } from '@/hooks/usePendingInvites';
import { acceptGroupInvite, declineGroupInvite, ApiError } from '@/lib/apiClient';

export default function InvitesPage() {
  const { invites, isLoading, refresh } = usePendingInvites();
  const [processingId, setProcessingId] = useState<string | null>(null);
  const [error, setError] = useState<string>('');

  const handleAction = async (code: string, inviteId: string, action: 'accept' | 'decline') => {
    setError('');
    setProcessingId(inviteId);
    try {
      if (action === 'accept') {
        await acceptGroupInvite(code);
      } else {
        await declineGroupInvite(code);
      }
      await refresh();
    } catch (err: unknown) {
      const message = err instanceof ApiError ? err.message : 'Unable to update invite';
      setError(message);
    } finally {
      setProcessingId(null);
    }
  };

  if (isLoading) {
    return (
      <div className="min-h-screen flex items-center justify-center bg-slate-50">
        <Loader2 className="w-8 h-8 animate-spin text-primary" />
      </div>
    );
  }

  return (
    <div className="space-y-8">
      <header className="flex items-center justify-between pb-6 border-b border-slate-100">
        <div>
          <h1 className="text-2xl font-bold text-slate-900">Invites</h1>
          <p className="text-slate-500">Manage your pending invitations and sent requests</p>
        </div>
      </header>

      {error && (
        <div className="rounded-xl bg-red-50 border border-red-100 p-4 text-sm text-red-700">
          {error}
        </div>
      )}

      <div className="grid gap-6">
        {invites.length === 0 ? (
          <Card>
            <CardHeader>
              <CardTitle className="text-lg">Pending Invites</CardTitle>
              <CardDescription>No pending invites at the moment.</CardDescription>
            </CardHeader>
            <CardContent>
              <div className="flex flex-col items-center justify-center py-8 text-slate-500">
                <Mail className="w-10 h-10 mb-3 opacity-20" />
                <p>Nothing to accept or decline yet.</p>
              </div>
            </CardContent>
          </Card>
        ) : (
          invites.map((invite) => (
            <Card key={invite.id} className="border-slate-100">
              <CardHeader className="flex flex-col gap-1">
                <CardTitle className="text-lg flex items-center gap-2">
                  <Mail className="w-4 h-4 text-slate-400" />
                  Invitation to {invite.groupName || 'a group'}
                </CardTitle>
                <CardDescription>
                  Invited by {invite.inviter?.email || 'an admin'} • Expires{' '}
                  {invite.expiresAt ? new Date(invite.expiresAt).toLocaleDateString() : 'soon'}
                </CardDescription>
              </CardHeader>
              <CardContent className="flex flex-col gap-3">
                <p className="text-sm text-slate-500">Join this group to collaborate with the team.</p>
                <div className="flex flex-wrap gap-2">
                  <Button
                    size="sm"
                    variant="secondary"
                    onClick={() => handleAction(invite.code, invite.id, 'accept')}
                    isLoading={processingId === invite.id && processingId !== null}
                  >
                    Accept
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => handleAction(invite.code, invite.id, 'decline')}
                    disabled={processingId === invite.id}
                  >
                    Decline
                  </Button>
                </div>
              </CardContent>
            </Card>
          ))
        )}
      </div>
    </div>
  );
}
