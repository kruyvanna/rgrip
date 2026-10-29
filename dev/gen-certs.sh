#!/bin/sh
# Generate a throwaway CA, server cert (localhost / pg) and client cert for local testing.
set -e
cd "$(dirname "$0")"
mkdir -p certs && cd certs
[ -f ca.crt ] && exit 0
openssl req -new -x509 -days 3650 -nodes -subj "/CN=rgrip-dev-ca" -keyout ca.key -out ca.crt 2>/dev/null
openssl req -new -nodes -subj "/CN=localhost" -keyout server.key -out server.csr 2>/dev/null
printf "subjectAltName=DNS:localhost,DNS:pg,IP:127.0.0.1\n" > san.ext
openssl x509 -req -days 3650 -in server.csr -CA ca.crt -CAkey ca.key -CAcreateserial -extfile san.ext -out server.crt 2>/dev/null
openssl req -new -nodes -subj "/CN=rgrip" -keyout client.key -out client.csr 2>/dev/null
# Extensions make it an X.509 v3 certificate; without them some OpenSSL
# versions (e.g. 3.0 on Ubuntu) issue v1, which rustls rejects.
printf "basicConstraints=CA:FALSE\nextendedKeyUsage=clientAuth\n" > client.ext
openssl x509 -req -days 3650 -in client.csr -CA ca.crt -CAkey ca.key -CAcreateserial -extfile client.ext -out client.crt 2>/dev/null
rm -f *.csr san.ext client.ext ca.srl
# SSH key for tunnel tests
ssh-keygen -q -t ed25519 -N "" -C rgrip-dev -f ../ssh/id_ed25519
echo "certs ready"
