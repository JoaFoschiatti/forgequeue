import { ArrowRight, FileImage, FileText, Inbox } from 'lucide-react'
import { Link } from 'react-router-dom'

import type { Job } from '@/api/types'
import { StatusBadge } from '@/components/status-badge'
import { Button } from '@/components/ui/button'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { formatBytes, formatDate, kindLabels, shortId } from '@/lib/format'

interface JobTableProps {
  jobs: Job[]
}

export function JobTable({ jobs }: JobTableProps) {
  if (jobs.length === 0) {
    return (
      <div className="flex min-h-64 flex-col items-center justify-center px-6 text-center">
        <span className="mb-4 flex size-12 items-center justify-center rounded-xl bg-muted text-muted-foreground">
          <Inbox className="size-6" aria-hidden="true" />
        </span>
        <p className="font-medium">Todavía no hay trabajos</p>
        <p className="mt-1 max-w-sm text-sm text-muted-foreground">
          Subí un archivo o cambiá los filtros para ver la actividad de esta sesión.
        </p>
      </div>
    )
  }

  return (
    <Table>
      <TableHeader>
        <TableRow>
          <TableHead>Archivo</TableHead>
          <TableHead>Estado</TableHead>
          <TableHead className="hidden md:table-cell">Tamaño</TableHead>
          <TableHead className="hidden lg:table-cell">Creado</TableHead>
          <TableHead className="w-12"><span className="sr-only">Abrir</span></TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {jobs.map((job) => (
          <TableRow key={job.id}>
            <TableCell>
              <div className="flex items-center gap-3">
                <span className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-secondary text-secondary-foreground">
                  {job.kind === 'pdf' ? (
                    <FileText className="size-4" aria-hidden="true" />
                  ) : (
                    <FileImage className="size-4" aria-hidden="true" />
                  )}
                </span>
                <div className="min-w-0">
                  <Link to={`/jobs/${job.id}`} className="block truncate font-medium hover:underline">
                    {job.original_name}
                  </Link>
                  <p className="font-mono text-xs text-muted-foreground">
                    {kindLabels[job.kind]} · {shortId(job.id)}
                  </p>
                </div>
              </div>
            </TableCell>
            <TableCell><StatusBadge status={job.status} /></TableCell>
            <TableCell className="hidden text-muted-foreground md:table-cell">
              {formatBytes(job.input_size)}
            </TableCell>
            <TableCell className="hidden text-muted-foreground lg:table-cell">
              {formatDate(job.created_at)}
            </TableCell>
            <TableCell>
              <Button variant="ghost" size="icon" asChild>
                <Link to={`/jobs/${job.id}`} aria-label={`Abrir ${job.original_name}`}>
                  <ArrowRight className="size-4" aria-hidden="true" />
                </Link>
              </Button>
            </TableCell>
          </TableRow>
        ))}
      </TableBody>
    </Table>
  )
}
