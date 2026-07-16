# Contribuir

## Preparación

La vía recomendada es `docker compose up --build`. Para desarrollo directo se requieren Rust 1.97.1, Node 24, PostgreSQL y PDFium.

## Antes de abrir un PR

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cd web && npm ci && npm run typecheck && npm run lint && npm test && npm run build
```

Si cambia un DTO o endpoint:

```bash
cd web && npm run generate:api
```

Incluir una prueba para cada transición de estado o regla de validación nueva. Mantener interfaz y documentación en español, y código, tipos y rutas API en inglés.

## Commits

Preferir commits pequeños con verbo imperativo. Explicar en el PR el comportamiento observable, el riesgo y cómo se verificó.
