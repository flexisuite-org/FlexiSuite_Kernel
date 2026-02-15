'use client';

import { createContext, useContext, useEffect, useState, useCallback } from 'react';
import { useRouter } from 'next/navigation';
import { getCookie, setCookie, removeCookie } from '@/lib/cookies';
import { UserProfile } from '@/types/api';
import { getProfile, logout as apiLogout, onUnauthorized } from '@/lib/apiClient';

interface AuthContextType {
  user: UserProfile | null;
  isLoading: boolean;
  login: (token: string, refreshToken: string, user: UserProfile) => void;
  logout: () => void;
  switchGroup: (groupId: string) => Promise<void>;
}

const AuthContext = createContext<AuthContextType | undefined>(undefined);

export function AuthProvider({ children }: { children: React.ReactNode }) {
  const [user, setUser] = useState<UserProfile | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const router = useRouter();

  const logout = useCallback(async () => {
    console.log('[Auth] Logout called');
    const token = getCookie('flexi_token');
    if (token) {
      try {
        await apiLogout();
      } catch (err) {
        console.warn('[Auth] Logout failed', err);
      }
    }
    removeCookie('flexi_token');
    removeCookie('flexi_refresh_token');
    setUser(null);
    router.push('/login');
  }, [router]);

  const switchGroup = useCallback(async (groupId: string) => {
    try {
      const { accessToken, refreshToken } = await import('@/lib/apiClient').then(m => m.switchGroup(groupId));
      setCookie('flexi_token', accessToken);
      setCookie('flexi_refresh_token', refreshToken);
      const profile = await import('@/lib/apiClient').then(m => m.getProfile());
      setUser(profile);
    } catch (err) {
      console.error('[Auth] failed to switch group', err);
    }
  }, []);

  useEffect(() => {
    let isMounted = true;
    const initAuth = async () => {
      try {
        const profile = await getProfile();
        if (isMounted) {
          setUser(profile);
        }
      } catch (err) {
        console.error('[Auth] failed to load profile', err);
      } finally {
        if (isMounted) setIsLoading(false);
      }
    };

    initAuth();
    return () => {
      isMounted = false;
    };
  }, []);

  useEffect(() => {
    const cleanup = onUnauthorized(() => {
      logout();
    });
    return cleanup;
  }, [logout]);

  const login = (token: string, refreshToken: string, userData: UserProfile) => {
    console.log('[Auth] Login called with user:', userData);
    setCookie('flexi_token', token);
    setCookie('flexi_refresh_token', refreshToken);
    setUser(userData);
    console.log('[Auth] Redirecting to /dashboard');
    router.push('/dashboard');
  };

  const value = { user, isLoading, login, logout, switchGroup };

  return (
    <AuthContext.Provider value={value}>
      {children}
    </AuthContext.Provider>
  );
}

export function useAuth() {
  const context = useContext(AuthContext);
  if (context === undefined) {
    throw new Error('useAuth must be used within an AuthProvider');
  }
  return context;
}
