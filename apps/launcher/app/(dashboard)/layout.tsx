'use client';

import { useEffect, useState } from 'react';
import { useRouter, usePathname } from 'next/navigation';
import Link from 'next/link';
import { useAuth } from '@/lib/auth';
import {
    Loader2,
    Home,
    ShoppingBag,
    Box,
    Mail,
    Settings,
    HelpCircle,
    LogOut,
    Bell,
    ChevronRight,
    User,
    Sparkles
} from 'lucide-react';
import { cn } from '@/lib/utils';

const NAV_ITEMS = [
    { label: 'Home', href: '/dashboard', icon: Home },
    { label: 'Store', href: '/store', icon: ShoppingBag },
    { label: 'Sandbox', href: '/sandbox', icon: Box },
    { label: 'Invites', href: '/invites', icon: Mail },
    { label: 'Settings', href: '/settings', icon: Settings },
];

export default function DashboardLayout({
    children,
}: {
    children: React.ReactNode;
}) {
    const { user, isLoading, logout } = useAuth();
    const router = useRouter();
    const pathname = usePathname();
    const [isUserMenuOpen, setIsUserMenuOpen] = useState(false);
    const [hasNotifications, setHasNotifications] = useState(true); // Mock notification state

    useEffect(() => {
        if (!isLoading && !user) {
            router.push('/login');
        }
    }, [user, isLoading, router]);

    if (isLoading) {
        return (
            <div className="min-h-screen flex items-center justify-center bg-slate-50">
                <Loader2 className="w-8 h-8 animate-spin text-primary" />
            </div>
        );
    }

    if (!user) return null;

    return (
        <div className="min-h-screen bg-slate-50 flex font-sans text-slate-800">
            {/* Sidebar Area */}
            <aside className="w-72 bg-white border-r border-slate-200 hidden md:flex flex-col fixed inset-y-0 z-30 transition-all duration-300">
                {/* Logo Section */}
                <div className="h-16 flex items-center px-6 border-b border-slate-100">
                    <Link href="/dashboard" className="flex items-center gap-3 group">
                        <div className="w-8 h-8 bg-primary/10 rounded-lg flex items-center justify-center group-hover:bg-primary/20 transition-colors">
                            <Sparkles className="w-4 h-4 text-primary" />
                        </div>
                        <span className="font-bold text-lg tracking-tight text-slate-900">FlexiSuite Lumina</span>
                    </Link>
                </div>

                {/* Navigation */}
                <div className="flex-1 flex flex-col py-6 px-3 gap-1 overflow-y-auto">
                    <div className="mb-2 px-4 text-[10px] font-bold text-slate-400 uppercase tracking-widest">
                        Menu
                    </div>
                    {NAV_ITEMS.map((item) => {
                        const isActive = pathname === item.href;
                        return (
                            <Link
                                key={item.href}
                                href={item.href}
                                className={cn(
                                    "flex items-center gap-3 px-4 py-2.5 rounded-lg text-sm font-medium transition-all duration-200 relative group",
                                    isActive
                                        ? "bg-primary/5 text-primary"
                                        : "text-slate-600 hover:bg-slate-50 hover:text-slate-900"
                                )}
                            >
                                {isActive && (
                                    <div className="absolute left-0 top-1/2 -translate-y-1/2 w-1 h-6 bg-primary rounded-r-full" />
                                )}
                                <item.icon className={cn("w-4 h-4 transition-colors", isActive ? "text-primary" : "text-slate-400 group-hover:text-slate-600")} />
                                {item.label}
                            </Link>
                        );
                    })}

                    <div className="mt-auto pt-6 border-t border-slate-100 mx-3">
                        <Link
                            href="/help"
                            className="flex items-center gap-3 px-4 py-2.5 rounded-lg text-sm font-medium text-slate-600 hover:bg-slate-50 hover:text-slate-900 transition-colors"
                        >
                            <HelpCircle className="w-4 h-4 text-slate-400" />
                            Help & Docs
                        </Link>
                    </div>
                </div>

                {/* User Profile Section */}
                <div className="p-4 border-t border-slate-100 bg-white">
                    <div className="relative">
                        <button
                            onClick={() => setIsUserMenuOpen(!isUserMenuOpen)}
                            className={cn(
                                "w-full flex items-center gap-3 p-2 rounded-lg transition-all duration-200 border border-transparent hover:bg-slate-50",
                                isUserMenuOpen && "bg-slate-50"
                            )}
                        >
                            <div className="w-9 h-9 rounded-full bg-slate-100 border border-white shadow-sm flex items-center justify-center text-slate-600 font-bold text-xs">
                                {user.email[0].toUpperCase()}
                            </div>
                            <div className="flex-1 min-w-0 text-left">
                                <div className="text-sm font-semibold text-slate-900 truncate">{user.email.split('@')[0]}</div>
                                <div className="text-xs text-slate-500 truncate">Free Plan</div>
                            </div>
                            <ChevronRight className={cn("w-4 h-4 text-slate-400 transition-transform duration-200", isUserMenuOpen && "rotate-90")} />
                        </button>

                        {/* User Menu Dropdown */}
                        {isUserMenuOpen && (
                            <div className="absolute bottom-full left-0 w-full mb-2 p-1 bg-white rounded-xl border border-slate-200 shadow-lg animate-scale-in origin-bottom z-50">
                                <Link href="/settings" className="flex items-center gap-2 px-3 py-2 text-sm font-medium text-slate-600 hover:bg-slate-50 hover:text-slate-900 rounded-lg transition-colors">
                                    <User className="w-4 h-4" />
                                    Account Settings
                                </Link>
                                <div className="h-px bg-slate-100 my-1" />
                                <button
                                    onClick={logout}
                                    className="w-full flex items-center gap-2 px-3 py-2 text-sm font-medium text-slate-600 hover:bg-red-50 hover:text-red-600 rounded-lg transition-all duration-200"
                                >
                                    <LogOut className="w-4 h-4" />
                                    Sign out
                                </button>
                            </div>
                        )}
                    </div>
                </div>
            </aside>

            {/* Main Content */}
            <main className="flex-1 md:ml-72 min-h-screen flex flex-col">
                {/* Top Header */}
                <header className="h-16 flex items-center justify-end px-8 sticky top-0 z-20 bg-slate-50/90 backdrop-blur-sm border-b border-transparent">
                    <div className="flex items-center gap-4">
                        <button className="relative p-2 rounded-full text-slate-400 hover:bg-white hover:text-primary hover:shadow-sm transition-all duration-200">
                            <Bell className="w-5 h-5" />
                            {hasNotifications && (
                                <span className="absolute top-2 right-2 w-2 h-2 bg-primary rounded-full border-2 border-slate-50" />
                            )}
                        </button>
                    </div>
                </header>

                <div className="flex-1 px-8 pb-12 max-w-6xl mx-auto w-full animate-fade-in">
                    {children}
                </div>
            </main>
        </div>
    );
}
