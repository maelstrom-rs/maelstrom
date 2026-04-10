#!/bin/sh
set -e

mkdir -p /etc/maelstrom /data/tls

SERVER="${SERVER_NAME:-localhost}"

TLS_CONF=""
if [ -f /complement/ca/ca.crt ] && [ -f /complement/ca/ca.key ]; then
    # Trust the complement CA so our outbound federation client verifies remote certs
    cp /complement/ca/ca.crt /usr/local/share/ca-certificates/complement.crt
    update-ca-certificates 2>/dev/null || true

    # Generate server certificate signed by complement CA
    # Include DNS SANs for the server name and common Docker networking patterns
    openssl req -new -newkey rsa:2048 -sha256 -days 1 -nodes \
        -subj "/CN=${SERVER}" \
        -addext "subjectAltName=DNS:${SERVER},DNS:localhost,IP:127.0.0.1" \
        -keyout /data/tls/server.key \
        -out /data/tls/server.csr 2>/dev/null

    openssl x509 -req -days 1 -sha256 \
        -in /data/tls/server.csr \
        -CA /complement/ca/ca.crt -CAkey /complement/ca/ca.key \
        -CAcreateserial \
        -copy_extensions copyall \
        -out /data/tls/server.crt 2>/dev/null

    TLS_CONF="federation_address = \"0.0.0.0:8448\"
tls_cert = \"/data/tls/server.crt\"
tls_key = \"/data/tls/server.key\"
complement_ca = \"/complement/ca/ca.crt\""
fi

cat > /etc/maelstrom/config.toml <<EOF
[server]
bind_address = "0.0.0.0:8008"
server_name = "${SERVER}"
public_base_url = "https://${SERVER}:8448"
${TLS_CONF}

[database]
endpoint = "mem://"
namespace = "maelstrom"
database = "maelstrom"
username = "root"
password = "root"
EOF

echo "Starting Maelstrom (server_name=${SERVER})"
exec maelstrom
