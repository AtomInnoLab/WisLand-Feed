FROM rust:1.88.0

WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends poppler-utils && rm -rf /var/lib/apt/lists/*

ENV PDF2IMAGE_POPPLER_PATH=/usr/bin