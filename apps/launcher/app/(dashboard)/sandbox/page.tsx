import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Box, Plus } from "lucide-react";
import { Button } from "@/components/ui/button";

export default function SandboxPage() {
    return (
        <div className="space-y-8">
            <header className="flex items-center justify-between pb-6 border-b border-slate-100">
                <div>
                    <h1 className="text-2xl font-bold text-slate-900">Sandbox & Drafts</h1>
                    <p className="text-slate-500">Manage your development sessions and draft components</p>
                </div>
                <Button>
                    <Plus className="w-4 h-4 mr-2" />
                    New Draft
                </Button>
            </header>

            <div className="flex flex-col items-center justify-center py-16 text-center border-2 border-dashed border-slate-200 rounded-xl bg-slate-50/50">
                <div className="w-16 h-16 bg-slate-100 rounded-full flex items-center justify-center mb-4">
                    <Box className="w-8 h-8 text-slate-400" />
                </div>
                <h3 className="text-lg font-semibold text-slate-900">No active drafts</h3>
                <p className="text-slate-500 max-w-sm mt-2 mb-6">
                    You haven't created any draft components yet. Start building your custom tools today.
                </p>
                <Button variant="outline">Learn about Custom UX</Button>
            </div>
        </div>
    );
}
