FROM lukemathwalker/cargo-chef:latest-rust-alpine AS chef
WORKDIR /build
RUN apk add --no-cache binutils

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /build/recipe.json recipe.json
# Build dependencies - cached layer
RUN cargo chef cook --release --recipe-path recipe.json

COPY . .

ARG GCLI_CLIENT_ID
ARG GCLI_CLIENT_SECRET
ARG SQLX_OFFLINE

RUN cargo build --release && \
    strip target/release/gcli-nexus && \
    mv target/release/gcli-nexus /tmp/gcli-nexus

FROM gcr.io/distroless/static-debian12
COPY --from=builder /tmp/gcli-nexus /app/gcli-nexus
WORKDIR /app
ENTRYPOINT ["/app/gcli-nexus"]