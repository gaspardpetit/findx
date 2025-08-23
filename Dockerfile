# syntax=docker/dockerfile:1

## Build stage
FROM rust:1.82 AS builder
WORKDIR /usr/src/localindex
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
COPY --from=builder /usr/src/localindex/target/release/localindex /usr/local/bin/localindex

ENTRYPOINT ["localindex"]
CMD ["--help"]
