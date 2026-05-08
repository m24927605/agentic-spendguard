# Init container that mints the demo manifest signing keypair, writes
# the catalog body JSON, and emits a signed manifest pointer matching
# the canonical bytes the sidecar verifies (services/sidecar/src/
# bootstrap/catalog.rs::verify_manifest_signature).
FROM python:3.12-alpine

RUN pip install --no-cache-dir 'cryptography>=42.0,<46'

COPY sign.py /usr/local/bin/sign.py
RUN chmod +x /usr/local/bin/sign.py

ENTRYPOINT ["python", "/usr/local/bin/sign.py"]
