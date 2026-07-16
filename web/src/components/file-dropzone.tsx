import { useRef, useState } from 'react'
import type { ChangeEvent, DragEvent } from 'react'
import { useMutation } from '@tanstack/react-query'
import { FileText, ImageIcon, LoaderCircle, UploadCloud, X } from 'lucide-react'
import { useNavigate } from 'react-router-dom'

import { ApiError, uploadJob } from '@/api/client'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { formatBytes } from '@/lib/format'
import { cn } from '@/lib/utils'

const MAX_FILE_BYTES = 10 * 1024 * 1024
const allowedTypes = new Set(['image/jpeg', 'image/png', 'image/webp', 'application/pdf'])

export function FileDropzone() {
  const navigate = useNavigate()
  const inputRef = useRef<HTMLInputElement>(null)
  const [file, setFile] = useState<File>()
  const [validationError, setValidationError] = useState<string>()
  const [dragging, setDragging] = useState(false)
  const mutation = useMutation({
    mutationFn: uploadJob,
    onSuccess: (job) => navigate(`/jobs/${job.id}`),
  })

  const selectFile = (nextFile: File | undefined): void => {
    mutation.reset()
    setValidationError(undefined)
    if (!nextFile) {
      setFile(undefined)
      return
    }
    if (!allowedTypes.has(nextFile.type)) {
      setFile(undefined)
      setValidationError('Usá una imagen JPEG, PNG o WebP, o un documento PDF.')
      return
    }
    if (nextFile.size > MAX_FILE_BYTES) {
      setFile(undefined)
      setValidationError('El archivo supera el máximo de 10 MiB.')
      return
    }
    setFile(nextFile)
  }

  const handleInput = (event: ChangeEvent<HTMLInputElement>): void => {
    selectFile(event.target.files?.[0])
  }

  const handleDrop = (event: DragEvent<HTMLDivElement>): void => {
    event.preventDefault()
    setDragging(false)
    selectFile(event.dataTransfer.files[0])
  }

  const error = validationError ?? (mutation.error instanceof ApiError ? mutation.error.message : undefined)

  return (
    <Card className="overflow-hidden border-primary/20 shadow-xl shadow-primary/5">
      <CardHeader>
        <CardTitle>Procesá un archivo</CardTitle>
        <CardDescription>El tipo se detecta por contenido, no por la extensión.</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div
          aria-label="Zona de carga"
          className={cn(
            'flex min-h-52 flex-col items-center justify-center rounded-xl border border-dashed p-6 text-center transition-colors',
            dragging ? 'border-primary bg-primary/5' : 'border-border bg-muted/30',
          )}
          onDragEnter={(event) => {
            event.preventDefault()
            setDragging(true)
          }}
          onDragOver={(event) => event.preventDefault()}
          onDragLeave={() => setDragging(false)}
          onDrop={handleDrop}
          role="region"
        >
          <span className="mb-4 flex size-12 items-center justify-center rounded-xl bg-primary/10 text-primary">
            <UploadCloud className="size-6" aria-hidden="true" />
          </span>
          <p className="font-medium">Arrastrá una imagen o PDF</p>
          <p className="mt-1 text-sm text-muted-foreground">JPEG, PNG, WebP o PDF · máximo 10 MiB</p>
          <Input
            ref={inputRef}
            type="file"
            accept="image/jpeg,image/png,image/webp,application/pdf"
            className="sr-only"
            onChange={handleInput}
            aria-label="Elegir archivo"
          />
          <Button
            type="button"
            variant="outline"
            className="mt-5"
            onClick={() => inputRef.current?.click()}
          >
            Elegir archivo
          </Button>
        </div>

        {file ? (
          <div className="flex items-center gap-3 rounded-lg border bg-card p-3">
            <span className="flex size-10 shrink-0 items-center justify-center rounded-lg bg-secondary">
              {file.type === 'application/pdf' ? (
                <FileText className="size-5" aria-hidden="true" />
              ) : (
                <ImageIcon className="size-5" aria-hidden="true" />
              )}
            </span>
            <div className="min-w-0 flex-1">
              <p className="truncate text-sm font-medium">{file.name}</p>
              <p className="text-xs text-muted-foreground">{formatBytes(file.size)}</p>
            </div>
            <Button
              type="button"
              variant="ghost"
              size="icon"
              aria-label="Quitar archivo"
              onClick={() => selectFile(undefined)}
              disabled={mutation.isPending}
            >
              <X className="size-4" aria-hidden="true" />
            </Button>
          </div>
        ) : null}

        {error ? (
          <Alert variant="destructive">
            <AlertTitle>No pudimos cargar el archivo</AlertTitle>
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        ) : null}

        <Button
          type="button"
          className="w-full"
          size="lg"
          disabled={!file || mutation.isPending}
          onClick={() => file && mutation.mutate(file)}
        >
          {mutation.isPending ? (
            <>
              <LoaderCircle className="size-4 animate-spin" aria-hidden="true" />
              Encolando trabajo…
            </>
          ) : (
            'Comenzar procesamiento'
          )}
        </Button>
      </CardContent>
    </Card>
  )
}
