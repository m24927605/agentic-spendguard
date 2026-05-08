# Init container that generates the demo PKI on first compose-up.
# Re-running is idempotent (skips if /pki/ca.crt already present).
FROM alpine:3.20

RUN apk add --no-cache openssl xxd

COPY generate.sh /usr/local/bin/generate.sh
RUN chmod +x /usr/local/bin/generate.sh

ENTRYPOINT ["/usr/local/bin/generate.sh"]
