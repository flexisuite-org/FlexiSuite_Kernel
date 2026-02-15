'use client';

import { useCallback, useEffect, useState } from 'react';
import { GroupInvite } from '@/types/api';
import { getPendingInvites, ApiError } from '@/lib/apiClient';

export function usePendingInvites() {
  const [invites, setInvites] = useState<GroupInvite[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setIsLoading(true);
    setError(null);
    try {
      const data = await getPendingInvites();
      setInvites(data);
    } catch (err) {
      if (err instanceof ApiError) {
        setError(err.message);
      } else {
        setError('Failed to load invites.');
        console.error(err);
      }
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  return { invites, isLoading, error, refresh };
}
