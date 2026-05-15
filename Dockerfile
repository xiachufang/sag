FROM rust:1.95-bookworm AS builder
WORKDIR /src
COPY . .
RUN cargo build --release --bin gateway

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates libssl3 sqlite3 \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /src/target/release/gateway /usr/local/bin/gateway

VOLUME ["/app/data"]
EXPOSE 8080
ENV RUST_LOG=info
ENTRYPOINT ["/usr/local/bin/gateway"]
CMD ["--config", "/app/config/example.standard.yaml"]
