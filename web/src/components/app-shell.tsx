import type { PropsWithChildren } from 'react'
import { Code2, Layers3 } from 'lucide-react'
import { NavLink } from 'react-router-dom'

import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'

export function AppShell({ children }: PropsWithChildren) {
  return (
    <div className="min-h-svh">
      <header className="sticky top-0 z-40 border-b bg-background/95 backdrop-blur">
        <div className="mx-auto flex h-16 max-w-7xl items-center gap-6 px-4 sm:px-6 lg:px-8">
          <NavLink to="/" className="flex items-center gap-2 font-medium tracking-tight">
            <span className="flex size-8 items-center justify-center rounded-lg bg-primary text-primary-foreground">
              <Layers3 className="size-4" aria-hidden="true" />
            </span>
            <span>ForgeQueue</span>
          </NavLink>
          <nav className="flex items-center gap-1" aria-label="Navegación principal">
            <NavItem to="/">Inicio</NavItem>
            <NavItem to="/jobs">Historial</NavItem>
          </nav>
          <Button variant="ghost" size="sm" className="ms-auto" asChild>
            <a
              href="https://github.com/JoaFoschiatti/forgequeue"
              target="_blank"
              rel="noreferrer"
            >
              <Code2 className="size-4" aria-hidden="true" />
              <span className="hidden sm:inline">Código</span>
              <span className="sr-only sm:hidden">Ver código en GitHub</span>
            </a>
          </Button>
        </div>
      </header>
      <main className="mx-auto max-w-7xl px-4 py-8 sm:px-6 sm:py-12 lg:px-8">{children}</main>
      <footer className="border-t">
        <div className="mx-auto flex max-w-7xl flex-col gap-2 px-4 py-8 text-sm text-muted-foreground sm:flex-row sm:items-center sm:justify-between sm:px-6 lg:px-8">
          <p>Procesamiento durable construido con Rust, PostgreSQL y React.</p>
          <p>Los archivos se eliminan automáticamente después de una hora.</p>
        </div>
      </footer>
    </div>
  )
}

interface NavItemProps extends PropsWithChildren {
  to: string
}

function NavItem({ to, children }: NavItemProps) {
  return (
    <NavLink
      to={to}
      className={({ isActive }) =>
        cn(
          'rounded-md px-3 py-2 text-sm text-muted-foreground transition-colors hover:text-foreground',
          isActive && 'bg-accent text-accent-foreground',
        )
      }
    >
      {children}
    </NavLink>
  )
}
