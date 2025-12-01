import { GroupInvite } from '@/types/api';
import { Button } from '@/components/ui/button';
import { Mail } from 'lucide-react';
import Link from 'next/link';

interface PendingInvitesSectionProps {
    invites: GroupInvite[];
}

export function PendingInvitesSection({ invites }: PendingInvitesSectionProps) {
    if (invites.length === 0) return null;

    return (
        <div className="mb-8">
            <h3 className="text-sm font-medium text-slate-500 mb-3 px-1">Pending Invites</h3>
            <div className="grid gap-3">
                {invites.map((invite) => (
                    <div
                        key={invite.id}
                        className="flex items-center justify-between p-4 bg-white border border-primary/20 rounded-xl shadow-sm bg-gradient-to-r from-primary/5 to-transparent"
                    >
                        <div className="flex items-center gap-3">
                            <div className="w-10 h-10 rounded-full bg-white flex items-center justify-center text-primary shadow-sm border border-primary/10">
                                <Mail className="w-5 h-5" />
                            </div>
                            <div>
                                <p className="text-sm font-medium text-slate-900">
                                    Invited to <span className="font-bold text-primary">{invite.groupName || 'a group'}</span>
                                </p>
                                <p className="text-xs text-slate-500">
                                    by {invite.inviter?.email || 'an admin'}
                                </p>
                            </div>
                        </div>
                        <Link href={`/invite/group/${invite.code}`}>
                            <Button size="sm" variant="secondary">
                                View
                            </Button>
                        </Link>
                    </div>
                ))}
            </div>
        </div>
    );
}
