# syntax=docker/dockerfile:1

## Build stage
FROM rust:1.88 AS builder
WORKDIR /usr/src/findx
COPY . .
RUN cargo build --release --locked

## Runtime stage
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Directory where data will be mounted
WORKDIR /data

# Copy compiled binary
COPY --from=builder /usr/src/findx/target/release/findx /usr/local/bin/findx

ENTRYPOINT ["findx"]
CMD ["--help"]
