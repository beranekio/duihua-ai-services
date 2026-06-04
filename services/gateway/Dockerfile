FROM rust:1.96 AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
COPY src ./src
RUN cargo build --release

FROM gcr.io/distroless/cc-debian13:nonroot
WORKDIR /app
COPY --from=builder /app/target/release/duihua-gateway /usr/local/bin/duihua-gateway
EXPOSE 8080
CMD ["duihua-gateway"]