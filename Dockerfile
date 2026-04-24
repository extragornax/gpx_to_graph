FROM rust:latest AS builder

WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
COPY src/ src/

RUN cargo build --release --features server --bin server

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/server /usr/local/bin/server

ENV PORT=3000
ENV GPX_SHARE_DIR=/data/shares
RUN mkdir -p /data/shares
VOLUME ["/data/shares"]
EXPOSE 3000

CMD ["server"]
