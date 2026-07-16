# Arquitectura de ForgeQueue

## Objetivos

ForgeQueue busca que tres propiedades sean observables: una petición HTTP no espera al procesamiento, dos workers no ejecutan el mismo lease al mismo tiempo y la caída de un worker no pierde el trabajo.

La solución es una aplicación interna modular. `forgequeue-core` contiene el lenguaje del dominio y `forgequeue-server` compone API, persistencia, almacenamiento, procesadores y worker. Separar procesos es una decisión de despliegue, no una duplicación de código.

## Componentes

| Componente | Responsabilidad |
| --- | --- |
| API Axum | sesiones, carga, consulta, acciones, descargas, SSE y health checks |
| PostgreSQL | fuente de verdad, cola, leases, intentos, idempotencia y eventos |
| Object store | originales y artefactos privados en memoria, filesystem o S3 |
| Worker | reclama un trabajo, renueva el lease, procesa y confirma el resultado |
| Cleanup | elimina objetos a la hora y metadatos a las 24 horas |
| React | representa el estado del dominio; no inventa estados locales de negocio |

## Reclamo de un trabajo

El worker abre una transacción y ejecuta conceptualmente:

```sql
WITH candidate AS (
  SELECT id
  FROM jobs
  WHERE status IN ('queued', 'retry_scheduled')
    AND available_at <= now()
  ORDER BY available_at, created_at
  FOR UPDATE SKIP LOCKED
  LIMIT 1
)
UPDATE jobs
SET status = 'running',
    lease_until = now() + interval '60 seconds',
    attempt_count = attempt_count + 1
FROM candidate
WHERE jobs.id = candidate.id
RETURNING jobs.*;
```

En la misma transacción inserta `job_attempts`. `SKIP LOCKED` permite que varios procesos consulten en paralelo sin bloquearse entre sí y sin entregar el mismo lease activo.

## Por qué es *at-least-once*

Existe una ventana inevitable: un worker puede guardar un resultado y caer antes de confirmar `succeeded`. El lease vence y otro worker repite el trabajo. Por eso ForgeQueue no promete *exactly-once*; hace que repetir sea seguro:

- La ruta del objeto depende de sesión, trabajo y nombre de artefacto.
- `job_outputs` tiene `UNIQUE(job_id, name)`.
- La persistencia usa `INSERT ... ON CONFLICT DO UPDATE`.
- Transformar la misma entrada produce el mismo conjunto de nombres.

La garantía útil no es “nunca se ejecuta dos veces”, sino “un reintento no duplica el resultado visible”.

## Heartbeats y recuperación

Cada worker renueva su lease cada 15 segundos. Otro ciclo inspecciona cada 10 segundos los trabajos `running` cuyo `lease_until` ya pasó:

1. Cierra el intento activo con `worker_lost`.
2. Mueve el trabajo a `retry_scheduled` respetando el backoff de 5 s o 30 s.
3. Publica `pg_notify('forgequeue_job_events', job_id)`.
4. Un worker libre lo reclama como un intento nuevo.

Al tercer intento, el trabajo pasa a `dead_lettered`.

Cada finalización está protegida por el `attempt_id`, el `worker_id` y un lease todavía vigente. Si el worker antiguo reaparece después de la recuperación, su resultado queda cercado y no puede sobrescribir el estado del intento nuevo.

## Actualizaciones en vivo

Cada cambio relevante emite `NOTIFY`. La API mantiene un `PgListener`, distribuye IDs mediante un canal broadcast interno y vuelve a consultar el detalle antes de enviarlo por SSE. El endpoint SSE usa `fetch`, no `EventSource`, porque necesita un bearer token.

El polling de 15 segundos del frontend es un respaldo deliberado: si el proxy corta SSE o se pierde un `NOTIFY`, la vista converge igualmente al estado persistido.

## Cancelación

- `queued` o `retry_scheduled`: se cancela de inmediato.
- `running`: pasa a `cancel_requested`.
- El procesador consulta cancelación antes y después de la fase costosa.
- Si el proceso cae con una cancelación solicitada, la recuperación finaliza como `cancelled`.

La cancelación es cooperativa; no interrumpe memoria nativa de PDFium en mitad de una llamada.

Un timeout sí marca el intento como fallido y termina el proceso worker. Tokio no puede cancelar de forma segura una tarea nativa iniciada con `spawn_blocking`; el reinicio del contenedor garantiza que esa llamada no conviva con el trabajo siguiente. En el rol `all`, el supervisor detiene también la API para que la plataforma reinicie la instancia completa.

## Sesiones e idempotencia

El token de sesión tiene 256 bits aleatorios. Sólo se almacena SHA-256. Todas las consultas a trabajos y outputs incluyen `session_id`.

`Idempotency-Key` tiene alcance de sesión y un índice único parcial. La API consulta primero por comodidad y, si dos peticiones compiten, captura el conflicto de unicidad y devuelve el trabajo creado por la ganadora.

## Aislamiento de PDFium

Cada proceso worker ejecuta un trabajo a la vez. En Compose los workers son contenedores separados, sin privilegios, con filesystem de sólo lectura y límites de recursos. Escalar significa agregar procesos, no aumentar concurrencia dentro del proceso que carga PDFium.

## Decisiones y límites conscientes

- PostgreSQL evita incorporar Redis/RabbitMQ en un portfolio y mantiene transacciones entre cola e intentos.
- `LISTEN/NOTIFY` es señal, no almacenamiento; PostgreSQL sigue siendo la fuente de verdad.
- La demo gratuita usa el rol `all`; producción real separaría API y workers.
- No hay OCR, workflows arbitrarios, cron ni ejecución de código.
- Las cuotas son protección de demo, no un sistema de billing distribuido.
