import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Mail, ArrowRight } from "lucide-react";
import { Button } from "@/components/ui/button";

export default function InvitesPage() {
    return (
        <div className="space-y-8">
            <header className="flex items-center justify-between pb-6 border-b border-slate-100">
                <div>
                    <h1 className="text-2xl font-bold text-slate-900">Invites</h1>
                    <p className="text-slate-500">Manage your pending invitations and sent requests</p>
                </div>
            </header>

            <div className="grid gap-6">
                <Card>
                    <CardHeader>
                        <CardTitle className="text-lg">Pending Invites</CardTitle>
                        <CardDescription>Invitations waiting for your response</CardDescription>
                    </CardHeader>
                    <CardContent>
                        <div className="flex flex-col items-center justify-center py-8 text-center text-slate-500">
                            <Mail className="w-10 h-10 mb-3 opacity-20" />
                            <p>No pending invites at the moment.</p>
                        </div>
                    </CardContent>
                </Card>

                <Card>
                    <CardHeader>
                        <div className="flex items-center justify-between">
                            <div>
                                <CardTitle className="text-lg">Invite Users</CardTitle>
                                <CardDescription>Invite colleagues to your groups</CardDescription>
                            </div>
                            <Button variant="outline">
                                Send Invite <ArrowRight className="w-4 h-4 ml-2" />
                            </Button>
                        </div>
                    </CardHeader>
                </Card>
            </div>
        </div>
    );
}
