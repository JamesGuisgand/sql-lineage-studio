# Dependencies are compiled in the builder (first deployment downloads crates.io).
FROM rust:1.88-slim-bookworm AS rust-builder
WORKDIR /build
COPY engine/Cargo.toml ./Cargo.toml
COPY engine/src/ ./src/
RUN cargo test --release && cargo build --release

# Runtime remains lean and rootless. Rust does the actual SQL parsing.
FROM python:3.13-slim-bookworm
ENV PYTHONDONTWRITEBYTECODE=1 PYTHONUNBUFFERED=1 PORT=8080 \
    MAX_SQL_BYTES=131072 RUST_TIMEOUT=8 \
    SQL_LINEAGE_BIN=/usr/local/bin/sql-lineage-rs
WORKDIR /app
COPY --from=rust-builder /build/target/release/sql-lineage-rs /usr/local/bin/sql-lineage-rs
COPY server.py /app/server.py
COPY web /app/web
COPY engine/examples/demo.sql /app/engine/examples/demo.sql
RUN useradd --system --uid 10001 --home-dir /nonexistent --shell /usr/sbin/nologin lineage
USER 10001:10001
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=4s --start-period=12s --retries=3 \
    CMD python -c "import urllib.request; urllib.request.urlopen('http://127.0.0.1:8080/healthz',timeout=2)" || exit 1
CMD ["python", "-u", "/app/server.py"]
