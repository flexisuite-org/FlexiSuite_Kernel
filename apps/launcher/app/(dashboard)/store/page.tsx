import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Input } from '@/components/ui/input';
import { Button } from '@/components/ui/button';
import { Search, Filter, Loader2 } from 'lucide-react';
import { useStorePackages } from '@/hooks/useStorePackages';

export default function StorePage() {
  const { packages, isLoading, error } = useStorePackages();

  return (
    <div className="space-y-8">
      <header className="flex flex-col md:flex-row md:items-center justify-between gap-4 pb-6 border-b border-slate-100">
        <div>
          <h1 className="text-2xl font-bold text-slate-900">App Store</h1>
          <p className="text-slate-500">Discover and install new components for your group</p>
        </div>
        <div className="flex gap-2 w-full md:w-auto">
          <div className="relative flex-1 md:flex-none">
            <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-slate-400" />
            <Input placeholder="Search apps..." className="pl-9" />
          </div>
          <Button variant="outline" size="icon">
            <Filter className="w-4 h-4" />
          </Button>
        </div>
      </header>

      {isLoading ? (
        <div className="flex items-center justify-center py-16">
          <Loader2 className="w-8 h-8 animate-spin text-slate-400" />
        </div>
      ) : error ? (
        <div className="rounded-xl border border-red-100 bg-red-50 p-4 text-sm text-red-700">
          {error}
        </div>
      ) : packages.length === 0 ? (
        <div className="rounded-xl border border-dashed border-slate-200 bg-slate-50 p-8 text-center text-slate-500">
          No store packages available right now. Check back later.
        </div>
      ) : (
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
          {packages.map((pkg) => (
            <Card key={pkg.id} className="hover:shadow-md transition-shadow">
              <CardHeader className="flex items-start gap-4 space-y-0">
                <div className="w-12 h-12 bg-slate-100 rounded-lg flex items-center justify-center text-slate-400">
                  {pkg.iconUrl ? (
                    <img src={pkg.iconUrl} alt={`${pkg.name} icon`} className="w-6 h-6" />
                  ) : (
                    <span className="text-base font-semibold uppercase text-slate-500">
                      {pkg.name.charAt(0)}
                    </span>
                  )}
                </div>
                <div className="flex-1">
                  <CardTitle className="text-base">{pkg.name}</CardTitle>
                  <CardDescription className="line-clamp-2 mt-1 text-slate-500">
                    {pkg.description || 'No description provided yet.'}
                  </CardDescription>
                </div>
              </CardHeader>
              <CardContent className="flex flex-col gap-2">
                <div className="flex items-center justify-between text-xs text-slate-500">
                  <span>{pkg.category || 'General'}</span>
                  <span>{pkg.publisher || 'Kernel team'}</span>
                </div>
                <div className="flex items-center justify-between">
                  <span className="text-[11px] uppercase tracking-wide text-slate-400">
                    {pkg.status || 'approved'}
                  </span>
                  <Button variant="secondary" size="sm">
                    Install
                  </Button>
                </div>
              </CardContent>
            </Card>
          ))}
        </div>
      )}
    </div>
  );
}
