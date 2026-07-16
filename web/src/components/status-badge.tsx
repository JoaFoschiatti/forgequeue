import type { JobStatus } from '@/api/types'
import { Badge } from '@/components/ui/badge'
import { cn } from '@/lib/utils'
import { statusLabels } from '@/lib/format'

interface StatusBadgeProps {
  status: JobStatus
}

const statusClasses: Record<JobStatus, string> = {
  queued: 'border-border bg-secondary text-secondary-foreground',
  running: 'border-primary/30 bg-primary/10 text-primary',
  retry_scheduled: 'border-chart-3/30 bg-chart-3/10 text-foreground',
  succeeded: 'border-primary/30 bg-primary/10 text-primary',
  cancel_requested: 'border-chart-3/30 bg-chart-3/10 text-foreground',
  cancelled: 'border-border bg-muted text-muted-foreground',
  dead_lettered: 'border-destructive/30 bg-destructive/10 text-destructive',
  expired: 'border-border bg-muted text-muted-foreground',
}

export function StatusBadge({ status }: StatusBadgeProps) {
  return (
    <Badge variant="outline" className={cn('whitespace-nowrap font-normal', statusClasses[status])}>
      {statusLabels[status]}
    </Badge>
  )
}
