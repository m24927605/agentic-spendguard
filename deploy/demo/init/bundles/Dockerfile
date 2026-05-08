# Init container that bakes the demo contract + schema bundles + side-car
# runtime.env carrying the computed sha256s. Idempotent on re-run.
FROM alpine:3.20

RUN apk add --no-cache bash coreutils tar gzip openssl

COPY generate.sh /usr/local/bin/generate.sh
RUN chmod +x /usr/local/bin/generate.sh

ENTRYPOINT ["/usr/local/bin/generate.sh"]
