export interface UserProfile {
    userId: string;
    email: string;
    roles: string[];
    memberships: GroupMembership[];
}

export interface GroupMembership {
    groupId: string;
    name: string;
    type: string;
    role: string;
}

export interface LauncherGroup {
    groupId: string;
    name: string;
    type: string;
    installs: AppInstallSummary[];
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

export interface AuthResponse {
    accessToken: string;
    refreshToken: string;
    user: UserProfile;
}
