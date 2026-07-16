# Demo de dos minutos

## Recorrido de producto

1. Abrir `http://localhost:5173` y resumir el problema: “el procesamiento pesado no debería vivir dentro del request”.
2. Subir una imagen. La API devuelve el trabajo y navega al detalle.
3. Señalar progreso SSE, intento actual y salidas WebP.
4. Descargar `metadata.json` o una preview.
5. Abrir Historial y mostrar filtros y aislamiento por sesión anónima.

## Recuperación automática

El script requiere PowerShell 7 y Docker Compose:

```powershell
pwsh ./scripts/demo-recovery.ps1
```

Internamente:

1. Levanta API, PostgreSQL, MinIO y frontend.
2. Detiene los workers normales.
3. Inicia un worker con una pausa controlada y lease de 10 segundos.
4. Carga una imagen y espera `running`.
5. Mata el contenedor sin cierre coordinado.
6. Inicia un segundo worker.
7. Espera `succeeded` y valida dos intentos.

La pausa sólo se activa con `DEMO_PROCESSING_DELAY_MILLISECONDS`; su valor por defecto es cero.

## Demostración manual

También se puede observar desde la interfaz:

```bash
docker compose stop worker-2
docker compose stop worker-1
docker compose run -d --name forgequeue-chaos \
  -e WORKER_ID=chaos-worker \
  -e LEASE_SECONDS=10 \
  -e HEARTBEAT_SECONDS=2 \
  -e DEMO_PROCESSING_DELAY_MILLISECONDS=45000 \
  worker-1 worker
```

Subir una imagen y, cuando aparezca “Pausa controlada para demostrar recuperación”:

```bash
docker stop forgequeue-chaos
docker compose start worker-2
```

Entre 15 y 30 segundos después, el detalle muestra un intento fallido con `worker_lost`, el estado `lease_recovered`, el backoff de 5 segundos y un segundo intento exitoso.

## Evidencia verificada

La prueba local release del 16 de julio de 2026 terminó con el mismo `job_id`, estado `succeeded`, dos intentos y tres artefactos. PostgreSQL conservó:

```text
1 | chaos-worker    | failed    | worker_lost
2 | recovery-worker | succeeded |
```

La suite de integración repite el contrato en un esquema PostgreSQL temporal dentro de CI.
