FROM rust:1.96 AS builder
RUN apt-get update \
  && apt-get install -y --no-install-recommends protobuf-compiler=3.21.12-11 \
  && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY background-worker ./background-worker
RUN cargo build --release -p duihua-background-worker

FROM gcr.io/distroless/cc-debian13:nonroot
WORKDIR /app
COPY --from=builder /app/target/release/duihua-background-worker /usr/local/bin/duihua-background-worker
CMD ["duihua-background-worker"]