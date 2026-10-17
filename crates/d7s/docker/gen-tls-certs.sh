#!/bin/sh
# Test-only CA + server cert (SAN DNS:localhost) for the *-tls compose services.
set -e
cd "$(dirname "$0")"
mkdir -p certs
cd certs
openssl req -x509 -newkey rsa:2048 -nodes -days 30 -subj "/CN=d7s-test-ca" \
  -keyout ca.key -out ca.crt
openssl req -newkey rsa:2048 -nodes -subj "/CN=localhost" \
  -keyout server.key -out server.csr
# DNS name only (no IP SAN) so verify-full via 127.0.0.1 must fail the host check.
printf 'subjectAltName=DNS:localhost\n' > san.ext
# Unrelated CA: verify-full with this as sslrootcert must fail.
openssl req -x509 -newkey rsa:2048 -nodes -days 30 -subj "/CN=d7s-other-ca" \
  -keyout other-ca.key -out other-ca.crt
openssl x509 -req -in server.csr -CA ca.crt -CAkey ca.key -CAcreateserial \
  -days 30 -extfile san.ext -out server.crt
# Containers run as other users; these are throwaway test keys.
chmod 644 ca.crt other-ca.crt server.crt server.key
echo "certs ready in $(pwd)"
