# 构建阶段
FROM rust:alpine AS builder

RUN apk add --no-cache musl-dev
WORKDIR /app
COPY . .
RUN cargo build --release --target x86_64-unknown-linux-musl

# 运行阶段
FROM gcr.io/distroless/static-debian12

COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/gcli-nexus /app/gcli-nexus
WORKDIR /app

ENTRYPOINT ["/app/gcli-nexus"]