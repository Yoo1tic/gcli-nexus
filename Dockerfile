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

ARG CLAUDE_SYSTEM_PREAMBLE
ARG SQLX_OFFLINE

RUN cargo build --release && \
    strip target/release/pollux && \
    mv target/release/pollux /tmp/pollux

FROM gcr.io/distroless/static-debian12
COPY --from=builder /tmp/pollux /app/pollux
WORKDIR /app
ENTRYPOINT ["/app/pollux"]
