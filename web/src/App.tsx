import { Suspense, lazy } from 'react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { BrowserRouter, Route, Routes } from 'react-router-dom'

import { AppShell } from '@/components/app-shell'
import { Skeleton } from '@/components/ui/skeleton'
import { Toaster } from '@/components/ui/sonner'
import { TooltipProvider } from '@/components/ui/tooltip'

const HomePage = lazy(() => import('@/pages/home-page').then((module) => ({ default: module.HomePage })))
const JobsPage = lazy(() => import('@/pages/jobs-page').then((module) => ({ default: module.JobsPage })))
const JobDetailPage = lazy(() => import('@/pages/job-detail-page').then((module) => ({ default: module.JobDetailPage })))
const NotFoundPage = lazy(() => import('@/pages/not-found-page').then((module) => ({ default: module.NotFoundPage })))

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 5_000,
      retry: 1,
    },
  },
})

export function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <TooltipProvider>
        <BrowserRouter>
          <AppShell>
            <Suspense fallback={<PageSkeleton />}>
              <Routes>
                <Route path="/" element={<HomePage />} />
                <Route path="/jobs" element={<JobsPage />} />
                <Route path="/jobs/:jobId" element={<JobDetailPage />} />
                <Route path="*" element={<NotFoundPage />} />
              </Routes>
            </Suspense>
          </AppShell>
        </BrowserRouter>
        <Toaster richColors position="bottom-right" />
      </TooltipProvider>
    </QueryClientProvider>
  )
}

function PageSkeleton() {
  return (
    <div className="space-y-5" aria-label="Cargando página">
      <Skeleton className="h-8 w-48" />
      <Skeleton className="h-52 w-full" />
    </div>
  )
}
