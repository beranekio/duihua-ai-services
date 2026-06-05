FROM python:3.12-alpine

WORKDIR /app
COPY server.py /app/server.py

ENV HOST=0.0.0.0
ENV PORT=8000
ENV DEFAULT_MODEL=HuggingFaceTB/SmolLM2-135M-Instruct

EXPOSE 8000
CMD ["python3", "/app/server.py"]