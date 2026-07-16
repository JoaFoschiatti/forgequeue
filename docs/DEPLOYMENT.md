# Despliegue gratuito de referencia

La topología de portfolio usa frontend estático, un backend combinado y servicios administrados. Los planes gratuitos cambian; confirmar límites antes de publicar. Los comandos de esta guía no imprimen secretos, pero crean recursos externos dentro de las cuentas conectadas.

## 1. Supabase

Crear un proyecto y:

- Copiar la connection string PostgreSQL con `sslmode=require`. Usar la conexión directa si el host tiene IPv6; en una red sólo IPv4, elegir **Session pooler** (puerto 5432). No usar transaction mode porque deshabilita prepared statements. Ver [Conexiones de Supabase](https://supabase.com/docs/guides/database/connecting-to-postgres).
- Crear un bucket **privado** llamado `forgequeue`.
- Habilitar el protocolo S3 desde Storage → Configuration → S3.
- Generar access key y secret key de uso exclusivo del backend.

La aplicación crea el esquema al arrancar. Un advisory lock serializa las migraciones si dos instancias comienzan juntas. El arranque también lista el bucket una vez para detectar endpoint, credenciales o bucket incorrectos antes de aceptar tráfico.

Variables de backend:

```dotenv
DATABASE_URL=postgresql://...
OBJECT_STORE_URL=s3://forgequeue
S3_ENDPOINT=https://<project>.storage.supabase.co/storage/v1/s3
S3_REGION=<region>
S3_ACCESS_KEY_ID=...
S3_SECRET_ACCESS_KEY=...
```

## 2. Koyeb

La creación puede hacerse desde el panel o con el script versionado. Para la vía automatizada, instalar Koyeb CLI, ejecutar `koyeb login` y exportar las variables obtenidas de Supabase:

```bash
export DATABASE_URL='postgresql://...?...sslmode=require'
export S3_ENDPOINT='https://<project>.storage.supabase.co/storage/v1/s3'
export S3_REGION='<region>'
export S3_ACCESS_KEY_ID='...'
export S3_SECRET_ACCESS_KEY='...'
export CORS_ORIGIN='https://forgequeue-web.pages.dev'
export RATE_LIMIT_SALT='<32-o-más-bytes-aleatorios>'
./scripts/deploy-koyeb.sh
```

El script guarda las cuatro credenciales sensibles como secretos de Koyeb mediante stdin, crea un Web Service gratuito desde el `Dockerfile`, selecciona el rol `all`, configura puerto y health check, y espera el despliegue. No admite modo privilegiado.

Configuración equivalente en el panel:

- Comando: `all`
- Puerto: `8080`
- Health check: `/health/ready`
- Instancia: la opción gratuita disponible.

Variables adicionales:

```dotenv
BIND_ADDRESS=0.0.0.0:8080
CORS_ORIGIN=https://<frontend>.pages.dev
TRUST_PROXY_HEADERS=true
RATE_LIMIT_SALT=<32-o-más-bytes-aleatorios>
LOG_FORMAT=json
RUST_LOG=forgequeue_server=info,tower_http=info
```

El rol `all` ejecuta API, un worker y limpieza dentro del mismo contenedor para reducir costo. Una instalación con carga real usaría servicios `api` y `worker` separados.

La instancia gratuita de Koyeb escala a cero después de una hora sin tráfico. La portada representa ese cold start como “Despertando el backend”. Referencias: [Koyeb instances](https://www.koyeb.com/docs/reference/instances) y [Scale-to-Zero](https://www.koyeb.com/docs/run-and-scale/scale-to-zero).

## 3. Cloudflare Pages

La vía recomendada es el workflow `.github/workflows/deploy-pages.yml`. Primero crear en Cloudflare Pages un proyecto **Direct Upload** llamado `forgequeue-web`; se puede hacer desde el panel o una sola vez con:

```bash
npx wrangler pages project create forgequeue-web --production-branch main
```

Luego crear en GitHub:

- Secrets `CLOUDFLARE_ACCOUNT_ID` y `CLOUDFLARE_API_TOKEN` con permiso Cloudflare Pages: Edit.
- Variable `VITE_API_URL` con la URL HTTPS final de Koyeb.
- Variable `CLOUDFLARE_PAGES_ENABLED=true`.

Ejecutar **Deploy web** manualmente una primera vez. Después se publica automáticamente cuando cambia `web/`. El workflow compila con Node 24, publica `dist` mediante Wrangler y registra la URL como un deployment de GitHub. `public/_redirects` conserva React Router al recargar una ruta de detalle.

Configuración equivalente mediante integración Git:

- Build command: `npm ci && npm run build`
- Output: `dist`
- Node: 24
- Variable: `VITE_API_URL=https://<backend>.koyeb.app`

También se puede publicar con Wrangler:

```bash
cd web
npm ci
VITE_API_URL=https://<backend>.koyeb.app npm run build
npx wrangler pages deploy dist --project-name forgequeue-web
```

Actualizar `CORS_ORIGIN` en Koyeb con el dominio final y reiniciar.

## Verificación posterior

```bash
curl -fsS https://<backend>.koyeb.app/health/ready
curl -fsS https://<backend>.koyeb.app/api/openapi.json >/dev/null
curl -fsS https://<frontend>.pages.dev/ >/dev/null
API_URL=https://<backend>.koyeb.app WORKER_METRICS_URLS= bash scripts/smoke.sh
```

Completar además un upload desde el dominio de Pages para comprobar CORS y descargar un artefacto desde una segunda petición autenticada.

## Operación

- `/health/live` confirma que el proceso vive.
- `/health/ready` comprueba PostgreSQL.
- `/metrics` es compatible con Prometheus.
- Ejecutar `forgequeue cleanup` sirve como tarea de mantenimiento manual.
- Supabase puede pausar un proyecto Free con baja actividad durante siete días; consultar [Project Pausing](https://supabase.com/docs/guides/platform/free-project-pausing) y el [pricing vigente](https://supabase.com/pricing).

## Secretos

Nunca versionar `.env`. Rotar credenciales S3 y `RATE_LIMIT_SALT` antes de una demo pública. El bucket debe permanecer privado; ForgeQueue transmite cada descarga sólo después de validar la sesión.
