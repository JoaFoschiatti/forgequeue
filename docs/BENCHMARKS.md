# Benchmarks

## Escenario

- 100 imágenes PNG pequeñas de 1×1 px (fixture sintética).
- Una API y dos procesos worker.
- Binario Rust `--release`, PostgreSQL 16.14 en WSL2 y object storage de filesystem compartido.
- Medición desde la primera carga aceptada hasta el último estado terminal.
- Polling de resultados paginado para minimizar el efecto del observador.

Ejecutar:

```powershell
$env:SESSION_HOURLY_LIMIT = 200
$env:IP_DAILY_LIMIT = 200
$env:GLOBAL_DAILY_LIMIT = 200
docker compose up --build -d
pwsh ./scripts/benchmark.ps1 -Count 100
```

## Resultado local de referencia

Ejecución del 16 de julio de 2026:

| Métrica | Resultado |
| --- | ---: |
| Trabajos exitosos | 100 / 100 |
| Intentos totales | 100 |
| Reparto worker 1 / worker 2 | 65 / 35 |
| Tiempo extremo a extremo | 4,41 s |
| Throughput observado | 22,68 trabajos/s |
| Aceptación HTTP p50 | 35,70 ms |
| Aceptación HTTP p95 | 41,44 ms |
| Procesamiento worker p50 | 44,95 ms |
| Procesamiento worker p95 | 53,83 ms |

Equipo: AMD Ryzen 7 5800X (8 núcleos/16 hilos), 31,9 GiB RAM, Windows 11 Pro, Rust 1.97.1. Los límites de cuota se elevaron a 200 sólo durante la prueba.

El resultado incluye carga HTTP secuencial, persistencia de cola, leasing, lectura/escritura del object store, dos transformaciones WebP, metadata JSON y consultas de estado. No incluye red remota ni S3 administrado.

## Qué reveló el benchmark

La primera versión dormía 750 ms después de **cada** trabajo. El benchmark inicial produjo aproximadamente 1 trabajo/s. El loop se corrigió para drenar inmediatamente cuando existe backlog y dormir sólo con la cola vacía; la muestra release actual alcanzó 22,68 trabajos/s.

El script reporta:

- tiempo total y trabajos por segundo;
- latencia p50/p95 de aceptación HTTP;
- éxitos, fallos y cantidad total de intentos;
- snapshot de métricas Prometheus.

No se comparan máquinas diferentes: el objetivo es detectar regresiones dentro del proyecto, no publicar una cifra universal.
