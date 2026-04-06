# Stage 1: Builder
FROM rust:1.94.0-alpine3.22 AS builder

RUN apk add --no-cache musl-dev openssl-dev openssl-libs-static pkgconfig g++ make

WORKDIR /usr/src/app

COPY . .

RUN cargo build --release -p approval_workflow

# Stage 2: Runtime
FROM alpine:3.22

RUN apk add --no-cache ca-certificates libgcc

RUN adduser -D -u 1000 appuser
WORKDIR /app

COPY --from=builder --chown=appuser:appuser /usr/src/app/target/release/approval_workflow /app/approval_workflow

USER appuser

ENV RUST_LOG=info

ENTRYPOINT ["/app/approval_workflow"]
