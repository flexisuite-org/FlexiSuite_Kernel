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
import { Button } from '@/components/ui/button';

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
        <div className="min-h-screen bg-slate-50 flex font-sans">
            {/* Sidebar Area */}
            <aside className="w-72 bg-white/80 backdrop-blur-xl border-r border-slate-200/60 hidden md:flex flex-col fixed inset-y-0 z-30 shadow-soft transition-all duration-300">
                {/* Logo Section */}
                <div className="h-20 flex items-center px-6 border-b border-slate-100/50">
                    <div className="flex items-center gap-3 group cursor-pointer">
                        <div className="w-10 h-10 bg-gradient-to-br from-primary to-pink-600 rounded-xl flex items-center justify-center shadow-glow group-hover:scale-105 transition-transform duration-300">
                            <Sparkles className="w-5 h-5 text-white" />
                        </div>
                        <div className="flex flex-col">
                            <span className="font-bold text-lg text-slate-900 tracking-tight leading-none">FlexiSuite</span>
                            <span className="text-xs font-medium text-primary tracking-widest uppercase mt-0.5">Lumina</span>
                        </div>
                    </div>
                </div>

                {/* Navigation */}
                <div className="flex-1 flex flex-col py-6 px-4 gap-1 overflow-y-auto">
                    <div className="mb-4 px-2 text-[10px] font-bold text-slate-400 uppercase tracking-widest">
                        Main Menu
                    </div>
                    {NAV_ITEMS.map((item) => {
                        const isActive = pathname === item.href;
                        return (
                            <Link
                                key={item.href}
                                href={item.href}
                                className={cn(
                                    "flex items-center gap-3 px-4 py-3 rounded-xl text-sm font-medium transition-all duration-200 group relative overflow-hidden",
                                    isActive
                                        ? "bg-primary/5 text-primary shadow-sm"
                                        : "text-slate-600 hover:bg-slate-50 hover:text-slate-900"
                                )}
                            >
                                {isActive && (
                                    <div className="absolute left-0 top-1/2 -translate-y-1/2 w-1 h-8 bg-primary rounded-r-full" />
                                )}
                                <item.icon className={cn("w-5 h-5 transition-colors", isActive ? "text-primary" : "text-slate-400 group-hover:text-slate-600")} />
                                {item.label}
                            </Link>
                        );
                    })}

                    <div className="mt-auto pt-6 border-t border-slate-100/50">
                        <Link
                            href="/help"
                            className="flex items-center gap-3 px-4 py-3 rounded-xl text-sm font-medium text-slate-600 hover:bg-slate-50 hover:text-slate-900 transition-colors"
                        >
                            <HelpCircle className="w-5 h-5 text-slate-400" />
                            Help & Documentation
                        </Link>
                    </div>
                </div>

                {/* User Profile Section */}
                <div className="p-4 border-t border-slate-100/50 bg-slate-50/30 backdrop-blur-sm">
                    <div className="relative">
                        <button
                            onClick={() => setIsUserMenuOpen(!isUserMenuOpen)}
                            className={cn(
                                "w-full flex items-center gap-3 p-2 rounded-xl transition-all duration-200 border border-transparent hover:bg-white hover:shadow-sm hover:border-slate-100",
                                isUserMenuOpen && "bg-white shadow-sm border-slate-100"
                            )}
                        >
                            <div className="w-10 h-10 rounded-full bg-gradient-to-tr from-slate-100 to-slate-200 border border-white shadow-sm flex items-center justify-center text-slate-600 font-bold text-sm">
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
                            <div className="absolute bottom-full left-0 w-full mb-2 p-1 bg-white/90 backdrop-blur-xl rounded-2xl border border-white/50 shadow-xl animate-scale-in origin-bottom">
                                <Link href="/settings" className="flex items-center gap-2 px-3 py-2.5 text-sm font-medium text-slate-600 hover:bg-slate-50 hover:text-slate-900 rounded-xl transition-colors">
                                    <User className="w-4 h-4" />
                                    Account Settings
                                </Link>
                                <div className="h-px bg-slate-100 my-1" />
                                <button
                                    onClick={logout}
                                    className="w-full flex items-center gap-2 px-3 py-2.5 text-sm font-medium text-slate-600 hover:bg-primary hover:text-white rounded-xl transition-all duration-200 group"
                                >
                                    <LogOut className="w-4 h-4 group-hover:text-white transition-colors" />
                                    Sign out
                                </button>
                            </div>
                        )}
                    </div>
                </div>
            </aside>

            {/* Main Content */}
            <main className="flex-1 md:ml-72 min-h-screen flex flex-col">
                {/* Top Header for Mobile/Notifications */}
                <header className="h-20 flex items-center justify-end px-8 sticky top-0 z-20 bg-slate-50/80 backdrop-blur-md">
                    <div className="flex items-center gap-4">
                        <button className="relative p-2.5 rounded-full text-slate-500 hover:bg-white hover:text-primary hover:shadow-sm transition-all duration-200">
                            <Bell className="w-5 h-5" />
                            {hasNotifications && (
                                <span className="absolute top-2 right-2.5 w-2 h-2 bg-primary rounded-full border-2 border-slate-50" />
                            )}
                        </button>
                    </div>
                </header>

                <div className="flex-1 px-8 pb-8 max-w-7xl mx-auto w-full animate-fade-in">
                    {children}
                </div>
            </main>
        </div>
    );
}
