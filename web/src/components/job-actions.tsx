import { useMutation, useQueryClient } from '@tanstack/react-query'
import { RotateCcw, Trash2, XCircle } from 'lucide-react'
import { useNavigate } from 'react-router-dom'
import { toast } from 'sonner'

import { cancelJob, deleteJob, retryJob } from '@/api/client'
import type { JobDetail } from '@/api/types'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'

interface JobActionsProps {
  job: JobDetail
}

export function JobActions({ job }: JobActionsProps) {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const refresh = (): void => {
    void queryClient.invalidateQueries({ queryKey: ['job', job.id] })
    void queryClient.invalidateQueries({ queryKey: ['jobs'] })
  }
  const cancelMutation = useMutation({
    mutationFn: () => cancelJob(job.id),
    onSuccess: () => {
      refresh()
      toast.success('Cancelación solicitada')
    },
    onError: (error) => toast.error(error.message),
  })
  const retryMutation = useMutation({
    mutationFn: () => retryJob(job.id),
    onSuccess: (nextJob) => {
      refresh()
      toast.success('Nuevo trabajo encolado')
      navigate(`/jobs/${nextJob.id}`)
    },
    onError: (error) => toast.error(error.message),
  })
  const deleteMutation = useMutation({
    mutationFn: () => deleteJob(job.id),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['jobs'] })
      toast.success('Trabajo eliminado')
      navigate('/jobs')
    },
    onError: (error) => toast.error(error.message),
  })

  const canCancel = ['queued', 'running', 'retry_scheduled'].includes(job.status)
  const canDelete = ['succeeded', 'cancelled', 'dead_lettered', 'expired'].includes(job.status)

  return (
    <div className="flex flex-wrap gap-2">
      {canCancel ? (
        <Button
          variant="outline"
          onClick={() => cancelMutation.mutate()}
          disabled={cancelMutation.isPending}
        >
          <XCircle className="size-4" aria-hidden="true" />
          Cancelar
        </Button>
      ) : null}
      {job.status === 'dead_lettered' ? (
        <Button
          variant="outline"
          onClick={() => retryMutation.mutate()}
          disabled={retryMutation.isPending}
        >
          <RotateCcw className="size-4" aria-hidden="true" />
          Volver a intentar
        </Button>
      ) : null}
      {canDelete ? (
        <AlertDialog>
          <AlertDialogTrigger asChild>
            <Button variant="ghost" disabled={deleteMutation.isPending}>
              <Trash2 className="size-4" aria-hidden="true" />
              Eliminar
            </Button>
          </AlertDialogTrigger>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>¿Eliminar este trabajo?</AlertDialogTitle>
              <AlertDialogDescription>
                Se borrarán sus metadatos y cualquier resultado que todavía no haya expirado.
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>Conservar</AlertDialogCancel>
              <AlertDialogAction
                variant="destructive"
                onClick={() => deleteMutation.mutate()}
              >
                Eliminar
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      ) : null}
    </div>
  )
}
