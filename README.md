# ForgeQueue

[![CI](https://github.com/JoaFoschiatti/forgequeue/actions/workflows/ci.yml/badge.svg)](https://github.com/JoaFoschiatti/forgequeue/actions/workflows/ci.yml)
[![Rust 1.97.1](https://img.shields.io/badge/Rust-1.97.1-CE422B?logo=rust)](rust-toolchain.toml)
[![License MIT](https://img.shields.io/badge/license-MIT-22c55e)](LICENSE)

Procesador distribuido de imágenes y PDFs construido para mostrar backend y cloud con Rust de una forma visual. Una persona sube un archivo, la API lo encola, un worker lo procesa y la interfaz refleja progreso, intentos y resultados en vivo.

![Portada de ForgeQueue](docs/forgequeue-home.png)

## Por qué existe

Procesar un PDF o redimensionar una imagen dentro de una petición HTTP vuelve frágil a cualquier aplicación: el cliente puede cortar la conexión, el proceso puede reiniciarse y un pico de trabajo bloquea la API. ForgeQueue separa ambas responsabilidades.

- La API persiste el archivo y responde con `202 Accepted`.
- PostgreSQL actúa como cola durable, sin infraestructura adicional.
- Los workers reclaman trabajos con un lease renovable.
- Si un worker desaparece, otro recupera el trabajo.
- Las escrituras deterministas hacen tolerable la entrega *at-least-once*.

## Inicio rápido

Requisito: Docker con Compose v2.

```bash
docker compose up --build
```

Después de que los health checks estén listos:

- Aplicación: [http://localhost:5173](http://localhost:5173)
- API y OpenAPI: [http://localhost:8080/docs](http://localhost:8080/docs)
- Métricas: [http://localhost:8080/metrics](http://localhost:8080/metrics)
- Métricas worker 1/2: [http://localhost:9101/metrics](http://localhost:9101/metrics) y [http://localhost:9102/metrics](http://localhost:9102/metrics)
- Consola de MinIO: [http://localhost:9001](http://localhost:9001) (`forgequeue` / `forgequeue-secret`)

El comando inicia React, la API Rust, dos procesos worker, PostgreSQL y MinIO. Los datos quedan en volúmenes de Docker.

## Arquitectura

```mermaid
flowchart LR
    Browser["React · sesión anónima"] -->|"multipart + bearer"| API["Axum API"]
    API -->|"metadatos y cola"| PG[("PostgreSQL")]
    API -->|"original privado"| S3[("MinIO / S3")]
    PG -->|"LISTEN / NOTIFY"| API
    API -->|"SSE autenticado"| Browser
    W1["Worker Rust 1"] -->|"SKIP LOCKED + heartbeat"| PG
    W2["Worker Rust 2"] -->|"SKIP LOCKED + heartbeat"| PG
    W1 <-->|"entrada / salidas"| S3
    W2 <-->|"entrada / salidas"| S3
```

La misma imagen de backend admite cuatro roles:

```bash
forgequeue api       # sólo HTTP y SSE
forgequeue worker    # un trabajo por proceso
forgequeue all       # API + worker + limpieza; útil para una demo gratuita
forgequeue cleanup   # una pasada de expiración
```

Más detalle en [Arquitectura](docs/ARCHITECTURE.md).

## Recorrido del producto

La interfaz está en español y no requiere registro.

1. **Inicio:** explica el sistema, valida y carga JPEG, PNG, WebP o PDF.
2. **Historial:** filtra los trabajos de la sesión por tipo y estado.
3. **Detalle:** consume SSE con `fetch`, muestra progreso, intentos, errores, previews y descargas privadas.

El navegador conserva un token opaco aleatorio. La API almacena sólo su SHA-256 y aplica aislamiento de sesión a cada consulta y descarga.

### Transformaciones

| Entrada | Salidas |
| --- | --- |
| JPEG, PNG o WebP | `metadata.json`, thumbnail WebP de 320 px y preview WebP de 1280 px |
| PDF de hasta 20 páginas | `metadata.json` y previews PNG de las primeras 3 páginas |

## Semántica de la cola

Los estados públicos son `queued`, `running`, `retry_scheduled`, `succeeded`, `cancel_requested`, `cancelled`, `dead_lettered` y `expired`.

```mermaid
stateDiagram-v2
    [*] --> queued
    queued --> running: lease
    queued --> cancelled: cancelar
    running --> succeeded
    running --> retry_scheduled: error o lease vencido
    running --> cancel_requested: cancelar
    cancel_requested --> cancelled
    retry_scheduled --> running: backoff cumplido
    retry_scheduled --> cancelled: cancelar
    running --> dead_lettered: intento 3
    succeeded --> expired: 1 hora
    cancelled --> expired: 1 hora
    dead_lettered --> expired: 1 hora
```

- Reserva transaccional con `FOR UPDATE SKIP LOCKED`.
- Lease de 60 s y heartbeat cada 15 s.
- Tres intentos totales; esperas de 5 s y 30 s.
- Salidas con claves deterministas y `UPSERT` por `(job_id, name)`.
- Recuperación de leases abandonados cada 10 s.
- Fencing por intento, worker y lease para rechazar finalizaciones obsoletas.
- `Idempotency-Key` único por sesión, incluso ante peticiones concurrentes.

## Demo de recuperación

Con Docker levantado, el siguiente script crea un worker lento controlado, espera a que tome un trabajo, lo mata y comprueba que otro worker lo termina:

```powershell
pwsh ./scripts/demo-recovery.ps1
```

La salida debe mostrar el mismo `job_id`, `attempt_count = 2` y una transición por `lease_recovered`. La guía manual está en [Demo de dos minutos](docs/DEMO.md).

## Pruebas

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo audit --ignore RUSTSEC-2023-0071

cd web
npm ci
npm run typecheck
npm run lint
npm test
npx playwright test
```

La suite PostgreSQL usa un esquema temporal y sólo se activa explícitamente:

```bash
FORGEQUEUE_DATABASE_TESTS=1 \
DATABASE_URL=postgres://forgequeue:forgequeue@localhost:5432/forgequeue \
cargo test -p forgequeue-server postgres_queue_contract
```

Esa prueba lanza tres reclamos concurrentes y verifica leases únicos, aislamiento de sesiones, idempotencia, cuotas que no se reinician al borrar, backoff de 5/30 segundos, dead letter, recuperación, expiración y `UPSERT` de artefactos. GitHub Actions la ejecuta contra PostgreSQL real.

## Observabilidad

Los logs son estructurados con `tracing` y cada respuesta expone `x-request-id`; el mismo valor aparece en errores Problem Details. Prometheus publica, entre otras:

- `forgequeue_jobs_created_total`
- `forgequeue_jobs_completed_total`
- `forgequeue_jobs_failed_total`
- `forgequeue_leases_recovered_total`
- `forgequeue_job_duration_seconds`

Para sumar Prometheus y Grafana:

```bash
docker compose --profile observability up --build
```

Prometheus queda en el puerto `9090` y Grafana en el `3000` (`admin` / `forgequeue`).

## Límites y seguridad

- 10 MiB por archivo, 25 megapíxeles por imagen y 20 páginas por PDF.
- Detección por contenido real; la extensión declarada no es confiable.
- Timeout de 90 s y un trabajo por proceso worker.
- Contenedor sin privilegios, filesystem de sólo lectura, `no-new-privileges`, límite de memoria, CPU y PIDs.
- Objetos siempre privados; la descarga pasa por autorización de sesión.
- Cinco cargas por sesión/hora, veinte por IP/día y cincuenta globales/día.
- Originales y resultados se borran a la hora; metadatos, a las 24 horas.
- Los encabezados de proxy se ignoran salvo que `TRUST_PROXY_HEADERS=true` en un entorno con proxy confiable.

Este es un proyecto de portfolio, no un servicio para datos sensibles. Ver [Política de seguridad](SECURITY.md).

## Contrato y tipos

Utoipa genera OpenAPI desde los handlers Rust. El frontend consume tipos generados, sin duplicar DTOs manualmente:

```bash
cd web
npm run generate:api
```

Los errores siguen Problem Details y añaden un `code` estable y `request_id`.

## Benchmark reproducible

El benchmark encola 100 imágenes pequeñas, espera a dos workers y reporta throughput y percentiles:

```powershell
pwsh ./scripts/benchmark.ps1 -Count 100
```

La referencia local verificada es **100/100 éxitos en 4,41 s (22,68 trabajos/s)**. La metodología, el reparto entre workers, el entorno y los percentiles están en [Benchmarks](docs/BENCHMARKS.md).

## Despliegue

La imagen `forgequeue` se usa con el rol `all` en un host pequeño; el frontend es estático. La receta prevista es Cloudflare Pages + Koyeb + Supabase PostgreSQL/S3. El repositorio incluye un workflow de Pages, un despliegue Koyeb repetible y `scripts/verify-deployment.sh` para probar la demo pública de extremo a extremo; las credenciales y comandos están en [Despliegue](docs/DEPLOYMENT.md).

Las migraciones se ejecutan al arrancar bajo un advisory lock de PostgreSQL, por lo que dos réplicas pueden iniciar sin competir.

## Tecnologías

Rust 2024, Tokio, Axum, SQLx, PostgreSQL, `object_store`, `image`, PDFium, Utoipa, `tracing`, React 19, TypeScript, Vite, TanStack Query, Tailwind CSS, shadcn/ui, Vitest y Playwright.

## Estructura

```text
crates/forgequeue-core/     dominio y máquina de estados
crates/forgequeue-server/   API, cola, worker y procesadores
migrations/                esquema SQLx
web/                       aplicación React
deploy/                    Prometheus y Grafana
scripts/                   demo de caos y benchmark
docs/                      arquitectura, operación y decisiones
```

## Licencia

[MIT](LICENSE).
