'use client';

import { getCookie, removeCookie } from '@/lib/cookies';
import {
  AppInstallSummary,
  AuthResponse,
  GroupInvite,
  GroupInviteAcceptResponse,
  GroupInviteDeclineResponse,
  LauncherGroup,
  RegistryPackageSummary,
  UserProfile
} from '@/types/api';

const BASE_URL = process.env.NEXT_PUBLIC_KERNEL_API;
const UNAUTHORIZED_EVENT = 'flexi-unauthorized';

if (!BASE_URL) {
  throw new Error('NEXT_PUBLIC_KERNEL_API is required for API calls');
}

export class ApiError extends Error {
  constructor(public status: number, message: string) {
    super(message);
    this.name = 'ApiError';
  }
}

interface ApiRequestOptions extends Omit<RequestInit, 'headers'> {
  headers?: HeadersInit;
  body?: BodyInit | Record<string, unknown>;
}

function buildUrl(path: string) {
  const trimmed = path.startsWith('/') ? path : `/${path}`;
  return `${BASE_URL}${trimmed}`;
}

function dispatchUnauthorized() {
  if (typeof window === 'undefined') return;
  removeCookie('flexi_token');
  removeCookie('flexi_refresh_token');
  window.dispatchEvent(new Event(UNAUTHORIZED_EVENT));
  if (window.location.pathname !== '/login') {
    window.location.assign('/login');
  }
}

async function parseJson(response: Response): Promise<unknown> {
  const text = await response.text();
  if (!text) return null;
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

function hasErrorMessage(value: unknown): value is { message?: string } {
  if (typeof value !== 'object' || value === null) return false;
  return (
    'message' in value &&
    typeof (value as Record<string, unknown>).message === 'string'
  );
}

async function request<T = unknown>(path: string, options: ApiRequestOptions = {}): Promise<T> {
  const url = buildUrl(path);
  const headers = new Headers(options.headers ?? {});
  const token = getCookie('flexi_token');

  if (token) {
    headers.set('Authorization', `Bearer ${token}`);
  }

  if (options.body && !headers.has('Content-Type')) {
    if (
      typeof options.body === 'string' ||
      options.body instanceof FormData ||
      options.body instanceof URLSearchParams
    ) {
      // leave headers alone
    } else {
      headers.set('Content-Type', 'application/json');
    }
  }

  const body =
    options.body &&
    headers.get('Content-Type')?.includes('application/json') &&
    typeof options.body !== 'string' &&
    !(options.body instanceof FormData)
      ? JSON.stringify(options.body)
      : (options.body as BodyInit | undefined);

  const response = await fetch(url, {
    ...options,
    headers,
    body,
  });

  const payload = await parseJson(response);

  if (!response.ok) {
    if (response.status === 401) {
      dispatchUnauthorized();
    }

    let message = response.statusText;
    if (typeof payload === 'string') {
      message = payload;
    } else if (hasErrorMessage(payload) && payload.message) {
      message = payload.message;
    }

    throw new ApiError(response.status, message || 'Request failed');
  }

  return payload as T;
}

export function onUnauthorized(cb: () => void) {
  if (typeof window === 'undefined') return;
  window.addEventListener(UNAUTHORIZED_EVENT, cb);
  return () => window.removeEventListener(UNAUTHORIZED_EVENT, cb);
}

export async function login(payload: { email: string; password: string }) {
  return request<AuthResponse>('/auth/login', {
    method: 'POST',
    body: payload,
  });
}

export async function signup(payload: { email: string; password: string; accountInviteCode: string }) {
  return request<AuthResponse>('/auth/signup', {
    method: 'POST',
    body: payload,
  });
}

export async function logout() {
  return request<void>('/auth/logout', { method: 'POST' });
}

export async function refreshTokens() {
  const token = getCookie('flexi_refresh_token');
  if (!token) {
    throw new ApiError(401, 'Missing refresh token');
  }
  return request<AuthResponse>('/auth/refresh', {
    method: 'POST',
    body: { refreshToken: token },
  });
}

export async function switchGroup(groupId: string) {
  const profile = await getProfile();
  const refreshToken = getCookie('flexi_refresh_token');
  if (!refreshToken) {
    throw new ApiError(401, 'Missing refresh token');
  }
  return request<AuthResponse>('/auth/switch', {
    method: 'POST',
    body: { userId: profile.userId, refreshToken, groupId },
  });
}

export async function getProfile() {
  return request<UserProfile>('/auth/me');
}

export async function getLauncherGroups() {
  return request<LauncherGroup[]>('/launcher/groups');
}

export async function getGroupInstalls(groupId: string) {
  return request<AppInstallSummary[]>(`/groups/${encodeURIComponent(groupId)}/installs`);
}

export async function getPendingInvites() {
  return request<GroupInvite[]>('/invites/pending');
}

export async function getGroupInvite(code: string) {
  return request<GroupInvite>(`/group-invites/link/${encodeURIComponent(code)}`);
}

export async function acceptGroupInvite(code: string) {
  return request<GroupInviteAcceptResponse>(`/group-invites/${encodeURIComponent(code)}/accept`, {
    method: 'POST',
  });
}

export async function declineGroupInvite(code: string) {
  return request<GroupInviteDeclineResponse>(`/group-invites/${encodeURIComponent(code)}/decline`, {
    method: 'POST',
  });
}

export async function getRegistryPackages() {
  return request<RegistryPackageSummary[]>('/registry/packages');
}
