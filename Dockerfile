# syntax=docker/dockerfile:1.7
FROM rust:1.97-bookworm AS builder
WORKDIR /app

COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
COPY migrations ./migrations
RUN cargo build --locked --release -p forgequeue-server

FROM debian:bookworm-slim AS runtime
ARG PDFIUM_VERSION=7947
ARG PDFIUM_SHA256=f73d69d309fe1f33cc7269dcc99be31ec44e1cf608e31d7e2fcc6545fc2f9323
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && curl -fsSL "https://github.com/bblanchon/pdfium-binaries/releases/download/chromium/${PDFIUM_VERSION}/pdfium-linux-x64.tgz" -o /tmp/pdfium.tgz \
    && echo "${PDFIUM_SHA256}  /tmp/pdfium.tgz" | sha256sum --check --strict \
    && tar -xzf /tmp/pdfium.tgz -C /tmp \
    && install -m 0755 /tmp/lib/libpdfium.so /usr/local/lib/libpdfium.so \
    && rm -rf /tmp/* /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home forgequeue \
    && ldconfig

COPY --from=builder /app/target/release/forgequeue-server /usr/local/bin/forgequeue

ENV BIND_ADDRESS=0.0.0.0:8080 \
    LOG_FORMAT=json \
    PDFIUM_LIBRARY_PATH=/usr/local/lib
EXPOSE 8080
USER forgequeue
ENTRYPOINT ["/usr/local/bin/forgequeue"]
CMD ["all"]
