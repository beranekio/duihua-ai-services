FROM rust:1.96 AS builder
RUN apt-get update \
  && apt-get install -y --no-install-recommends protobuf-compiler \
  && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY gateway ./gateway
COPY background-worker ./background-worker
RUN cargo build --release -p duihua-gateway

FROM gcr.io/distroless/cc-debian13:nonroot
WORKDIR /app
COPY --from=builder /app/target/release/duihua-gateway /usr/local/bin/duihua-gateway
EXPOSE 8080
CMD ["duihua-gateway"]