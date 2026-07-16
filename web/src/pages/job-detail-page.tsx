import { useQuery } from '@tanstack/react-query'
import { ArrowLeft, Clock3, FileImage, FileText, Hash, LoaderCircle } from 'lucide-react'
import { Link, useParams } from 'react-router-dom'

import { getJob } from '@/api/client'
import { ArtifactPreview } from '@/components/artifact-preview'
import { AttemptTimeline } from '@/components/attempt-timeline'
import { JobActions } from '@/components/job-actions'
import { StatusBadge } from '@/components/status-badge'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Progress } from '@/components/ui/progress'
import { Separator } from '@/components/ui/separator'
import { useJobEvents } from '@/hooks/use-job-events'
import { formatBytes, formatDate, kindLabels, shortId } from '@/lib/format'

const terminalStatuses = new Set(['succeeded', 'cancelled', 'dead_lettered', 'expired'])

export function JobDetailPage() {
  const { jobId } = useParams()
  const query = useQuery({
    queryKey: ['job', jobId],
    queryFn: () => getJob(jobId ?? ''),
    enabled: Boolean(jobId),
    refetchInterval: (result) =>
      result.state.data && terminalStatuses.has(result.state.data.status) ? false : 15_000,
  })
  const active = Boolean(query.data && !terminalStatuses.has(query.data.status))
  useJobEvents(jobId, active)

  if (query.isPending) {
    return (
      <div className="flex min-h-[50svh] items-center justify-center">
        <LoaderCircle className="size-7 animate-spin text-muted-foreground" aria-label="Cargando trabajo" />
      </div>
    )
  }

  if (query.isError || !query.data) {
    return (
      <div className="mx-auto max-w-2xl space-y-6">
        <Alert variant="destructive">
          <AlertTitle>No encontramos el trabajo</AlertTitle>
          <AlertDescription>
            {query.error?.message ?? 'Puede haber expirado o pertenecer a otra sesión.'}
          </AlertDescription>
        </Alert>
        <Button variant="outline" asChild><Link to="/jobs">Volver al historial</Link></Button>
      </div>
    )
  }

  const job = query.data

  return (
    <div className="space-y-8">
      <Button variant="ghost" size="sm" asChild className="-ms-3">
        <Link to="/jobs"><ArrowLeft className="size-4" aria-hidden="true" />Historial</Link>
      </Button>

      <div className="flex flex-col justify-between gap-6 lg:flex-row lg:items-start">
        <div className="min-w-0">
          <div className="mb-3 flex flex-wrap items-center gap-3">
            <StatusBadge status={job.status} />
            <span className="font-mono text-xs text-muted-foreground">#{shortId(job.id)}</span>
          </div>
          <h1 className="truncate text-3xl font-medium tracking-tight sm:text-4xl">
            {job.original_name}
          </h1>
          <p className="mt-3 text-muted-foreground">
            {kindLabels[job.kind]} · {formatBytes(job.input_size)} · creado {formatDate(job.created_at)}
          </p>
        </div>
        <JobActions job={job} />
      </div>

      <Card className="border-primary/20">
        <CardHeader className="pb-3">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div>
              <CardTitle className="text-base">{stageLabel(job.stage)}</CardTitle>
              <CardDescription>
                Intento {job.attempt_count} de {job.max_attempts}
              </CardDescription>
            </div>
            <span className="font-mono text-sm text-muted-foreground">{job.progress}%</span>
          </div>
        </CardHeader>
        <CardContent>
          <Progress value={job.progress} aria-label={`Progreso ${job.progress}%`} />
          {job.last_error_detail ? (
            <Alert variant="destructive" className="mt-5">
              <AlertTitle>{job.last_error_code ?? 'Error de procesamiento'}</AlertTitle>
              <AlertDescription>{job.last_error_detail}</AlertDescription>
            </Alert>
          ) : null}
        </CardContent>
      </Card>

      <div className="grid gap-6 lg:grid-cols-[1fr_22rem]">
        <section aria-labelledby="results-title">
          <div className="mb-5 flex items-end justify-between gap-4">
            <div>
              <h2 id="results-title" className="text-xl font-medium">Resultados</h2>
              <p className="mt-1 text-sm text-muted-foreground">
                Los artefactos se eliminan una hora después de finalizar.
              </p>
            </div>
          </div>
          {job.outputs.length > 0 ? (
            <div className="grid gap-4 sm:grid-cols-2">
              {job.outputs.map((output) => (
                <ArtifactPreview key={output.id} jobId={job.id} output={output} />
              ))}
            </div>
          ) : (
            <Card>
              <CardContent className="flex min-h-48 flex-col items-center justify-center text-center">
                {job.kind === 'pdf' ? (
                  <FileText className="size-7 text-muted-foreground" aria-hidden="true" />
                ) : (
                  <FileImage className="size-7 text-muted-foreground" aria-hidden="true" />
                )}
                <p className="mt-4 font-medium">Todavía no hay resultados</p>
                <p className="mt-1 text-sm text-muted-foreground">
                  Aparecerán acá cuando el worker complete cada salida.
                </p>
              </CardContent>
            </Card>
          )}
        </section>

        <aside className="space-y-6">
          <Card>
            <CardHeader><CardTitle className="text-base">Intentos</CardTitle></CardHeader>
            <CardContent><AttemptTimeline attempts={job.attempts} /></CardContent>
          </Card>
          <Card>
            <CardHeader><CardTitle className="text-base">Detalles</CardTitle></CardHeader>
            <CardContent className="space-y-4 text-sm">
              <Detail icon={Hash} label="Identificador" value={job.id} mono />
              <Separator />
              <Detail icon={Clock3} label="Última actualización" value={formatDate(job.updated_at)} />
              {job.artifacts_expire_at ? (
                <>
                  <Separator />
                  <Detail icon={Clock3} label="Resultados hasta" value={formatDate(job.artifacts_expire_at)} />
                </>
              ) : null}
            </CardContent>
          </Card>
        </aside>
      </div>
    </div>
  )
}

interface DetailProps {
  icon: typeof Hash
  label: string
  value: string
  mono?: boolean
}

function Detail({ icon: Icon, label, value, mono }: DetailProps) {
  return (
    <div className="flex gap-3">
      <Icon className="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
      <div className="min-w-0">
        <p className="text-xs text-muted-foreground">{label}</p>
        <p className={mono ? 'truncate font-mono text-xs' : 'mt-0.5'}>{value}</p>
      </div>
    </div>
  )
}

function stageLabel(stage: string): string {
  const labels: Record<string, string> = {
    queued: 'Esperando un worker',
    claimed: 'Worker asignado',
    loading_input: 'Leyendo archivo original',
    demo_delay: 'Pausa controlada para demostrar recuperación',
    decoding_image: 'Decodificando imagen',
    image_encoded: 'Imagen transformada',
    rendering_pdf: 'Renderizando páginas',
    pdf_rendered: 'Previews generados',
    storing_outputs: 'Guardando resultados',
    retry_wait: 'Esperando próximo intento',
    lease_recovered: 'Recuperado después de una caída',
    cancel_requested: 'Cancelación solicitada',
    cancelled: 'Trabajo cancelado',
    completed: 'Procesamiento completado',
    dead_lettered: 'Sin más reintentos',
    expired: 'Resultados expirados',
  }
  return labels[stage] ?? stage.replaceAll('_', ' ')
}
