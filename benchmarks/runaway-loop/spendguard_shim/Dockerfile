FROM python:3.12-slim

WORKDIR /app
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

COPY app.py .

EXPOSE 8090
CMD ["sh", "-c", "uvicorn app:app --host 0.0.0.0 --port 8090 --no-access-log --workers ${SHIM_UVICORN_WORKERS:-1}"]
