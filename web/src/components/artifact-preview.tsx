import { useEffect, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { AlertCircle, Code2, Download, LoaderCircle } from 'lucide-react'

import { downloadOutput } from '@/api/client'
import type { JobOutput } from '@/api/types'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardFooter, CardHeader, CardTitle } from '@/components/ui/card'
import { formatBytes } from '@/lib/format'

interface ArtifactPreviewProps {
  jobId: string
  output: JobOutput
}

export function ArtifactPreview({ jobId, output }: ArtifactPreviewProps) {
  const query = useQuery({
    queryKey: ['output', jobId, output.id],
    queryFn: ({ signal }) => downloadOutput(jobId, output.id, signal),
    staleTime: Number.POSITIVE_INFINITY,
  })
  const [objectUrl, setObjectUrl] = useState<string>()

  useEffect(() => {
    if (!query.data) return
    const url = URL.createObjectURL(query.data)
    setObjectUrl(url)
    return () => URL.revokeObjectURL(url)
  }, [query.data])

  const isImage = output.content_type.startsWith('image/')

  return (
    <Card className="overflow-hidden">
      <CardContent className="flex aspect-video items-center justify-center bg-muted/40 p-0">
        {query.isPending ? (
          <LoaderCircle className="size-6 animate-spin text-muted-foreground" aria-label="Cargando resultado" />
        ) : query.isError ? (
          <div className="flex flex-col items-center gap-2 px-4 text-center text-sm text-destructive">
            <AlertCircle className="size-6" aria-hidden="true" />
            No se pudo cargar este resultado
          </div>
        ) : objectUrl && isImage ? (
          <img
            src={objectUrl}
            alt={`Resultado ${output.name}`}
            className="size-full object-contain"
          />
        ) : (
          <Code2 className="size-8 text-muted-foreground" aria-hidden="true" />
        )}
      </CardContent>
      <CardHeader className="pb-2">
        <CardTitle className="truncate font-mono text-sm">{output.name}</CardTitle>
      </CardHeader>
      <CardFooter className="flex items-center justify-between gap-3 text-xs text-muted-foreground">
        <span>
          {output.width && output.height ? `${output.width}×${output.height} · ` : ''}
          {formatBytes(output.size)}
        </span>
        {objectUrl ? (
          <Button variant="ghost" size="sm" asChild>
            <a href={objectUrl} download={output.name}>
              <Download className="size-4" aria-hidden="true" />
              Descargar
            </a>
          </Button>
        ) : (
          <Button variant="ghost" size="sm" disabled>
            <Download className="size-4" aria-hidden="true" />
            Descargar
          </Button>
        )}
      </CardFooter>
    </Card>
  )
}
