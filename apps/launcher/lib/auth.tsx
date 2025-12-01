'use client';

import { createContext, useContext, useEffect, useState, useCallback } from 'react';
import { useRouter, usePathname } from 'next/navigation';
import { getCookie, setCookie, removeCookie } from '@/lib/cookies';
import { UserProfile } from '@/types/api';

interface AuthContextType {
  user: UserProfile | null;
  isLoading: boolean;
  login: (token: string, refreshToken: string, user: UserProfile) => void;
  logout: () => void;
}

const AuthContext = createContext<AuthContextType | undefined>(undefined);

export function AuthProvider({ children }: { children: React.ReactNode }) {
  const [user, setUser] = useState<UserProfile | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const router = useRouter();
  const pathname = usePathname();

  const logout = useCallback(() => {
    console.log('[Auth] Logout called');
    removeCookie('flexi_token');
    removeCookie('flexi_refresh_token');
    setUser(null);
    router.push('/login');
  }, [router]);

  useEffect(() => {
    const initAuth = async () => {
      const token = getCookie('flexi_token');
      console.log('[Auth] Checking token:', token ? 'exists' : 'missing');

      if (!token) {
        console.log('[Auth] No token found, user not logged in');
        setIsLoading(false);
        return;
      }

      const apiUrl = process.env.NEXT_PUBLIC_KERNEL_API;
      console.log('[Auth] API URL:', apiUrl);

      try {
        const res = await fetch(`${apiUrl}/auth/me`, {
          headers: { Authorization: `Bearer ${token}` }
        });

        console.log('[Auth] /auth/me response status:', res.status);

        if (res.ok) {
          const userData = await res.json();
          console.log('[Auth] User data received:', userData);
          setUser(userData);
        } else {
          console.log('[Auth] Auth check failed, logging out');
          const errorText = await res.text();
          console.log('[Auth] Error response:', errorText);
          logout();
        }
      } catch (e) {
        console.error('[Auth] Auth check failed with error:', e);
        setIsLoading(false);
      } finally {
        setIsLoading(false);
      }
    };

    initAuth();
  }, [logout]);

  const login = (token: string, refreshToken: string, userData: UserProfile) => {
    console.log('[Auth] Login called with user:', userData);
    setCookie('flexi_token', token);
    setCookie('flexi_refresh_token', refreshToken);
    setUser(userData);
    console.log('[Auth] Redirecting to /dashboard');
    router.push('/dashboard');
  };

  const value = { user, isLoading, login, logout };

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
