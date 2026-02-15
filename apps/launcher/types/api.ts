export interface UserProfile {
    userId: string;
    email: string;
    currentGroupId: string | null;
    roles: string[];
    memberships: GroupMembership[];
}

export interface GroupMembership {
    groupId: string;
    name: string | null;
    type: string | null;
    membershipRoles: string[];
}

export interface LauncherGroup {
    groupId: string;
    name: string;
    type: string;
    installs: AppInstallSummary[];
    insights?: LauncherInsights;
}

export interface AppInstallSummary {
    installId: string;
    packageId: string;
    name: string;
    iconUrl?: string;
}

export interface GroupInvite {
    id: string;
    code: string;
    groupId: string;
    groupName?: string;
    kind: 'LINK' | 'EMAIL';
    inviter?: { userId: string; email?: string };
    expiresAt?: string;
}

export interface GroupInviteAcceptResponse {
    accepted: boolean;
    groupId: string;
    roles: string[];
}

export interface GroupInviteDeclineResponse {
    declined: boolean;
}

export interface AuthResponse {
    accessToken: string;
    refreshToken: string;
    user: UserProfile;
}

export interface RegistryPackageSummary {
    id: string;
    name: string;
    description?: string;
    category?: string;
    publisher?: string;
    iconUrl?: string;
    status?: 'draft' | 'approved' | 'revoked' | 'deprecated' | string;
}

export interface LauncherInsights {
    recentApps?: AppInstallSummary[];
    pinnedApps?: AppInstallSummary[];
    recommendations?: string[];
    jobs?: JobSummary[];
    drafts?: DraftSummary[];
}

export interface JobSummary {
    jobId: string;
    title: string;
    status: 'running' | 'queued' | 'completed' | 'failed' | 'paused' | string;
    message?: string;
    progress?: number;
    updatedAt?: string;
}

export interface DraftSummary {
    id: string;
    title: string;
    status: string;
    updatedAt: string;
    owner?: string;
}
