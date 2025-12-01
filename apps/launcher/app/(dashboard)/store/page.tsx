import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { ShoppingBag, Search, Filter } from "lucide-react";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";

export default function StorePage() {
    return (
        <div className="space-y-8">
            <header className="flex flex-col md:flex-row md:items-center justify-between gap-4 pb-6 border-b border-slate-100">
                <div>
                    <h1 className="text-2xl font-bold text-slate-900">App Store</h1>
                    <p className="text-slate-500">Discover and install new components for your group</p>
                </div>
                <div className="flex gap-2">
                    <div className="relative w-full md:w-64">
                        <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-slate-400" />
                        <Input placeholder="Search apps..." className="pl-9" />
                    </div>
                    <Button variant="outline" size="icon">
                        <Filter className="w-4 h-4" />
                    </Button>
                </div>
            </header>

            <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
                {/* Placeholder Items */}
                {[1, 2, 3, 4, 5, 6].map((i) => (
                    <Card key={i} className="hover:shadow-md transition-shadow">
                        <CardHeader className="flex flex-row items-start gap-4 space-y-0">
                            <div className="w-12 h-12 bg-slate-100 rounded-lg flex items-center justify-center text-slate-400">
                                <ShoppingBag className="w-6 h-6" />
                            </div>
                            <div className="flex-1">
                                <CardTitle className="text-base">Sample App {i}</CardTitle>
                                <CardDescription className="line-clamp-2 mt-1">
                                    This is a placeholder for a store application description. It does amazing things.
                                </CardDescription>
                            </div>
                        </CardHeader>
                        <CardContent>
                            <Button className="w-full" variant="secondary">Install</Button>
                        </CardContent>
                    </Card>
                ))}
            </div>
        </div>
    );
}
