import { GroupMembership } from '@/types/api';
import { cn } from '@/lib/utils';
import { Building2, Plus } from 'lucide-react';

interface GroupSwitcherProps {
    groups: GroupMembership[];
    currentGroupId: string | null;
    onGroupSelect: (groupId: string) => void;
}

export function GroupSwitcher({ groups, currentGroupId, onGroupSelect }: GroupSwitcherProps) {
    return (
        <div className="flex flex-col gap-2 py-4">
            <div className="px-4 pb-2">
                <h2 className="text-xs font-semibold text-slate-400 uppercase tracking-wider">Groups</h2>
            </div>
            <div className="space-y-1 px-2">
                {groups.map((group) => (
                    <button
                        key={group.groupId}
                        onClick={() => onGroupSelect(group.groupId)}
                        className={cn(
                            "w-full flex items-center gap-3 px-3 py-2 rounded-lg text-sm font-medium transition-colors",
                            currentGroupId === group.groupId
                                ? "bg-primary/10 text-primary"
                                : "text-slate-600 hover:bg-slate-100 hover:text-slate-900"
                        )}
                    >
                        <div className={cn(
                            "w-8 h-8 rounded-md flex items-center justify-center shrink-0",
                            currentGroupId === group.groupId ? "bg-primary text-white" : "bg-slate-200 text-slate-500"
                        )}>
                            <Building2 className="w-4 h-4" />
                        </div>
                        <span className="truncate">{group.name}</span>
                    </button>
                ))}

                <button className="w-full flex items-center gap-3 px-3 py-2 rounded-lg text-sm font-medium text-slate-500 hover:bg-slate-100 hover:text-slate-900 transition-colors border border-dashed border-slate-200 mt-2">
                    <div className="w-8 h-8 rounded-md flex items-center justify-center shrink-0 border border-slate-300">
                        <Plus className="w-4 h-4" />
                    </div>
                    <span>Create Group</span>
                </button>
            </div>
        </div>
    );
}
