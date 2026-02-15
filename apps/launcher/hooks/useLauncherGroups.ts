'use client';

import { useCallback, useEffect, useState } from 'react';
import { LauncherGroup } from '@/types/api';
import { getLauncherGroups, ApiError } from '@/lib/apiClient';

export function useLauncherGroups() {
  const [groups, setGroups] = useState<LauncherGroup[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setIsLoading(true);
    setError(null);
    try {
      const data = await getLauncherGroups();
      setGroups(data);
    } catch (err) {
      if (err instanceof ApiError) {
        setError(err.message);
      } else {
        setError('Unable to load groups.');
        console.error(err);
      }
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  return { groups, isLoading, error, refresh };
}
