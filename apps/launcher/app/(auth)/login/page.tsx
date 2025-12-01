'use client';

import { useState } from 'react';
import Link from 'next/link';
import { useAuth } from '@/lib/auth';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Card, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from '@/components/ui/card';
import { AlertCircle } from 'lucide-react';

export default function LoginPage() {
    const { login } = useAuth();
    const [email, setEmail] = useState('');
    const [password, setPassword] = useState('');
    const [isLoading, setIsLoading] = useState(false);
    const [error, setError] = useState('');

    const handleSubmit = async (e: React.FormEvent) => {
        e.preventDefault();
        setIsLoading(true);
        setError('');

        try {
            const res = await fetch(`${process.env.NEXT_PUBLIC_KERNEL_API}/auth/login`, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ email, password }),
            });

            const data = await res.json();
            console.log('[Login] API response:', data);

            if (!res.ok) {
                throw new Error(data.message || 'Login failed');
            }

            // If user data is not in the response, fetch it from /auth/me
            let userData = data.user;
            if (!userData) {
                console.log('[Login] User not in response, fetching from /auth/me');
                const meRes = await fetch(`${process.env.NEXT_PUBLIC_KERNEL_API}/auth/me`, {
                    headers: { Authorization: `Bearer ${data.accessToken}` }
                });
                if (meRes.ok) {
                    userData = await meRes.json();
                    console.log('[Login] User data from /auth/me:', userData);
                }
            }

            console.log('[Login] Calling login with:', {
                hasAccessToken: !!data.accessToken,
                hasRefreshToken: !!data.refreshToken,
                user: userData
            });
            login(data.accessToken, data.refreshToken, userData);
        } catch (err: any) {
            console.error('[Login] Error:', err);
            setError(err.message);
        } finally {
            setIsLoading(false);
        }
    };

    return (
        <div className="min-h-screen flex items-center justify-center p-4 bg-slate-50">
            <Card className="w-full max-w-md shadow-lg border-0">
                <CardHeader className="space-y-1 text-center">
                    <CardTitle className="text-2xl font-bold text-primary">FlexiSuite Lumina</CardTitle>
                    <CardDescription>
                        Enter your email to sign in to your workspace
                    </CardDescription>
                </CardHeader>
                <CardContent>
                    <form onSubmit={handleSubmit} className="space-y-4">
                        {error && (
                            <div className="bg-red-50 text-red-600 p-3 rounded-lg text-sm flex items-center gap-2">
                                <AlertCircle className="w-4 h-4" />
                                {error}
                            </div>
                        )}
                        <div className="space-y-2">
                            <label htmlFor="email" className="text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70">Email</label>
                            <Input
                                id="email"
                                type="email"
                                placeholder="m@example.com"
                                value={email}
                                onChange={(e) => setEmail(e.target.value)}
                                required
                            />
                        </div>
                        <div className="space-y-2">
                            <label htmlFor="password" className="text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70">Password</label>
                            <Input
                                id="password"
                                type="password"
                                value={password}
                                onChange={(e) => setPassword(e.target.value)}
                                required
                            />
                        </div>
                        <Button type="submit" className="w-full" isLoading={isLoading}>
                            Log In
                        </Button>
                    </form>
                </CardContent>
                <CardFooter className="flex justify-center">
                    <p className="text-sm text-slate-500">
                        Don't have an account?{' '}
                        <Link href="/signup" className="text-primary hover:underline font-medium">
                            Sign up
                        </Link>
                    </p>
                </CardFooter>
            </Card>
        </div>
    );
}
