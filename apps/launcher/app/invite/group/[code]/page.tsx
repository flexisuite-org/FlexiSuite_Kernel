'use client';

import { use, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { useAuth } from '@/lib/auth';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from '@/components/ui/card';
import { Loader2, CheckCircle2, XCircle } from 'lucide-react';
import { GroupInvite } from '@/types/api';
import {
  getGroupInvite,
  acceptGroupInvite,
  declineGroupInvite,
  ApiError,
} from '@/lib/apiClient';

export default function InvitePage({ params }: { params: Promise<{ code: string }> }) {
  const { code } = use(params);
  const { user, isLoading: isAuthLoading } = useAuth();
  const router = useRouter();
  const [invite, setInvite] = useState<GroupInvite | null>(null);
  const [error, setError] = useState('');
  const [isFetching, setIsFetching] = useState(true);
  const [isProcessing, setIsProcessing] = useState(false);

  useEffect(() => {
    setIsFetching(true);
    setError('');
    getGroupInvite(code)
      .then((data) => setInvite(data))
      .catch((err: unknown) => {
        const message = err instanceof ApiError ? err.message : 'Failed to load invite';
        setError(message);
      })
      .finally(() => setIsFetching(false));
  }, [code]);

  const handleAction = async (action: 'accept' | 'decline') => {
    if (!user) {
      router.push(`/login?returnTo=/invite/group/${code}`);
      return;
    }

    if (!invite) return;

    setIsProcessing(true);
    setError('');

    try {
      if (action === 'accept') {
        await acceptGroupInvite(code);
      } else {
        await declineGroupInvite(code);
      }
      router.push('/dashboard');
    } catch (err: unknown) {
      const message = err instanceof ApiError ? err.message : `Failed to ${action} invite`;
      setError(message);
    } finally {
      setIsProcessing(false);
    }
  };

  if (isAuthLoading || isFetching) {
    return (
      <div className="min-h-screen flex items-center justify-center bg-slate-50">
        <Loader2 className="w-8 h-8 animate-spin text-primary" />
      </div>
    );
  }

  if (error) {
    return (
      <div className="min-h-screen flex items-center justify-center bg-slate-50 p-4">
        <Card className="w-full max-w-md border-red-200">
          <CardHeader className="text-center">
            <XCircle className="w-12 h-12 text-red-500 mx-auto mb-2" />
            <CardTitle className="text-red-600">Error</CardTitle>
            <CardDescription>{error}</CardDescription>
          </CardHeader>
          <CardFooter className="flex justify-center">
            <Button variant="ghost" onClick={() => router.push('/')}>Go Home</Button>
          </CardFooter>
        </Card>
      </div>
    );
  }

  if (!invite) return null;

  return (
    <div className="min-h-screen flex items-center justify-center bg-slate-50 p-4">
      <Card className="w-full max-w-md shadow-lg border-0">
        <CardHeader className="text-center space-y-4">
          <div className="w-16 h-16 bg-primary/10 rounded-full flex items-center justify-center mx-auto text-primary">
            <CheckCircle2 className="w-8 h-8" />
          </div>
          <div>
            <CardTitle className="text-2xl">You’ve been invited!</CardTitle>
            <CardDescription className="mt-2 text-base">
              You have been invited to join the group <br />
              <span className="font-bold text-slate-900 text-lg">{invite.groupName || 'this group'}</span>
            </CardDescription>
          </div>
        </CardHeader>

        <CardContent className="text-center text-slate-500 text-sm">
          {user ? (
            <p>
              Signed in as <span className="font-medium text-slate-900">{user.email}</span>
            </p>
          ) : (
            <p>You need to sign in to accept this invitation.</p>
          )}
        </CardContent>

        <CardFooter className="flex flex-col gap-3">
          {user ? (
            <>
              <Button
                className="w-full"
                size="lg"
                onClick={() => handleAction('accept')}
                isLoading={isProcessing && invite ? true : false}
              >
                Accept Invitation
              </Button>
              <Button variant="ghost" className="w-full" onClick={() => handleAction('decline')} disabled={isProcessing}>
                Decline
              </Button>
            </>
          ) : (
            <Button className="w-full" size="lg" onClick={() => router.push(`/login?returnTo=/invite/group/${code}`)}>
              Log in to Accept
            </Button>
          )}
        </CardFooter>
      </Card>
    </div>
  );
}
