# Política de seguridad

ForgeQueue es una demostración de portfolio y no debe recibir información sensible.

## Reportar una vulnerabilidad

No abrir un issue público con detalles explotables. Usar el canal privado de GitHub Security Advisories del repositorio e incluir versión, impacto, reproducción mínima y mitigación sugerida.

## Modelo de amenaza

La aplicación asume entradas hostiles y aplica tamaño máximo, detección por contenido, límites de píxeles/páginas, timeout, cuotas, almacenamiento privado y expiración. PDFium corre como usuario sin privilegios dentro de un contenedor con filesystem de sólo lectura y límites de recursos.

No se afirma aislamiento apto para archivos altamente sensibles ni resistencia frente a todos los ataques de parser. Una operación real debería sumar escaneo antivirus, WAF, secretos administrados, TLS extremo a extremo, auditoría y aislamiento más fuerte de workers.

## Versiones soportadas

Sólo la rama `main` recibe correcciones.

## Dependencias

Dependabot revisa Cargo, npm y GitHub Actions semanalmente. RustSec se ejecuta en cada cambio junto con las pruebas y la construcción completa del contenedor.

`RUSTSEC-2023-0071` se ignora de forma explícita porque pertenece a `rsa`, una dependencia del driver MySQL opcional que SQLx conserva en `Cargo.lock`; ForgeQueue desactiva los features por defecto y habilita exclusivamente PostgreSQL. `cargo tree -i rsa` no encuentra una ruta alcanzable en este build. El ignore debe eliminarse si SQLx deja de registrar esa dependencia o si el proyecto habilita MySQL.
