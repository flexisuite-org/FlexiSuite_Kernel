'use client';

import { useCallback, useEffect, useState } from 'react';
import { RegistryPackageSummary } from '@/types/api';
import { getRegistryPackages, ApiError } from '@/lib/apiClient';

export function useStorePackages() {
  const [packages, setPackages] = useState<RegistryPackageSummary[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setIsLoading(true);
    setError(null);
    try {
      const data = await getRegistryPackages();
      setPackages(data);
    } catch (err) {
      if (err instanceof ApiError) {
        setError(err.message);
      } else {
        setError('Unable to load store packages.');
        console.error(err);
      }
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  return { packages, isLoading, error, refresh };
}
