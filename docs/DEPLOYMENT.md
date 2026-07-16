# Despliegue gratuito de referencia

La topología de portfolio usa frontend estático, un backend combinado y servicios administrados. Los planes gratuitos cambian; confirmar límites antes de publicar.

## 1. Supabase

Crear un proyecto y conservar:

- Connection string PostgreSQL con TLS.
- Endpoint S3, región, bucket privado, access key y secret key.

La aplicación crea el esquema al arrancar. Un advisory lock serializa las migraciones si dos instancias comienzan juntas.

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

Crear un servicio desde el `Dockerfile` raíz.

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

Koyeb puede dormir una instancia gratuita tras inactividad. La portada representa ese cold start como “Despertando el backend”. Referencia: [Koyeb instances](https://www.koyeb.com/docs/reference/instances).

## 3. Cloudflare Pages

Configurar el directorio `web`:

- Build command: `npm ci && npm run build`
- Output: `dist`
- Node: 24
- Variable: `VITE_API_URL=https://<backend>.koyeb.app`

También se puede publicar con Wrangler:

```bash
cd web
npm ci
VITE_API_URL=https://<backend>.koyeb.app npm run build
npx wrangler pages deploy dist --project-name forgequeue
```

Actualizar `CORS_ORIGIN` en Koyeb con el dominio final y reiniciar.

## Operación

- `/health/live` confirma que el proceso vive.
- `/health/ready` comprueba PostgreSQL.
- `/metrics` es compatible con Prometheus.
- Ejecutar `forgequeue cleanup` sirve como tarea de mantenimiento manual.
- Supabase puede pausar proyectos inactivos; consultar el [pricing vigente](https://supabase.com/pricing).

## Secretos

Nunca versionar `.env`. Rotar credenciales S3 y `RATE_LIMIT_SALT` antes de una demo pública. El bucket debe permanecer privado; ForgeQueue transmite cada descarga sólo después de validar la sesión.
