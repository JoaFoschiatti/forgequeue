import type { JobKind, JobStatus } from '@/api/types'

export const statusLabels: Record<JobStatus, string> = {
  queued: 'En espera',
  running: 'Procesando',
  retry_scheduled: 'Reintentando',
  succeeded: 'Completado',
  cancel_requested: 'Cancelando',
  cancelled: 'Cancelado',
  dead_lettered: 'Fallido definitivamente',
  expired: 'Expirado',
}

export const kindLabels: Record<JobKind, string> = {
  image: 'Imagen',
  pdf: 'PDF',
}

export function formatBytes(value: number): string {
  if (value < 1024) return `${value} B`
  if (value < 1024 ** 2) return `${(value / 1024).toFixed(1)} KiB`
  return `${(value / 1024 ** 2).toFixed(1)} MiB`
}

export function formatDate(value: string): string {
  return new Intl.DateTimeFormat('es-AR', {
    dateStyle: 'medium',
    timeStyle: 'short',
  }).format(new Date(value))
}

export function shortId(value: string): string {
  return value.slice(0, 8)
}
