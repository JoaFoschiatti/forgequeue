import { CheckCircle2, CircleDot, RotateCcw, XCircle } from 'lucide-react'

import type { JobAttempt } from '@/api/types'
import { formatDate } from '@/lib/format'

interface AttemptTimelineProps {
  attempts: JobAttempt[]
}

export function AttemptTimeline({ attempts }: AttemptTimelineProps) {
  if (attempts.length === 0) {
    return <p className="text-sm text-muted-foreground">El trabajo todavía no fue tomado por un worker.</p>
  }

  return (
    <ol className="space-y-4">
      {attempts.map((attempt) => (
        <li key={attempt.id} className="flex gap-3">
          <span className="mt-0.5 text-muted-foreground">
            <AttemptIcon status={attempt.status} />
          </span>
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <p className="text-sm font-medium">Intento {attempt.number}</p>
              <time className="text-xs text-muted-foreground" dateTime={attempt.started_at}>
                {formatDate(attempt.started_at)}
              </time>
            </div>
            <p className="mt-1 font-mono text-xs text-muted-foreground">worker {attempt.worker_id}</p>
            {attempt.error_detail ? (
              <p className="mt-2 text-sm text-destructive">{attempt.error_detail}</p>
            ) : null}
          </div>
        </li>
      ))}
    </ol>
  )
}

function AttemptIcon({ status }: { status: string }) {
  if (status === 'succeeded') return <CheckCircle2 className="size-4 text-primary" aria-hidden="true" />
  if (status === 'failed') return <XCircle className="size-4 text-destructive" aria-hidden="true" />
  if (status === 'cancelled') return <RotateCcw className="size-4" aria-hidden="true" />
  return <CircleDot className="size-4 animate-pulse text-primary" aria-hidden="true" />
}
