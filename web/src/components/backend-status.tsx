import { useQuery } from '@tanstack/react-query'
import { CheckCircle2, LoaderCircle, ServerOff } from 'lucide-react'

import { health } from '@/api/client'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'

export function BackendStatus() {
  const query = useQuery({
    queryKey: ['backend-health'],
    queryFn: async ({ signal }) => health(signal),
    retry: 8,
    retryDelay: (attempt) => Math.min(1000 * 1.7 ** attempt, 8000),
    refetchInterval: 30_000,
  })

  if (query.isPending) {
    return (
      <Alert>
        <LoaderCircle className="size-4 animate-spin" aria-hidden="true" />
        <AlertTitle>Despertando el backend</AlertTitle>
        <AlertDescription>La instancia gratuita puede tardar unos segundos.</AlertDescription>
      </Alert>
    )
  }

  if (query.isError || !query.data) {
    return (
      <Alert variant="destructive">
        <ServerOff className="size-4" aria-hidden="true" />
        <AlertTitle>Backend no disponible</AlertTitle>
        <AlertDescription>Reintentaremos automáticamente. También podés recargar la página.</AlertDescription>
      </Alert>
    )
  }

  return (
    <Alert className="border-primary/25 bg-primary/5">
      <CheckCircle2 className="size-4 text-primary" aria-hidden="true" />
      <AlertTitle>Servicio listo</AlertTitle>
      <AlertDescription>La cola está aceptando nuevos trabajos.</AlertDescription>
    </Alert>
  )
}
