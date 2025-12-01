import { AppInstallSummary } from '@/types/api';
import { Box } from 'lucide-react';

interface AppTileGridProps {
    apps: AppInstallSummary[];
    onAppClick: (packageId: string) => void;
}

export function AppTileGrid({ apps, onAppClick }: AppTileGridProps) {
    if (apps.length === 0) {
        return (
            <div className="flex flex-col items-center justify-center py-12 text-slate-400 border-2 border-dashed border-slate-200 rounded-xl">
                <Box className="w-12 h-12 mb-2 opacity-50" />
                <p>No apps installed in this group.</p>
            </div>
        );
    }

    return (
        <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 gap-4">
            {apps.map((app) => (
                <button
                    key={app.packageId}
                    onClick={() => {
                        // Open app in new tab
                        // In a real implementation, this might point to a specific URL for the app
                        // For now, we'll simulate it or use a placeholder URL
                        window.open(`/apps/${app.packageId}`, '_blank');
                        if (onAppClick) onAppClick(app.packageId);
                    }}
                    className="flex flex-col items-center justify-center p-6 rounded-xl bg-white border border-slate-100 shadow-sm hover:shadow-md hover:border-primary/20 hover:-translate-y-1 transition-all duration-300 group animate-fade-in"
                >
                    <div className="w-16 h-16 mb-4 rounded-2xl bg-gradient-to-br from-slate-50 to-slate-100 flex items-center justify-center text-2xl shadow-inner group-hover:scale-110 transition-transform duration-300">
                        {/* Placeholder icon logic - use first letter */}
                        <span className="font-bold text-slate-700 group-hover:text-primary transition-colors">
                            {app.name.charAt(0).toUpperCase()}
                        </span>
                    </div>
                    <span className="text-sm font-medium text-slate-700 group-hover:text-slate-900 text-center line-clamp-2">
                        {app.name}
                    </span>
                </button>
            ))}
        </div>
    );
}
