FROM rust:1.95-bookworm AS builder
WORKDIR /src
COPY . .
RUN cargo build --release --bin gateway

FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates libssl3 sqlite3 \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /src/target/release/gateway /usr/local/bin/gateway
COPY --from=builder /src/pricing-catalog.json /app/pricing-catalog.json
COPY --from=builder /src/config /app/config

VOLUME ["/app/data"]
EXPOSE 8080
ENV RUST_LOG=info
ENV GATEWAY_PRICING_CATALOG=/app/pricing-catalog.json
ENTRYPOINT ["/usr/local/bin/gateway"]

FROM runtime AS lite
CMD ["--config", "/app/config/example.lite.yaml"]

FROM runtime AS standard
CMD ["--config", "/app/config/example.standard.yaml"]
