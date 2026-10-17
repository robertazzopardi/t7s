#!/bin/sh
# Test-only CA + server cert (SAN localhost/127.0.0.1) for the *-tls compose services.
set -e
cd "$(dirname "$0")"
mkdir -p certs
cd certs
openssl req -x509 -newkey rsa:2048 -nodes -days 30 -subj "/CN=d7s-test-ca" \
  -keyout ca.key -out ca.crt
openssl req -newkey rsa:2048 -nodes -subj "/CN=localhost" \
  -keyout server.key -out server.csr
printf 'subjectAltName=DNS:localhost,IP:127.0.0.1\n' > san.ext
openssl x509 -req -in server.csr -CA ca.crt -CAkey ca.key -CAcreateserial \
  -days 30 -extfile san.ext -out server.crt
# Containers run as other users; these are throwaway test keys.
chmod 644 ca.crt server.crt server.key
echo "certs ready in $(pwd)"
