import { ArrowRight, Braces, Database, Gauge, RefreshCw, ShieldCheck, Workflow } from 'lucide-react'
import { Link } from 'react-router-dom'

import { BackendStatus } from '@/components/backend-status'
import { FileDropzone } from '@/components/file-dropzone'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Separator } from '@/components/ui/separator'

const steps = [
  {
    icon: Workflow,
    title: 'Encolá',
    description: 'La API guarda el archivo y responde antes de procesarlo.',
  },
  {
    icon: RefreshCw,
    title: 'Procesá',
    description: 'Un worker toma el lease y renueva su heartbeat mientras trabaja.',
  },
  {
    icon: ShieldCheck,
    title: 'Recuperá',
    description: 'Si el worker cae, otro recupera el trabajo sin perderlo.',
  },
] as const

export function HomePage() {
  return (
    <div className="space-y-20">
      <section className="grid items-center gap-12 lg:grid-cols-[1.05fr_0.95fr]">
        <div>
          <Badge variant="outline" className="mb-6 border-primary/30 bg-primary/5 text-primary">
            Rust · at-least-once delivery
          </Badge>
          <h1 className="max-w-3xl text-4xl font-medium tracking-tight text-balance sm:text-5xl lg:text-6xl">
            Trabajo pesado fuera de tu request principal.
          </h1>
          <p className="mt-6 max-w-2xl text-lg leading-8 text-muted-foreground">
            ForgeQueue procesa imágenes y PDFs en segundo plano. Podés seguir cada etapa,
            observar los reintentos y comprobar cómo el sistema sobrevive a la caída de un worker.
          </p>
          <div className="mt-8 flex flex-wrap gap-3">
            <Button size="lg" asChild>
              <a href="#procesar">
                Probar ahora
                <ArrowRight className="size-4" aria-hidden="true" />
              </a>
            </Button>
            <Button size="lg" variant="outline" asChild>
              <Link to="/jobs">Ver historial</Link>
            </Button>
          </div>
          <div className="mt-10 grid max-w-xl grid-cols-3 gap-4 border-t pt-6">
            <Metric value="60 s" label="lease" />
            <Metric value="3" label="intentos" />
            <Metric value="1 h" label="retención" />
          </div>
        </div>
        <div id="procesar" className="space-y-4 scroll-mt-24">
          <BackendStatus />
          <FileDropzone />
        </div>
      </section>

      <section aria-labelledby="how-it-works">
        <div className="max-w-2xl">
          <p className="text-sm font-medium text-primary">Un recorrido durable</p>
          <h2 id="how-it-works" className="mt-2 text-3xl font-medium tracking-tight">
            Qué ocurre después de subir un archivo
          </h2>
          <p className="mt-3 text-muted-foreground">
            El panel muestra una versión tangible de las garantías que normalmente quedan ocultas
            dentro de la infraestructura.
          </p>
        </div>
        <div className="mt-10 grid gap-8 md:grid-cols-3">
          {steps.map((step, index) => (
            <div key={step.title} className="border-t pt-6">
              <div className="flex items-center justify-between">
                <span className="flex size-10 items-center justify-center rounded-lg bg-primary/10 text-primary">
                  <step.icon className="size-5" aria-hidden="true" />
                </span>
                <span className="font-mono text-xs text-muted-foreground">0{index + 1}</span>
              </div>
              <h3 className="mt-5 font-medium">{step.title}</h3>
              <p className="mt-2 text-sm leading-6 text-muted-foreground">{step.description}</p>
            </div>
          ))}
        </div>
      </section>

      <Separator />

      <section className="grid gap-10 lg:grid-cols-[0.8fr_1.2fr]">
        <div>
          <p className="text-sm font-medium text-primary">Diseñado para explicarse</p>
          <h2 className="mt-2 text-3xl font-medium tracking-tight">Una demo, varias decisiones reales</h2>
        </div>
        <div className="grid gap-6 sm:grid-cols-2">
          <Capability icon={Database} title="PostgreSQL como cola">
            Leasing transaccional con <code className="font-mono text-foreground">SKIP LOCKED</code>.
          </Capability>
          <Capability icon={Gauge} title="Observabilidad">
            Logs estructurados, métricas Prometheus y progreso mediante SSE.
          </Capability>
          <Capability icon={Braces} title="Contrato compartido">
            OpenAPI generado en Rust y tipos TypeScript derivados automáticamente.
          </Capability>
          <Capability icon={ShieldCheck} title="Entrada controlada">
            MIME real, límites de píxeles y páginas, cuotas y expiración automática.
          </Capability>
        </div>
      </section>
    </div>
  )
}

function Metric({ value, label }: { value: string; label: string }) {
  return (
    <div>
      <p className="font-mono text-lg font-medium">{value}</p>
      <p className="text-xs text-muted-foreground">{label}</p>
    </div>
  )
}

interface CapabilityProps {
  icon: typeof Database
  title: string
  children: React.ReactNode
}

function Capability({ icon: Icon, title, children }: CapabilityProps) {
  return (
    <div className="flex gap-4">
      <Icon className="mt-0.5 size-5 shrink-0 text-primary" aria-hidden="true" />
      <div>
        <h3 className="font-medium">{title}</h3>
        <p className="mt-1 text-sm leading-6 text-muted-foreground">{children}</p>
      </div>
    </div>
  )
}
