import { useMemo, useState } from 'react'
import { useInfiniteQuery } from '@tanstack/react-query'
import { AlertCircle, LoaderCircle } from 'lucide-react'

import { listJobs } from '@/api/client'
import type { JobKind, JobStatus } from '@/api/types'
import { JobTable } from '@/components/job-table'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent } from '@/components/ui/card'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'

export function JobsPage() {
  const [status, setStatus] = useState<JobStatus | 'all'>('all')
  const [kind, setKind] = useState<JobKind | 'all'>('all')
  const query = useInfiniteQuery({
    queryKey: ['jobs', status, kind],
    queryFn: ({ pageParam }) => listJobs({ status, kind, cursor: pageParam }),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (page) => page.next_cursor ?? undefined,
  })
  const jobs = useMemo(() => query.data?.pages.flatMap((page) => page.items) ?? [], [query.data])

  return (
    <div className="space-y-8">
      <div className="flex flex-col justify-between gap-5 sm:flex-row sm:items-end">
        <div>
          <p className="text-sm font-medium text-primary">Actividad de la sesión</p>
          <h1 className="mt-2 text-3xl font-medium tracking-tight">Historial de trabajos</h1>
          <p className="mt-2 text-muted-foreground">
            Sólo se muestran los trabajos asociados a este navegador durante las últimas 24 horas.
          </p>
        </div>
        <div className="flex flex-col gap-3 sm:flex-row">
          <Select value={kind} onValueChange={(value) => setKind(value as JobKind | 'all')}>
            <SelectTrigger className="w-full sm:w-40" aria-label="Filtrar por tipo">
              <SelectValue placeholder="Tipo" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">Todos los tipos</SelectItem>
              <SelectItem value="image">Imágenes</SelectItem>
              <SelectItem value="pdf">PDFs</SelectItem>
            </SelectContent>
          </Select>
          <Select value={status} onValueChange={(value) => setStatus(value as JobStatus | 'all')}>
            <SelectTrigger className="w-full sm:w-52" aria-label="Filtrar por estado">
              <SelectValue placeholder="Estado" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">Todos los estados</SelectItem>
              <SelectItem value="queued">En espera</SelectItem>
              <SelectItem value="running">Procesando</SelectItem>
              <SelectItem value="retry_scheduled">Reintentando</SelectItem>
              <SelectItem value="cancel_requested">Cancelación solicitada</SelectItem>
              <SelectItem value="succeeded">Completados</SelectItem>
              <SelectItem value="dead_lettered">Fallidos</SelectItem>
              <SelectItem value="cancelled">Cancelados</SelectItem>
              <SelectItem value="expired">Expirados</SelectItem>
            </SelectContent>
          </Select>
        </div>
      </div>

      {query.isError ? (
        <Alert variant="destructive">
          <AlertCircle className="size-4" aria-hidden="true" />
          <AlertTitle>No pudimos cargar el historial</AlertTitle>
          <AlertDescription>{query.error.message}</AlertDescription>
        </Alert>
      ) : (
        <Card className="overflow-hidden">
          <CardContent className="p-0">
            {query.isPending ? (
              <div className="flex min-h-64 items-center justify-center">
                <LoaderCircle className="size-6 animate-spin text-muted-foreground" aria-label="Cargando trabajos" />
              </div>
            ) : (
              <JobTable jobs={jobs} />
            )}
          </CardContent>
        </Card>
      )}

      {query.hasNextPage ? (
        <div className="flex justify-center">
          <Button
            variant="outline"
            onClick={() => void query.fetchNextPage()}
            disabled={query.isFetchingNextPage}
          >
            {query.isFetchingNextPage ? 'Cargando…' : 'Cargar más'}
          </Button>
        </div>
      ) : null}
    </div>
  )
}
