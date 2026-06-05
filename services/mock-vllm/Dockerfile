FROM gcr.io/distroless/python3-debian13:nonroot

WORKDIR /app
COPY server.py /app/server.py

ENV HOST=0.0.0.0
ENV PORT=8000
ENV DEFAULT_MODEL=HuggingFaceTB/SmolLM2-135M-Instruct

EXPOSE 8000
CMD ["/app/server.py"]