FROM rust:1.96 AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY common ./common
COPY gateway ./gateway
COPY background-worker ./background-worker
RUN cargo build --release -p duihua-background-worker

FROM gcr.io/distroless/cc-debian13:nonroot
WORKDIR /app
COPY --from=builder /app/target/release/duihua-background-worker /usr/local/bin/duihua-background-worker
CMD ["duihua-background-worker"]