import { SearchX } from 'lucide-react'
import { Link } from 'react-router-dom'

import { Button } from '@/components/ui/button'

export function NotFoundPage() {
  return (
    <div className="flex min-h-[55svh] flex-col items-center justify-center text-center">
      <SearchX className="size-10 text-muted-foreground" aria-hidden="true" />
      <p className="mt-6 font-mono text-sm text-primary">404</p>
      <h1 className="mt-2 text-3xl font-medium tracking-tight">Esta ruta no existe</h1>
      <p className="mt-3 max-w-md text-muted-foreground">
        Volvé al inicio para procesar un archivo o revisá el historial de tu sesión.
      </p>
      <Button className="mt-6" asChild><Link to="/">Ir al inicio</Link></Button>
    </div>
  )
}
