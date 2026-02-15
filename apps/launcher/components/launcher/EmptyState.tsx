import { Button } from '@/components/ui/button';
import { Card } from '@/components/ui/card';
import { Plus, Sparkles } from 'lucide-react';

export function EmptyState() {
    return (
        <div className="flex items-center justify-center py-16">
            <Card className="max-w-md p-8 text-center space-y-6">
                <div className="w-16 h-16 bg-primary/10 rounded-full flex items-center justify-center mx-auto">
                    <Sparkles className="w-8 h-8 text-primary" />
                </div>

                <div className="space-y-2">
                    <h3 className="text-xl font-bold text-slate-900">Welcome to FlexiSuite!</h3>
                    <p className="text-slate-500">
                        You don’t have any groups yet. Create your first group to get started.
                    </p>
                </div>

                <div className="space-y-3">
                    <Button className="w-full" size="lg">
                        <Plus className="w-4 h-4 mr-2" />
                        Create Your First Group
                    </Button>

                    <p className="text-xs text-slate-400">
                        or ask an admin to invite you to an existing group
                    </p>
                </div>
            </Card>
        </div>
    );
}
